use std::sync::Arc;

use mongodb::{bson::Document, ClientSession};
use skiff_runtime_capability_context::DbRuntimeFinalizer;
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::capability_error;
#[cfg(test)]
use crate::prepared_runtime::PreparedRuntimeTestOutcome;
use crate::{
    recoverable_read_context, recoverable_write_context, CollectedRecoverableRootStore,
    CurrentRequestRecoverableArtifactStore, DbRecoverableRuntimeContext, Result, ServiceDbRuntime,
};
use skiff_runtime_model::recoverable::RecoverableArtifactRetentionRoot;

pub(crate) struct PreparedCreate {
    type_name: String,
    collection_name: String,
    document: Document,
    roots: Vec<RecoverableArtifactRetentionRoot>,
    context: DbRecoverableRuntimeContext,
}

pub(crate) struct CompletedCreate {
    type_name: String,
    document: Document,
    context: DbRecoverableRuntimeContext,
}

impl ServiceDbRuntime {
    pub(crate) fn prepare_create_runtime_command(
        &self,
        type_name: &str,
        value: &RuntimeValue,
        heap: &RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<PreparedCreate> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let artifact_store = CurrentRequestRecoverableArtifactStore::new(&context);
        let mut root_store = CollectedRecoverableRootStore::default();
        let document = {
            let mut write_context =
                recoverable_write_context(&context, &artifact_store, &mut root_store);
            binding.document_from_runtime_business_value(value, heap, Some(&mut write_context))?
        };
        Ok(PreparedCreate {
            type_name: type_name.to_string(),
            collection_name: binding.collection_name.clone(),
            document,
            roots: root_store.roots,
            context,
        })
    }
}

impl PreparedCreate {
    pub(crate) async fn execute(
        self,
        runtime: &ServiceDbRuntime,
        session: Option<&mut ClientSession>,
    ) -> Result<CompletedCreate> {
        match session {
            Some(session) => self.execute_inner(runtime, Some(session)).await,
            None => {
                let mut transaction = runtime.start_transaction().await?;
                let result = self.execute_inner(runtime, Some(&mut transaction)).await;
                runtime.finish_transaction(transaction, result, &[]).await
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn complete_for_test(
        self,
        outcome: PreparedRuntimeTestOutcome,
    ) -> Result<CompletedCreate> {
        if !matches!(outcome, PreparedRuntimeTestOutcome::Create) {
            return Err(crate::ServiceDbError::Decode(
                "prepared runtime test driver returned the wrong create result".to_string(),
            ));
        }
        Ok(CompletedCreate {
            type_name: self.type_name,
            document: self.document,
            context: self.context,
        })
    }

    async fn execute_inner(
        self,
        runtime: &ServiceDbRuntime,
        mut session: Option<&mut ClientSession>,
    ) -> Result<CompletedCreate> {
        runtime
            .persist_recoverable_artifact_retention_roots(&self.roots, session.as_deref_mut())
            .await?;
        let result_document = self.document.clone();
        runtime
            .mongo_executor(&self.collection_name, session)
            .await?
            .insert_one(self.document)
            .await?;
        Ok(CompletedCreate {
            type_name: self.type_name,
            document: result_document,
            context: self.context,
        })
    }
}

impl CompletedCreate {
    pub(crate) fn finalize(
        self,
        runtime: &ServiceDbRuntime,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let binding = runtime.metadata.collection_for_type(&self.type_name)?;
        let read_context = recoverable_read_context(&self.context);
        binding.runtime_business_value_from_document(self.document, heap, Some(&read_context))
    }

    pub(crate) fn into_finalizer(
        self,
        runtime: Arc<ServiceDbRuntime>,
    ) -> DbRuntimeFinalizer<RuntimeValue> {
        DbRuntimeFinalizer::new(move |heap| self.finalize(&runtime, heap).map_err(capability_error))
    }
}
