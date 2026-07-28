use std::sync::Arc;

use mongodb::{
    bson::{Bson, Document},
    ClientSession,
};
use skiff_runtime_capability_context::{DbOneSelector, DbRuntimeFinalizer};
use skiff_runtime_model::{
    recoverable::RecoverableArtifactRetentionRoot, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};

use super::capability_error;
#[cfg(test)]
use crate::prepared_runtime::PreparedRuntimeTestOutcome;
use crate::{
    cascade::cascade_plan_for_replacement, guarded_filter, has_matching_lease_guards,
    mongo::MongoFindOnePlan, mongo::MongoOneWritePlan, recoverable_read_context,
    recoverable_write_context, service_db_now_ms, CollectedRecoverableRootStore,
    CurrentRequestRecoverableArtifactStore, DbLeaseHold, DbRecoverableRuntimeContext, Result,
    ServiceDbError, ServiceDbRuntime, SKIFF_LEASES_FIELD,
};

pub(crate) struct PreparedReplace {
    type_name: String,
    collection_name: String,
    filter: Document,
    sort: Option<Document>,
    replacement: Document,
    roots: Vec<RecoverableArtifactRetentionRoot>,
    context: DbRecoverableRuntimeContext,
}

pub(crate) struct CompletedReplace {
    type_name: String,
    document: Option<Document>,
    context: DbRecoverableRuntimeContext,
}

impl ServiceDbRuntime {
    pub(crate) fn prepare_replace_one_runtime_command(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: &RuntimeValue,
        heap: &RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<PreparedReplace> {
        let binding = self.metadata.collection_for_target_key(type_name)?;
        let normalized = binding.normalize_one_selector(selector)?;
        let artifact_store = CurrentRequestRecoverableArtifactStore::new(&context);
        let mut root_store = CollectedRecoverableRootStore::default();
        let replacement = {
            let mut write_context =
                recoverable_write_context(&context, &artifact_store, &mut root_store);
            binding.replacement_document_from_runtime_business_value(
                value,
                heap,
                Some(&mut write_context),
                normalized.normalized_key(),
                normalized.encrypted_context(),
            )?
        };
        Ok(PreparedReplace {
            type_name: type_name.to_string(),
            collection_name: binding.collection_name.clone(),
            filter: normalized.filter,
            sort: normalized.sort,
            replacement,
            roots: root_store.roots,
            context,
        })
    }
}

impl PreparedReplace {
    pub(crate) async fn execute(
        self,
        runtime: &ServiceDbRuntime,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<CompletedReplace> {
        match session {
            Some(session) => {
                self.execute_inner(runtime, lease_guards, Some(session))
                    .await
            }
            None => {
                let mut transaction = runtime.start_transaction().await?;
                let result = self
                    .execute_inner(runtime, lease_guards, Some(&mut transaction))
                    .await;
                runtime
                    .finish_transaction(transaction, result, lease_guards)
                    .await
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn complete_for_test(
        self,
        outcome: PreparedRuntimeTestOutcome,
    ) -> Result<CompletedReplace> {
        let PreparedRuntimeTestOutcome::Optional(document) = outcome else {
            return Err(ServiceDbError::Decode(
                "prepared runtime test driver returned the wrong replace result".to_string(),
            ));
        };
        Ok(CompletedReplace {
            type_name: self.type_name,
            document,
            context: self.context,
        })
    }

    async fn execute_inner(
        self,
        runtime: &ServiceDbRuntime,
        lease_guards: &[DbLeaseHold],
        mut session: Option<&mut ClientSession>,
    ) -> Result<CompletedReplace> {
        let binding = runtime
            .metadata
            .collection_for_target_key(&self.type_name)?;
        runtime
            .persist_recoverable_artifact_retention_roots(&self.roots, session.as_deref_mut())
            .await?;
        let guarded_filter = guarded_filter(
            binding,
            self.filter.clone(),
            lease_guards,
            service_db_now_ms(),
        )?;
        let mut executor = runtime
            .mongo_executor(&self.collection_name, session)
            .await?;
        runtime
            .assert_lease_guards_live(binding, &self.filter, lease_guards, &mut executor)
            .await?;
        let old_document = if binding.has_immutable_file_cascade()
            || has_matching_lease_guards(binding, lease_guards)
        {
            executor
                .find_one(MongoFindOnePlan {
                    filter: self.filter.clone(),
                    sort: self.sort.clone(),
                    ..Default::default()
                })
                .await?
        } else {
            None
        };
        let mut replacement = self.replacement;
        if let Some(Bson::Document(leases)) = old_document
            .as_ref()
            .and_then(|document| document.get(SKIFF_LEASES_FIELD))
        {
            replacement.insert(SKIFF_LEASES_FIELD, Bson::Document(leases.clone()));
        }
        let document = executor
            .find_one_and_replace(
                MongoOneWritePlan {
                    filter: guarded_filter,
                    sort: self.sort,
                },
                replacement.clone(),
            )
            .await?;
        if document.is_none() {
            runtime
                .assert_lease_guards_live(binding, &self.filter, lease_guards, &mut executor)
                .await?;
        }
        if let Some(old_document) = &old_document {
            runtime
                .delete_skiff_files_by_plan(
                    cascade_plan_for_replacement(
                        old_document,
                        &replacement,
                        &binding.immutable_file_paths,
                    ),
                    executor.session_mut(),
                )
                .await?;
        }
        Ok(CompletedReplace {
            type_name: self.type_name,
            document,
            context: self.context,
        })
    }
}

impl CompletedReplace {
    pub(crate) fn finalize(
        self,
        runtime: &ServiceDbRuntime,
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        let binding = runtime
            .metadata
            .collection_for_target_key(&self.type_name)?;
        let read_context = recoverable_read_context(&self.context);
        self.document
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .transpose()
    }

    pub(crate) fn into_finalizer(
        self,
        runtime: Arc<ServiceDbRuntime>,
    ) -> DbRuntimeFinalizer<Option<RuntimeValue>> {
        DbRuntimeFinalizer::new(move |heap| self.finalize(&runtime, heap).map_err(capability_error))
    }
}

pub(crate) fn is_lease_lost(result: &Result<CompletedReplace>) -> bool {
    matches!(result, Err(ServiceDbError::LeaseLost(_)))
}
