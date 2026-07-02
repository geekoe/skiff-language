use std::{pin::Pin, sync::Arc};

use serde_json::Value;
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityContextApi, DbCapabilityError, DbCapabilityFactory,
    DbCapabilityFuture, DbCapabilityLeaseHandle, DbCapabilityLeaseHold,
    DbCapabilityLeaseHoldHandle, DbCapabilityResult, DbCapabilityStore, DbCapabilityStoreApi,
    DbDocument, DbKey, DbOneSelector, DbOrderEntry, DbPageResult, DbQuery,
    DbRecoverableRuntimeContext, DbRuntimeChange, DbWriteResult, FieldPath, FileCapabilityRecord,
    ServiceDbChange, ServiceDbFindOptions,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};
use tokio::sync::Mutex;

use crate::{DbLeaseHold, DbRequestState, ServiceDbError, ServiceDbRuntime, ServiceDbStore};

#[derive(Clone)]
pub struct ServiceDbCapabilityFactory {
    runtime: Arc<ServiceDbRuntime>,
}

impl ServiceDbCapabilityFactory {
    pub fn new(runtime: Arc<ServiceDbRuntime>) -> Self {
        Self { runtime }
    }

    pub fn context_for_request(
        &self,
        owner: impl Into<String>,
        request_id: impl Into<String>,
    ) -> DbCapabilityContext {
        DbCapabilityContext::new(ServiceDbCapabilityHandle::new(
            Some(self.clone()),
            owner,
            request_id,
        ))
    }

    fn runtime(&self) -> Arc<ServiceDbRuntime> {
        self.runtime.clone()
    }
}

impl DbCapabilityFactory for ServiceDbCapabilityFactory {
    fn context_for_request(&self, owner: String, request_id: String) -> DbCapabilityContext {
        ServiceDbCapabilityFactory::context_for_request(self, owner, request_id)
    }
}

impl ServiceDbRuntime {
    pub fn capability_factory(self: Arc<Self>) -> ServiceDbCapabilityFactory {
        ServiceDbCapabilityFactory::new(self)
    }
}

#[derive(Clone)]
pub struct ServiceDbCapabilityHandle {
    runtime: Option<ServiceDbCapabilityFactory>,
    request_state: Arc<Mutex<DbRequestState>>,
}

impl ServiceDbCapabilityHandle {
    fn new(
        runtime: Option<ServiceDbCapabilityFactory>,
        owner: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self::with_state(
            runtime,
            Arc::new(Mutex::new(DbRequestState::new(owner, request_id))),
        )
    }

    pub fn with_state(
        runtime: Option<ServiceDbCapabilityFactory>,
        request_state: Arc<Mutex<DbRequestState>>,
    ) -> Self {
        Self {
            runtime,
            request_state,
        }
    }

    fn require_service_db_store(
        &self,
        target: &str,
        unavailable_reason: &str,
    ) -> DbCapabilityResult<ServiceDbCapabilityStore> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| DbCapabilityError::provider_unavailable(target, unavailable_reason))?;
        Ok(ServiceDbCapabilityStore::new(ServiceDbStore::new(
            runtime.runtime(),
            self.request_state.clone(),
        )))
    }
}

impl DbCapabilityContextApi for ServiceDbCapabilityHandle {
    fn require_store(
        &self,
        target: &str,
        unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore> {
        self.require_service_db_store(target, unavailable_reason)
            .map(DbCapabilityStore::new)
    }
}

#[derive(Clone)]
pub struct ServiceDbCapabilityStore {
    store: ServiceDbStore,
}

impl ServiceDbCapabilityStore {
    pub fn new(store: ServiceDbStore) -> Self {
        Self { store }
    }
}

#[derive(Debug)]
struct ServiceDbLeaseHold(DbLeaseHold);

impl DbCapabilityLeaseHoldHandle for ServiceDbLeaseHold {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn eq_handle(&self, other: &dyn DbCapabilityLeaseHoldHandle) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| other.0 == self.0)
    }
}

impl DbCapabilityStoreApi for ServiceDbCapabilityStore {
    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(async move {
            self.store
                .begin_transaction()
                .await
                .map_err(db_capability_error)
        })
    }

    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(async move {
            self.store
                .commit_transaction()
                .await
                .map_err(db_capability_error)
        })
    }

    fn abort_transaction(&self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(self.store.abort_transaction())
    }

    fn find_one_by_key<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        Box::pin(async move {
            self.store
                .find_one_by_key(type_name, key, projection)
                .await
                .map_err(db_capability_error)
        })
    }

    fn find_one_by_key_runtime<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        Box::pin(async move {
            self.store
                .find_one_by_key_runtime(type_name, key, projection, heap, context)
                .await
                .map_err(db_capability_error)
        })
    }

    fn find_one_by_query<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        Box::pin(async move {
            self.store
                .find_one_by_query(type_name, query, order, projection)
                .await
                .map_err(db_capability_error)
        })
    }

    fn find_one_by_query_runtime<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        Box::pin(async move {
            self.store
                .find_one_by_query_runtime(type_name, query, order, projection, heap, context)
                .await
                .map_err(db_capability_error)
        })
    }

    fn find_many_page<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, DbPageResult> {
        Box::pin(async move {
            self.store
                .find_many_page(type_name, query, options, projection)
                .await
                .map_err(db_capability_error)
        })
    }

    fn find_many_page_runtime<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Vec<RuntimeValue>> {
        Box::pin(async move {
            self.store
                .find_many_page_runtime(type_name, query, options, projection, heap, context)
                .await
                .map_err(db_capability_error)
        })
    }

    fn create<'a>(
        &'a self,
        type_name: &'a str,
        value: DbDocument,
    ) -> DbCapabilityFuture<'a, DbDocument> {
        Box::pin(async move {
            self.store
                .create(type_name, value)
                .await
                .map_err(db_capability_error)
        })
    }

    fn create_runtime<'a>(
        &'a self,
        type_name: &'a str,
        value: &'a RuntimeValue,
        heap: &'a RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, RuntimeValue> {
        Box::pin(async move {
            self.store
                .create_runtime(type_name, value, heap, context)
                .await
                .map_err(db_capability_error)
        })
    }

    fn insert_many_result<'a>(
        &'a self,
        type_name: &'a str,
        values: Vec<DbDocument>,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async move {
            self.store
                .insert_many_result(type_name, values)
                .await
                .map_err(db_capability_error)
        })
    }

    fn update_one<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        Box::pin(async move {
            self.store
                .update_one(type_name, selector, change)
                .await
                .map_err(db_capability_error)
        })
    }

    fn update_one_runtime<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        Box::pin(async move {
            self.store
                .update_one_runtime(type_name, selector, change, heap, context)
                .await
                .map_err(db_capability_error)
        })
    }

    fn update_many<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
        change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async move {
            self.store
                .update_many(type_name, query, change)
                .await
                .map_err(db_capability_error)
        })
    }

    fn upsert_by_key<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        insert: DbDocument,
        change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async move {
            self.store
                .upsert_by_key(type_name, key, insert, change)
                .await
                .map_err(db_capability_error)
        })
    }

    fn replace_one<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        value: DbDocument,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        Box::pin(async move {
            self.store
                .replace_one(type_name, selector, value)
                .await
                .map_err(db_capability_error)
        })
    }

    fn replace_one_runtime<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
        value: &'a RuntimeValue,
        heap: &'a mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        Box::pin(async move {
            self.store
                .replace_one_runtime(type_name, selector, value, heap, context)
                .await
                .map_err(db_capability_error)
        })
    }

    fn delete_one<'a>(
        &'a self,
        type_name: &'a str,
        selector: DbOneSelector,
    ) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async move {
            self.store
                .delete_one(type_name, selector)
                .await
                .map_err(db_capability_error)
        })
    }

    fn delete_many<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async move {
            self.store
                .delete_many(type_name, query)
                .await
                .map_err(db_capability_error)
        })
    }

    fn count<'a>(&'a self, type_name: &'a str, query: DbQuery) -> DbCapabilityFuture<'a, u64> {
        Box::pin(async move {
            self.store
                .count(type_name, query)
                .await
                .map_err(db_capability_error)
        })
    }

    fn exists_by_key<'a>(&'a self, type_name: &'a str, key: DbKey) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async move {
            self.store
                .exists_by_key(type_name, key)
                .await
                .map_err(db_capability_error)
        })
    }

    fn exists_by_query<'a>(
        &'a self,
        type_name: &'a str,
        query: DbQuery,
    ) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async move {
            self.store
                .exists_by_query(type_name, query)
                .await
                .map_err(db_capability_error)
        })
    }

    fn claim_lease<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>> {
        Box::pin(async move {
            self.store
                .claim_lease(type_name, key, slot)
                .await
                .map(|handle| {
                    handle.map(|handle| {
                        DbCapabilityLeaseHandle::new(
                            DbCapabilityLeaseHold::new(Arc::new(ServiceDbLeaseHold(handle.hold))),
                            handle.value,
                            handle.ttl_ms,
                        )
                    })
                })
                .map_err(db_capability_error)
        })
    }

    fn renew_lease<'a>(&'a self, hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async move {
            let hold = hold.downcast_ref::<ServiceDbLeaseHold>().ok_or_else(|| {
                DbCapabilityError::decode("db lease hold belongs to a different store")
            })?;
            self.store
                .renew_lease(&hold.0)
                .await
                .map_err(db_capability_error)
        })
    }

    fn release_lease<'a>(&'a self, hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let hold = hold.downcast_ref::<ServiceDbLeaseHold>().ok_or_else(|| {
                DbCapabilityError::decode("db lease hold belongs to a different store")
            })?;
            self.store
                .release_lease(&hold.0)
                .await
                .map_err(db_capability_error)
        })
    }

    fn read_lease<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<Value>> {
        Box::pin(async move {
            self.store
                .read_lease(type_name, key, slot)
                .await
                .map_err(db_capability_error)
        })
    }

    fn lease_lost(&self) -> Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        Box::pin(self.store.lease_lost())
    }

    fn insert_skiff_file_record<'a>(
        &'a self,
        record: FileCapabilityRecord,
    ) -> DbCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.store
                .insert_skiff_file_record(record)
                .await
                .map_err(db_capability_error)
        })
    }

    fn find_skiff_file_by_id<'a>(
        &'a self,
        id: &'a str,
    ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>> {
        Box::pin(async move {
            self.store
                .find_skiff_file_by_id(id)
                .await
                .map_err(db_capability_error)
        })
    }

    fn delete_skiff_file_by_id<'a>(&'a self, id: &'a str) -> DbCapabilityFuture<'a, u64> {
        Box::pin(async move {
            self.store
                .delete_skiff_file_by_id(id)
                .await
                .map_err(db_capability_error)
        })
    }
}

fn db_capability_error(error: ServiceDbError) -> DbCapabilityError {
    DbCapabilityError::opaque(error)
}
