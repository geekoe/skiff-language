use std::sync::Arc;

use mongodb::{bson::Document, ClientSession};
use skiff_runtime_capability_context::{
    DbKey, DbOrderEntry, DbQuery, DbRuntimeFinalizer, FieldPath, ServiceDbFindOptions,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::capability_error;
#[cfg(test)]
use crate::prepared_runtime::{PreparedRuntimeTestKind, PreparedRuntimeTestOutcome};
use crate::{
    mongo::MongoFindManyPlan, mongo::MongoFindOnePlan, recoverable_read_context,
    DbRecoverableRuntimeContext, Result, ServiceDbRuntime,
};

pub(crate) struct PreparedFindOne {
    type_name: String,
    collection_name: String,
    plan: MongoFindOnePlan,
    context: DbRecoverableRuntimeContext,
    #[cfg(test)]
    test_kind: PreparedRuntimeTestKind,
}

pub(crate) struct CompletedFindOne {
    type_name: String,
    document: Option<Document>,
    context: DbRecoverableRuntimeContext,
}

pub(crate) struct PreparedFindMany {
    type_name: String,
    collection_name: String,
    plan: Option<MongoFindManyPlan>,
    context: DbRecoverableRuntimeContext,
}

pub(crate) struct CompletedFindMany {
    type_name: String,
    documents: Vec<Document>,
    context: DbRecoverableRuntimeContext,
}

impl ServiceDbRuntime {
    pub(crate) fn prepare_find_one_by_key_runtime_command(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        context: DbRecoverableRuntimeContext,
    ) -> Result<PreparedFindOne> {
        let binding = self.metadata.collection_for_type(type_name)?;
        Ok(PreparedFindOne {
            type_name: type_name.to_string(),
            collection_name: binding.collection_name.clone(),
            plan: MongoFindOnePlan {
                filter: binding.key_filter(&key)?,
                sort: None,
                projection: binding.projection_document(projection.as_deref())?,
            },
            context,
            #[cfg(test)]
            test_kind: PreparedRuntimeTestKind::FindOneByKey,
        })
    }

    pub(crate) fn prepare_find_one_by_query_runtime_command(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        context: DbRecoverableRuntimeContext,
    ) -> Result<PreparedFindOne> {
        let binding = self.metadata.collection_for_type(type_name)?;
        Ok(PreparedFindOne {
            type_name: type_name.to_string(),
            collection_name: binding.collection_name.clone(),
            plan: MongoFindOnePlan {
                filter: binding.query_filter(query)?,
                sort: binding.order_document(&order)?,
                projection: binding.projection_document(projection.as_deref())?,
            },
            context,
            #[cfg(test)]
            test_kind: PreparedRuntimeTestKind::FindOneByQuery,
        })
    }

    pub(crate) fn prepare_find_many_page_runtime_command(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        context: DbRecoverableRuntimeContext,
    ) -> Result<PreparedFindMany> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.query_filter(query)?;
        let sort = binding.page_sort_document(&options)?;
        let plan = if options.limit == Some(0) {
            None
        } else {
            Some(MongoFindManyPlan {
                filter,
                sort,
                projection: binding.projection_document(projection.as_deref())?,
                limit: options.limit,
                offset: options.offset,
            })
        };
        Ok(PreparedFindMany {
            type_name: type_name.to_string(),
            collection_name: binding.collection_name.clone(),
            plan,
            context,
        })
    }
}

impl PreparedFindOne {
    pub(crate) async fn execute(
        self,
        runtime: &ServiceDbRuntime,
        session: Option<&mut ClientSession>,
    ) -> Result<CompletedFindOne> {
        let document = runtime
            .mongo_executor(&self.collection_name, session)
            .await?
            .find_one(self.plan)
            .await?;
        Ok(CompletedFindOne {
            type_name: self.type_name,
            document,
            context: self.context,
        })
    }

    #[cfg(test)]
    pub(crate) fn complete_for_test(
        self,
        outcome: PreparedRuntimeTestOutcome,
    ) -> Result<CompletedFindOne> {
        let PreparedRuntimeTestOutcome::Optional(document) = outcome else {
            return Err(crate::ServiceDbError::Decode(
                "prepared runtime test driver returned the wrong find-one result".to_string(),
            ));
        };
        Ok(CompletedFindOne {
            type_name: self.type_name,
            document,
            context: self.context,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_kind(&self) -> PreparedRuntimeTestKind {
        self.test_kind
    }
}

impl CompletedFindOne {
    pub(crate) fn finalize(
        self,
        runtime: &ServiceDbRuntime,
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        let binding = runtime.metadata.collection_for_type(&self.type_name)?;
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

impl PreparedFindMany {
    pub(crate) fn requires_provider(&self) -> bool {
        self.plan.is_some()
    }

    pub(crate) async fn execute(
        self,
        runtime: &ServiceDbRuntime,
        session: Option<&mut ClientSession>,
    ) -> Result<CompletedFindMany> {
        let documents = match self.plan {
            Some(plan) => {
                runtime
                    .mongo_executor(&self.collection_name, session)
                    .await?
                    .find_many(plan)
                    .await?
            }
            None => Vec::new(),
        };
        Ok(CompletedFindMany {
            type_name: self.type_name,
            documents,
            context: self.context,
        })
    }

    #[cfg(test)]
    pub(crate) fn complete_for_test(
        self,
        outcome: PreparedRuntimeTestOutcome,
    ) -> Result<CompletedFindMany> {
        let PreparedRuntimeTestOutcome::Many(documents) = outcome else {
            return Err(crate::ServiceDbError::Decode(
                "prepared runtime test driver returned the wrong find-many result".to_string(),
            ));
        };
        Ok(CompletedFindMany {
            type_name: self.type_name,
            documents,
            context: self.context,
        })
    }
}

impl CompletedFindMany {
    pub(crate) fn finalize(
        self,
        runtime: &ServiceDbRuntime,
        heap: &mut RequestHeap,
    ) -> Result<Vec<RuntimeValue>> {
        let binding = runtime.metadata.collection_for_type(&self.type_name)?;
        let read_context = recoverable_read_context(&self.context);
        self.documents
            .into_iter()
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .collect()
    }

    pub(crate) fn into_finalizer(
        self,
        runtime: Arc<ServiceDbRuntime>,
    ) -> DbRuntimeFinalizer<Vec<RuntimeValue>> {
        DbRuntimeFinalizer::new(move |heap| self.finalize(&runtime, heap).map_err(capability_error))
    }
}
