use std::{
    any::Any,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use serde_json::{json, Value};
use skiff_runtime_boundary::recoverable::FailClosedRecoverableBehaviorHooks;
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
    },
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};
use tokio::sync::oneshot;

use super::*;

#[derive(Default)]
struct TestStoreState {
    legacy_runtime_calls: AtomicUsize,
    raw_calls: AtomicUsize,
    wait_starts: AtomicUsize,
    finalize_calls: AtomicUsize,
    create_finalize_fails: AtomicBool,
    replace_wait_fails: AtomicBool,
    create_gate: Mutex<Option<oneshot::Receiver<()>>>,
}

struct RawTestStore {
    state: Arc<TestStoreState>,
}

impl RawTestStore {
    fn new(state: Arc<TestStoreState>) -> Self {
        Self { state }
    }

    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        self.state.raw_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        self.state.raw_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.state.raw_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }

    fn find_one_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.state.raw_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Some(DbDocument::new(json!({ "id": "raw-1" })))) })
    }

    fn find_one_by_key_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.legacy_runtime_calls();
        Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
    }

    fn find_one_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        Box::pin(async { Ok(None) })
    }

    fn find_one_by_query_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.legacy_runtime_calls();
        Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
    }

    fn find_many_page<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, DbPageResult> {
        Box::pin(async { Ok(DbPageResult { values: Vec::new() }) })
    }

    fn find_many_page_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Vec<RuntimeValue>> {
        self.legacy_runtime_calls();
        Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
    }

    fn create<'a>(
        &'a self,
        _type_name: &'a str,
        value: DbDocument,
    ) -> DbCapabilityFuture<'a, DbDocument> {
        Box::pin(async move { Ok(value) })
    }

    fn create_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _value: &'a RuntimeValue,
        _heap: &'a RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, RuntimeValue> {
        self.legacy_runtime_calls();
        Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
    }

    fn insert_many_result<'a>(
        &'a self,
        _type_name: &'a str,
        _values: Vec<DbDocument>,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async { Ok(DbWriteResult::new(json!({ "inserted": 0 }))) })
    }

    fn update_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        Box::pin(async { Ok(None) })
    }

    fn update_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _change: DbRuntimeChange,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.legacy_runtime_calls();
        Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
    }

    fn update_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async { Ok(DbWriteResult::new(json!({ "updated": 0 }))) })
    }

    fn upsert_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _insert: DbDocument,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async { Ok(DbWriteResult::new(json!({ "upserted": 0 }))) })
    }

    fn replace_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: DbDocument,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        Box::pin(async { Ok(None) })
    }

    fn replace_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: &'a RuntimeValue,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.legacy_runtime_calls();
        Box::pin(async { Err(DbCapabilityError::decode("legacy runtime path called")) })
    }

    fn delete_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
    ) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn delete_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        Box::pin(async { Ok(DbWriteResult::new(json!({ "deleted": 0 }))) })
    }

    fn count<'a>(&'a self, _type_name: &'a str, _query: DbQuery) -> DbCapabilityFuture<'a, u64> {
        Box::pin(async { Ok(0) })
    }

    fn exists_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
    ) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn exists_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn claim_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>> {
        self.state.raw_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(Some(DbCapabilityLeaseHandle::new(
                test_hold(),
                DbDocument::new(json!({ "lease": "value" })),
                1_000,
            )))
        })
    }

    fn renew_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, bool> {
        Box::pin(async { Ok(true) })
    }

    fn release_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, ()> {
        self.state.raw_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn read_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<Value>> {
        self.state.raw_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Some(json!({ "lease": "value" }))) })
    }

    fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async { false })
    }

    fn insert_skiff_file_record<'a>(
        &'a self,
        _record: FileCapabilityRecord,
    ) -> DbCapabilityFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn find_skiff_file_by_id<'a>(
        &'a self,
        _id: &'a str,
    ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>> {
        Box::pin(async { Ok(None) })
    }

    fn delete_skiff_file_by_id<'a>(&'a self, _id: &'a str) -> DbCapabilityFuture<'a, u64> {
        Box::pin(async { Ok(0) })
    }

    fn legacy_runtime_calls(&self) {
        self.state
            .legacy_runtime_calls
            .fetch_add(1, Ordering::SeqCst);
    }
}

macro_rules! delegate_raw_store_api {
    ($field:ident) => {
        fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
            self.$field.begin_transaction()
        }

        fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
            self.$field.commit_transaction()
        }

        fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.$field.abort_transaction()
        }

        fn find_one_by_key<'a>(
            &'a self,
            type_name: &'a str,
            key: DbKey,
            projection: Option<Vec<FieldPath>>,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            self.$field.find_one_by_key(type_name, key, projection)
        }

        fn find_one_by_key_runtime<'a>(
            &'a self,
            type_name: &'a str,
            key: DbKey,
            projection: Option<Vec<FieldPath>>,
            heap: &'a mut RequestHeap,
            context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
            self.$field
                .find_one_by_key_runtime(type_name, key, projection, heap, context)
        }

        fn find_one_by_query<'a>(
            &'a self,
            type_name: &'a str,
            query: DbQuery,
            order: Vec<DbOrderEntry>,
            projection: Option<Vec<FieldPath>>,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            self.$field
                .find_one_by_query(type_name, query, order, projection)
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
            self.$field
                .find_one_by_query_runtime(type_name, query, order, projection, heap, context)
        }

        fn find_many_page<'a>(
            &'a self,
            type_name: &'a str,
            query: DbQuery,
            options: ServiceDbFindOptions,
            projection: Option<Vec<FieldPath>>,
        ) -> DbCapabilityFuture<'a, DbPageResult> {
            self.$field
                .find_many_page(type_name, query, options, projection)
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
            self.$field
                .find_many_page_runtime(type_name, query, options, projection, heap, context)
        }

        fn create<'a>(
            &'a self,
            type_name: &'a str,
            value: DbDocument,
        ) -> DbCapabilityFuture<'a, DbDocument> {
            self.$field.create(type_name, value)
        }

        fn create_runtime<'a>(
            &'a self,
            type_name: &'a str,
            value: &'a RuntimeValue,
            heap: &'a RequestHeap,
            context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, RuntimeValue> {
            self.$field.create_runtime(type_name, value, heap, context)
        }

        fn insert_many_result<'a>(
            &'a self,
            type_name: &'a str,
            values: Vec<DbDocument>,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            self.$field.insert_many_result(type_name, values)
        }

        fn update_one<'a>(
            &'a self,
            type_name: &'a str,
            selector: DbOneSelector,
            change: ServiceDbChange,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            self.$field.update_one(type_name, selector, change)
        }

        fn update_one_runtime<'a>(
            &'a self,
            type_name: &'a str,
            selector: DbOneSelector,
            change: DbRuntimeChange,
            heap: &'a mut RequestHeap,
            context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
            self.$field
                .update_one_runtime(type_name, selector, change, heap, context)
        }

        fn update_many<'a>(
            &'a self,
            type_name: &'a str,
            query: DbQuery,
            change: ServiceDbChange,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            self.$field.update_many(type_name, query, change)
        }

        fn upsert_by_key<'a>(
            &'a self,
            type_name: &'a str,
            key: DbKey,
            insert: DbDocument,
            change: ServiceDbChange,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            self.$field.upsert_by_key(type_name, key, insert, change)
        }

        fn replace_one<'a>(
            &'a self,
            type_name: &'a str,
            selector: DbOneSelector,
            value: DbDocument,
        ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
            self.$field.replace_one(type_name, selector, value)
        }

        fn replace_one_runtime<'a>(
            &'a self,
            type_name: &'a str,
            selector: DbOneSelector,
            value: &'a RuntimeValue,
            heap: &'a mut RequestHeap,
            context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
            self.$field
                .replace_one_runtime(type_name, selector, value, heap, context)
        }

        fn delete_one<'a>(
            &'a self,
            type_name: &'a str,
            selector: DbOneSelector,
        ) -> DbCapabilityFuture<'a, bool> {
            self.$field.delete_one(type_name, selector)
        }

        fn delete_many<'a>(
            &'a self,
            type_name: &'a str,
            query: DbQuery,
        ) -> DbCapabilityFuture<'a, DbWriteResult> {
            self.$field.delete_many(type_name, query)
        }

        fn count<'a>(&'a self, type_name: &'a str, query: DbQuery) -> DbCapabilityFuture<'a, u64> {
            self.$field.count(type_name, query)
        }

        fn exists_by_key<'a>(
            &'a self,
            type_name: &'a str,
            key: DbKey,
        ) -> DbCapabilityFuture<'a, bool> {
            self.$field.exists_by_key(type_name, key)
        }

        fn exists_by_query<'a>(
            &'a self,
            type_name: &'a str,
            query: DbQuery,
        ) -> DbCapabilityFuture<'a, bool> {
            self.$field.exists_by_query(type_name, query)
        }

        fn claim_lease<'a>(
            &'a self,
            type_name: &'a str,
            key: DbKey,
            slot: &'a str,
        ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>> {
            self.$field.claim_lease(type_name, key, slot)
        }

        fn renew_lease<'a>(
            &'a self,
            hold: &'a DbCapabilityLeaseHold,
        ) -> DbCapabilityFuture<'a, bool> {
            self.$field.renew_lease(hold)
        }

        fn release_lease<'a>(
            &'a self,
            hold: &'a DbCapabilityLeaseHold,
        ) -> DbCapabilityFuture<'a, ()> {
            self.$field.release_lease(hold)
        }

        fn read_lease<'a>(
            &'a self,
            type_name: &'a str,
            key: DbKey,
            slot: &'a str,
        ) -> DbCapabilityFuture<'a, Option<Value>> {
            self.$field.read_lease(type_name, key, slot)
        }

        fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
            self.$field.lease_lost()
        }

        fn insert_skiff_file_record<'a>(
            &'a self,
            record: FileCapabilityRecord,
        ) -> DbCapabilityFuture<'a, ()> {
            self.$field.insert_skiff_file_record(record)
        }

        fn find_skiff_file_by_id<'a>(
            &'a self,
            id: &'a str,
        ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>> {
            self.$field.find_skiff_file_by_id(id)
        }

        fn delete_skiff_file_by_id<'a>(&'a self, id: &'a str) -> DbCapabilityFuture<'a, u64> {
            self.$field.delete_skiff_file_by_id(id)
        }
    };
}

struct PreparedFakeStore {
    raw: RawTestStore,
}

impl PreparedFakeStore {
    fn state(&self) -> Arc<TestStoreState> {
        Arc::clone(&self.raw.state)
    }

    fn ready<T>(&self, value: T) -> PreparedDbRuntimeOperation<T>
    where
        T: Send + 'static,
    {
        let wait_state = self.state();
        PreparedDbRuntimeOperation::new(async move {
            wait_state.wait_starts.fetch_add(1, Ordering::SeqCst);
            let finalize_state = Arc::clone(&wait_state);
            Ok(DbRuntimeFinalizer::new(move |_heap| {
                finalize_state.finalize_calls.fetch_add(1, Ordering::SeqCst);
                Ok(value)
            }))
        })
    }
}

impl DbCapabilityStoreApi for PreparedFakeStore {
    delegate_raw_store_api!(raw);

    fn prepare_find_one_by_key_runtime(
        &self,
        _type_name: &str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        Ok(self.ready(Some(RuntimeValue::String("key".to_string()))))
    }

    fn prepare_find_one_by_query_runtime(
        &self,
        _type_name: &str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        Ok(self.ready(Some(RuntimeValue::String("query".to_string()))))
    }

    fn prepare_find_many_page_runtime(
        &self,
        _type_name: &str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbManyRuntimeOperation> {
        Ok(self.ready(vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)]))
    }

    fn prepare_create_runtime(
        &self,
        _type_name: &str,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbValueRuntimeOperation> {
        if let RuntimeValue::Heap(handle) = value {
            heap.get(*handle)
                .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
        }
        let gate = self
            .raw
            .state
            .create_gate
            .lock()
            .expect("create gate lock")
            .take();
        let wait_state = self.state();
        Ok(PreparedDbRuntimeOperation::new(async move {
            wait_state.wait_starts.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = gate {
                gate.await
                    .map_err(|_| DbCapabilityError::decode("create gate dropped"))?;
            }
            let finalize_state = Arc::clone(&wait_state);
            Ok(DbRuntimeFinalizer::new(move |heap| {
                finalize_state.finalize_calls.fetch_add(1, Ordering::SeqCst);
                if finalize_state.create_finalize_fails.load(Ordering::SeqCst) {
                    heap.alloc_array(Vec::new())
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                    heap.alloc_array(Vec::new())
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                    unreachable!("second allocation must exceed the test heap limit");
                }
                let handle = heap
                    .alloc_array(vec![RuntimeValue::String("created".to_string())])
                    .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                Ok(RuntimeValue::Heap(handle))
            }))
        }))
    }

    fn prepare_update_one_runtime(
        &self,
        _type_name: &str,
        _selector: DbOneSelector,
        _change: DbRuntimeChange,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        Ok(self.ready(Some(RuntimeValue::Bool(true))))
    }

    fn prepare_replace_one_runtime(
        &self,
        _type_name: &str,
        _selector: DbOneSelector,
        _value: &RuntimeValue,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        let wait_state = self.state();
        Ok(PreparedDbRuntimeOperation::new(async move {
            wait_state.wait_starts.fetch_add(1, Ordering::SeqCst);
            if wait_state.replace_wait_fails.load(Ordering::SeqCst) {
                return Err(DbCapabilityError::decode("prepared replace failed"));
            }
            let finalize_state = Arc::clone(&wait_state);
            Ok(DbRuntimeFinalizer::new(move |_heap| {
                finalize_state.finalize_calls.fetch_add(1, Ordering::SeqCst);
                Ok(None)
            }))
        }))
    }
}

struct DefaultPreparedStore {
    raw: RawTestStore,
}

impl DbCapabilityStoreApi for DefaultPreparedStore {
    delegate_raw_store_api!(raw);
}

#[derive(Debug)]
struct TestLeaseHold(u64);

impl DbCapabilityLeaseHoldHandle for TestLeaseHold {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn eq_handle(&self, other: &dyn DbCapabilityLeaseHoldHandle) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| other.0 == self.0)
    }
}

fn test_hold() -> DbCapabilityLeaseHold {
    DbCapabilityLeaseHold::new(Arc::new(TestLeaseHold(7)))
}

fn runtime_context() -> DbRecoverableRuntimeContext {
    DbRecoverableRuntimeContext {
        behavior_hooks: Arc::new(FailClosedRecoverableBehaviorHooks),
        expected_plans: DbRecoverableRuntimeExpectedPlans::default(),
        artifact_identity: "artifact:test".to_string(),
        build_id: "build:test".to_string(),
        boundary_context: RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::DbValue,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        ),
        retention_expires_at_epoch_millis: None,
    }
}

fn prepared_store(gate: Option<oneshot::Receiver<()>>) -> (DbCapabilityStore, Arc<TestStoreState>) {
    let state = Arc::new(TestStoreState {
        create_gate: Mutex::new(gate),
        ..TestStoreState::default()
    });
    let store = DbCapabilityStore::new(PreparedFakeStore {
        raw: RawTestStore::new(Arc::clone(&state)),
    });
    (store, state)
}

fn default_prepared_store() -> (DbCapabilityStore, Arc<TestStoreState>) {
    let state = Arc::new(TestStoreState::default());
    let store = DbCapabilityStore::new(DefaultPreparedStore {
        raw: RawTestStore::new(Arc::clone(&state)),
    });
    (store, state)
}

async fn wait_until_started(state: &TestStoreState, expected: usize) {
    for _ in 0..32 {
        if state.wait_starts.load(Ordering::SeqCst) == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(state.wait_starts.load(Ordering::SeqCst), expected);
}

fn assert_unavailable<T>(result: DbCapabilityResult<PreparedDbRuntimeOperation<T>>) {
    let error = match result {
        Ok(_) => panic!("default prepared runtime operation must fail closed"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("prepared DB runtime operation is unavailable"),
        "{error}"
    );
}

#[tokio::test]
async fn prepared_db_pending_wait_releases_caller_heap_until_finalize() {
    let (gate_tx, gate_rx) = oneshot::channel();
    let (store, state) = prepared_store(Some(gate_rx));
    let mut heap = RequestHeap::default();
    let input_handle = heap
        .alloc_array(vec![RuntimeValue::String("input".to_string())])
        .expect("input allocation");
    let input = RuntimeValue::Heap(input_handle);

    let prepared = store
        .prepare_create_runtime("Item", &input, &mut heap, runtime_context())
        .expect("prepare create");

    let independent_handle = heap
        .alloc_array(vec![RuntimeValue::String("independent".to_string())])
        .expect("caller heap must be independently mutable after prepare");
    let checkpoint_before_wait = heap.checkpoint();
    let stats_before_wait = heap.stats();
    let len_before_wait = heap.len();

    let wait_task = tokio::spawn(prepared.into_wait());
    wait_until_started(&state, 1).await;
    assert_eq!(heap.checkpoint(), checkpoint_before_wait);
    assert_eq!(heap.stats(), stats_before_wait);
    assert_eq!(heap.len(), len_before_wait);
    heap.get(input_handle).expect("input node remains");
    heap.get(independent_handle)
        .expect("independent caller mutation remains");

    gate_tx.send(()).expect("release prepared wait");
    let completion = wait_task
        .await
        .expect("wait task joins")
        .expect("prepared wait succeeds");
    assert_eq!(state.wait_starts.load(Ordering::SeqCst), 1);
    assert_eq!(heap.checkpoint(), checkpoint_before_wait);
    assert_eq!(heap.stats(), stats_before_wait);
    assert_eq!(heap.len(), len_before_wait);

    let value = completion.finalize(&mut heap).expect("finalize create");
    assert!(matches!(value, RuntimeValue::Heap(_)));
    assert_eq!(heap.len(), len_before_wait + 1);
    assert_eq!(state.finalize_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn prepared_db_ready_and_pending_waits_start_once() {
    let (ready_store, ready_state) = prepared_store(None);
    let mut ready_heap = RequestHeap::default();
    let ready = ready_store
        .prepare_find_one_by_key_runtime(
            "Item",
            DbKey::new(json!("one")),
            None,
            &mut ready_heap,
            runtime_context(),
        )
        .expect("prepare ready find");
    let ready_completion = ready.into_wait().await.expect("ready wait");
    assert_eq!(ready_state.wait_starts.load(Ordering::SeqCst), 1);
    let ready_value = ready_completion
        .finalize(&mut ready_heap)
        .expect("ready finalize");
    assert_eq!(ready_value, Some(RuntimeValue::String("key".to_string())));

    let (gate_tx, gate_rx) = oneshot::channel();
    let (pending_store, pending_state) = prepared_store(Some(gate_rx));
    let mut pending_heap = RequestHeap::default();
    let pending = pending_store
        .prepare_create_runtime(
            "Item",
            &RuntimeValue::Null,
            &mut pending_heap,
            runtime_context(),
        )
        .expect("prepare pending create");
    let pending_task = tokio::spawn(pending.into_wait());
    wait_until_started(&pending_state, 1).await;
    gate_tx.send(()).expect("release pending wait");
    let pending_completion = pending_task
        .await
        .expect("pending task joins")
        .expect("pending wait succeeds");
    pending_completion
        .finalize(&mut pending_heap)
        .expect("pending finalize");
    assert_eq!(pending_state.wait_starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn prepared_db_drop_and_error_do_not_restart_wait_or_finalize() {
    let (gate_tx, gate_rx) = oneshot::channel();
    let (drop_store, drop_state) = prepared_store(Some(gate_rx));
    let mut drop_heap = RequestHeap::default();
    let prepared = drop_store
        .prepare_create_runtime(
            "Item",
            &RuntimeValue::Null,
            &mut drop_heap,
            runtime_context(),
        )
        .expect("prepare dropped wait");
    let wait_task = tokio::spawn(prepared.into_wait());
    wait_until_started(&drop_state, 1).await;
    wait_task.abort();
    let _ = wait_task.await;
    assert!(gate_tx.send(()).is_err(), "dropped wait owns its gate");
    assert_eq!(drop_state.wait_starts.load(Ordering::SeqCst), 1);
    assert_eq!(drop_state.finalize_calls.load(Ordering::SeqCst), 0);

    let (error_store, error_state) = prepared_store(None);
    error_state.replace_wait_fails.store(true, Ordering::SeqCst);
    let mut error_heap = RequestHeap::default();
    let prepared = error_store
        .prepare_replace_one_runtime(
            "Item",
            DbOneSelector::key(json!("one")),
            &RuntimeValue::Null,
            &mut error_heap,
            runtime_context(),
        )
        .expect("prepare failing replace");
    let error = prepared.into_wait().await.err().expect("wait must fail");
    assert_eq!(error.to_string(), "prepared replace failed");
    assert_eq!(error_state.wait_starts.load(Ordering::SeqCst), 1);
    assert_eq!(error_state.finalize_calls.load(Ordering::SeqCst), 0);

    error_state
        .replace_wait_fails
        .store(false, Ordering::SeqCst);
    let completion = error_store
        .prepare_replace_one_runtime(
            "Item",
            DbOneSelector::key(json!("two")),
            &RuntimeValue::Null,
            &mut error_heap,
            runtime_context(),
        )
        .expect("prepare replace")
        .into_wait()
        .await
        .expect("replace wait");
    let finalize_count = error_state.finalize_calls.load(Ordering::SeqCst);
    drop(completion);
    assert_eq!(
        error_state.finalize_calls.load(Ordering::SeqCst),
        finalize_count,
        "dropping the one-shot completion must not run it"
    );
    assert_eq!(error_state.wait_starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn prepared_db_finalize_resource_failure_rolls_back_partial_allocations() {
    let (store, state) = prepared_store(None);
    state.create_finalize_fails.store(true, Ordering::SeqCst);
    let mut heap = RequestHeap::new(RequestHeapLimits {
        max_nodes: 2,
        ..RequestHeapLimits::default()
    });
    let baseline_handle = heap
        .alloc_array(vec![RuntimeValue::String("baseline".to_string())])
        .expect("baseline allocation");
    let baseline = RuntimeValue::Heap(baseline_handle);
    let completion = store
        .prepare_create_runtime("Item", &baseline, &mut heap, runtime_context())
        .expect("prepare create")
        .into_wait()
        .await
        .expect("wait succeeds before finalization");
    let checkpoint = heap.checkpoint();
    let stats = heap.stats();

    let error = completion
        .finalize(&mut heap)
        .expect_err("second finalizer allocation must exceed max_nodes");
    assert!(error.to_string().contains("max heap nodes"), "{error}");
    assert_eq!(heap.checkpoint(), checkpoint);
    assert_eq!(heap.stats(), stats);
    heap.get(baseline_handle)
        .expect("pre-existing node must survive rollback");
    assert_eq!(state.finalize_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn prepared_db_typed_results_cover_all_runtime_paths_without_confusion() {
    let (store, state) = prepared_store(None);
    let mut heap = RequestHeap::default();

    let key_value: Option<RuntimeValue> = store
        .prepare_find_one_by_key_runtime(
            "Item",
            DbKey::new(json!("one")),
            None,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare key find")
        .into_wait()
        .await
        .expect("key wait")
        .finalize(&mut heap)
        .expect("key finalize");
    assert_eq!(key_value, Some(RuntimeValue::String("key".to_string())));

    let query_value: Option<RuntimeValue> = store
        .prepare_find_one_by_query_runtime(
            "Item",
            DbQuery::new(json!({})),
            Vec::new(),
            None,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare query find")
        .into_wait()
        .await
        .expect("query wait")
        .finalize(&mut heap)
        .expect("query finalize");
    assert_eq!(query_value, Some(RuntimeValue::String("query".to_string())));

    let many_value: Vec<RuntimeValue> = store
        .prepare_find_many_page_runtime(
            "Item",
            DbQuery::new(json!({})),
            ServiceDbFindOptions::default(),
            None,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare many")
        .into_wait()
        .await
        .expect("many wait")
        .finalize(&mut heap)
        .expect("many finalize");
    assert_eq!(
        many_value,
        vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)]
    );

    let created_value: RuntimeValue = store
        .prepare_create_runtime("Item", &RuntimeValue::Null, &mut heap, runtime_context())
        .expect("prepare create")
        .into_wait()
        .await
        .expect("create wait")
        .finalize(&mut heap)
        .expect("create finalize");
    assert!(matches!(created_value, RuntimeValue::Heap(_)));

    let updated_value: Option<RuntimeValue> = store
        .prepare_update_one_runtime(
            "Item",
            DbOneSelector::key(json!("one")),
            DbRuntimeChange::default(),
            &mut heap,
            runtime_context(),
        )
        .expect("prepare update")
        .into_wait()
        .await
        .expect("update wait")
        .finalize(&mut heap)
        .expect("update finalize");
    assert_eq!(updated_value, Some(RuntimeValue::Bool(true)));

    let replaced_value: Option<RuntimeValue> = store
        .prepare_replace_one_runtime(
            "Item",
            DbOneSelector::key(json!("one")),
            &RuntimeValue::Null,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare replace")
        .into_wait()
        .await
        .expect("replace wait")
        .finalize(&mut heap)
        .expect("replace finalize");
    assert_eq!(replaced_value, None);
    assert_eq!(state.wait_starts.load(Ordering::SeqCst), 6);
    assert_eq!(state.finalize_calls.load(Ordering::SeqCst), 6);
}

#[test]
fn prepared_db_default_implementation_fails_closed_without_legacy_fallback() {
    let (store, state) = default_prepared_store();
    let mut heap = RequestHeap::default();
    let value = RuntimeValue::Null;

    assert_unavailable(store.prepare_find_one_by_key_runtime(
        "Item",
        DbKey::new(json!("one")),
        None,
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_find_one_by_query_runtime(
        "Item",
        DbQuery::new(json!({})),
        Vec::new(),
        None,
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_find_many_page_runtime(
        "Item",
        DbQuery::new(json!({})),
        ServiceDbFindOptions::default(),
        None,
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_create_runtime("Item", &value, &mut heap, runtime_context()));
    assert_unavailable(store.prepare_update_one_runtime(
        "Item",
        DbOneSelector::key(json!("one")),
        DbRuntimeChange::default(),
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_replace_one_runtime(
        "Item",
        DbOneSelector::key(json!("one")),
        &value,
        &mut heap,
        runtime_context(),
    ));

    assert_eq!(
        state.legacy_runtime_calls.load(Ordering::SeqCst),
        0,
        "prepared defaults must never call heap-borrowing runtime methods"
    );
}

#[tokio::test]
async fn prepared_db_addition_preserves_raw_transaction_and_lease_forwarding() {
    let (store, state) = default_prepared_store();
    store.begin_transaction().await.expect("begin transaction");
    let raw = store
        .find_one_by_key("Item", DbKey::new(json!("raw-1")), None)
        .await
        .expect("raw find")
        .expect("raw document");
    assert_eq!(raw.as_value(), &json!({ "id": "raw-1" }));
    let lease = store
        .claim_lease("Item", DbKey::new(json!("raw-1")), "worker")
        .await
        .expect("claim lease")
        .expect("lease");
    assert_eq!(lease.value.as_value(), &json!({ "lease": "value" }));
    let read = store
        .read_lease("Item", DbKey::new(json!("raw-1")), "worker")
        .await
        .expect("read lease");
    assert_eq!(read, Some(json!({ "lease": "value" })));
    store
        .release_lease(&lease.hold)
        .await
        .expect("release lease");
    store
        .commit_transaction()
        .await
        .expect("commit transaction");
    store.abort_transaction().await;
    assert_eq!(state.raw_calls.load(Ordering::SeqCst), 7);
    assert_eq!(state.legacy_runtime_calls.load(Ordering::SeqCst), 0);
}
