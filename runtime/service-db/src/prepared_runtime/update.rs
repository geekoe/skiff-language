use std::sync::Arc;

use mongodb::{bson::Document, ClientSession};
use skiff_runtime_capability_context::{DbOneSelector, DbRuntimeFinalizer, ServiceDbChange};
use skiff_runtime_model::{
    recoverable::RecoverableArtifactRetentionRoot, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};

use super::capability_error;
#[cfg(test)]
use crate::prepared_runtime::PreparedRuntimeTestOutcome;
use crate::{
    cascade::cascade_plan_for_change, guarded_filter, mongo::MongoFindOnePlan,
    mongo::MongoOneWritePlan, recoverable_read_context, recoverable_write_context,
    service_db_now_ms, CollectedRecoverableRootStore, CurrentRequestRecoverableArtifactStore,
    DbLeaseHold, DbRecoverableRuntimeContext, DbRuntimeChange, Result, ServiceDbError,
    ServiceDbRuntime,
};

enum PreparedUpdateKind {
    Read {
        plan: MongoFindOnePlan,
    },
    Write {
        filter: Document,
        sort: Option<Document>,
        update: Document,
        wire_change: ServiceDbChange,
        cascade_paths: Vec<Vec<String>>,
        roots: Vec<RecoverableArtifactRetentionRoot>,
    },
}

pub(crate) struct PreparedUpdate {
    type_name: String,
    collection_name: String,
    context: DbRecoverableRuntimeContext,
    kind: PreparedUpdateKind,
}

pub(crate) struct CompletedUpdate {
    type_name: String,
    document: Option<Document>,
    context: DbRecoverableRuntimeContext,
}

impl ServiceDbRuntime {
    pub(crate) fn prepare_update_one_runtime_command(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<PreparedUpdate> {
        let binding = self.metadata.collection_for_target_key(type_name)?;
        let cascade_paths = binding.immutable_file_paths_for_change(&change.wire_change);
        let wire_change = change.wire_change.clone();
        let normalized = binding.normalize_one_selector(selector)?;
        let artifact_store = CurrentRequestRecoverableArtifactStore::new(&context);
        let mut root_store = CollectedRecoverableRootStore::default();
        let update = {
            let mut write_context =
                recoverable_write_context(&context, &artifact_store, &mut root_store);
            binding.runtime_change_update_document(
                type_name,
                change,
                heap,
                Some(&mut write_context),
                normalized.encrypted_context(),
            )?
        };
        let kind = if update.is_empty() {
            PreparedUpdateKind::Read {
                plan: MongoFindOnePlan {
                    filter: normalized.filter,
                    sort: normalized.sort,
                    projection: None,
                },
            }
        } else {
            PreparedUpdateKind::Write {
                filter: normalized.filter,
                sort: normalized.sort,
                update,
                wire_change,
                cascade_paths,
                roots: root_store.roots,
            }
        };
        Ok(PreparedUpdate {
            type_name: type_name.to_string(),
            collection_name: binding.collection_name.clone(),
            context,
            kind,
        })
    }
}

impl PreparedUpdate {
    pub(crate) async fn execute(
        self,
        runtime: &ServiceDbRuntime,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<CompletedUpdate> {
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
    ) -> Result<CompletedUpdate> {
        let PreparedRuntimeTestOutcome::Optional(document) = outcome else {
            return Err(ServiceDbError::Decode(
                "prepared runtime test driver returned the wrong update result".to_string(),
            ));
        };
        Ok(CompletedUpdate {
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
    ) -> Result<CompletedUpdate> {
        let binding = runtime
            .metadata
            .collection_for_target_key(&self.type_name)?;
        let document = match self.kind {
            PreparedUpdateKind::Read { plan } => {
                runtime
                    .mongo_executor(&self.collection_name, session)
                    .await?
                    .find_one(plan)
                    .await?
            }
            PreparedUpdateKind::Write {
                filter,
                sort,
                update,
                wire_change,
                cascade_paths,
                roots,
            } => {
                runtime
                    .persist_recoverable_artifact_retention_roots(&roots, session.as_deref_mut())
                    .await?;
                let guarded_filter =
                    guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
                let mut executor = runtime
                    .mongo_executor(&self.collection_name, session)
                    .await?;
                runtime
                    .assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                    .await?;
                let old_document = if cascade_paths.is_empty() {
                    None
                } else {
                    executor
                        .find_one(MongoFindOnePlan {
                            filter: filter.clone(),
                            sort: sort.clone(),
                            ..Default::default()
                        })
                        .await?
                };
                let document = executor
                    .find_one_and_update(
                        MongoOneWritePlan {
                            filter: guarded_filter,
                            sort,
                        },
                        update,
                    )
                    .await?;
                if document.is_none() {
                    runtime
                        .assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                        .await?;
                }
                if let Some(old_document) = &old_document {
                    runtime
                        .delete_skiff_files_by_plan(
                            cascade_plan_for_change(old_document, &wire_change, &cascade_paths),
                            executor.session_mut(),
                        )
                        .await?;
                }
                document
            }
        };
        Ok(CompletedUpdate {
            type_name: self.type_name,
            document,
            context: self.context,
        })
    }
}

impl CompletedUpdate {
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

pub(crate) fn is_lease_lost(result: &Result<CompletedUpdate>) -> bool {
    matches!(result, Err(ServiceDbError::LeaseLost(_)))
}
