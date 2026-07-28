use std::{any::Any, future::Future, pin::Pin, sync::Arc};

use skiff_runtime_capability_context::{
    DbCapabilityContextApi, DbCapabilityError, DbCapabilityFuture, DbCapabilityLeaseHandle,
    DbCapabilityLeaseHold, DbCapabilityLeaseHoldHandle, DbCapabilityResult, DbCapabilityStore,
    DbCapabilityStoreApi, DbDocument, DbKey, DbOneSelector, DbOrderEntry, DbPageResult, DbQuery,
    DbRecoverableRuntimeContext, DbRuntimeChange, DbRuntimeFinalizer, DbWriteResult, FieldPath,
    FileCapabilityRecord, PreparedDbValueRuntimeOperation, ServiceDbChange, ServiceDbFindOptions,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::{DbEventKind, DbPhase, FakeDbState, PreparedFinalize};

#[derive(Clone)]
pub(in crate::program_db::tests) struct FakeDbContext {
    state: Arc<FakeDbState>,
    store: DbCapabilityStore,
}

impl FakeDbContext {
    pub(in crate::program_db::tests) fn new(state: Arc<FakeDbState>) -> Self {
        let store = DbCapabilityStore::new(FakeDbStore::new(Arc::clone(&state)));
        Self { state, store }
    }

    pub(in crate::program_db::tests) fn store(&self) -> DbCapabilityStore {
        self.store.clone()
    }
}

impl DbCapabilityContextApi for FakeDbContext {
    fn require_store(
        &self,
        _target: &str,
        _unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore> {
        self.state.record_context_require();
        Ok(self.store())
    }
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

pub(in crate::program_db::tests) fn test_lease_hold(id: u64) -> DbCapabilityLeaseHold {
    DbCapabilityLeaseHold::new(Arc::new(TestLeaseHold(id)))
}

pub(in crate::program_db::tests) fn test_lease_handle(
    id: u64,
    value: serde_json::Value,
    ttl_ms: u64,
) -> DbCapabilityLeaseHandle {
    DbCapabilityLeaseHandle::new(test_lease_hold(id), DbDocument::new(value), ttl_ms)
}

struct FinalizerDrop {
    state: Arc<FakeDbState>,
    terminal: bool,
}

impl Drop for FinalizerDrop {
    fn drop(&mut self) {
        self.state.record(
            DbPhase::PreparedCreateFinalize,
            if self.terminal {
                DbEventKind::DropAfterTerminal
            } else {
                DbEventKind::DropBeforeTerminal
            },
        );
    }
}

#[derive(Clone)]
pub(in crate::program_db::tests) struct FakeDbStore {
    state: Arc<FakeDbState>,
}

impl FakeDbStore {
    pub(in crate::program_db::tests) fn new(state: Arc<FakeDbState>) -> Self {
        Self { state }
    }

    fn unexpected(&self, method: &str) -> ! {
        panic!("unexpected DB method {method}")
    }

    fn legacy_runtime(&self, method: &str) -> ! {
        self.state.record_legacy_runtime_call();
        self.unexpected(method)
    }
}

impl DbCapabilityStoreApi for FakeDbStore {
    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(self.state.begin.take(&self.state, DbPhase::Begin))
    }

    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        Box::pin(self.state.commit.take(&self.state, DbPhase::Commit))
    }

    fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let future = self.state.abort.take(&self.state, DbPhase::Abort);
        Box::pin(async move {
            future
                .await
                .unwrap_or_else(|error| panic!("abort_transaction failed: {error}"));
        })
    }

    fn find_one_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("find_one_by_key")
    }

    fn find_one_by_key_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.legacy_runtime("find_one_by_key_runtime")
    }

    fn find_one_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("find_one_by_query")
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
        self.legacy_runtime("find_one_by_query_runtime")
    }

    fn find_many_page<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, DbPageResult> {
        self.unexpected("find_many_page")
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
        self.legacy_runtime("find_many_page_runtime")
    }

    fn create<'a>(
        &'a self,
        _type_name: &'a str,
        _value: DbDocument,
    ) -> DbCapabilityFuture<'a, DbDocument> {
        let (script, phase) = if self.state.raw_create.remaining() > 0 {
            (&self.state.raw_create, DbPhase::RawCreate)
        } else {
            (&self.state.body_create, DbPhase::BodyCreate)
        };
        Box::pin(script.take(&self.state, phase))
    }

    fn create_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _value: &'a RuntimeValue,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, RuntimeValue> {
        self.legacy_runtime("create_runtime")
    }

    fn prepare_create_runtime(
        &self,
        _type_name: &str,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbValueRuntimeOperation> {
        let RuntimeValue::Heap(handle) = value else {
            return Err(DbCapabilityError::decode(
                "prepared create fixture requires a heap-attached value",
            ));
        };
        heap.get(*handle)
            .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
        let wait = self
            .state
            .prepared_create
            .take(&self.state, DbPhase::PreparedCreateWait);
        let state = Arc::clone(&self.state);
        Ok(PreparedDbValueRuntimeOperation::new(async move {
            let plan: PreparedFinalize = wait.await?;
            state.record(DbPhase::PreparedCreateFinalize, DbEventKind::Constructed);
            let finalizer_state = Arc::clone(&state);
            Ok(DbRuntimeFinalizer::new(move |heap| {
                let mut drop_guard = FinalizerDrop {
                    state: Arc::clone(&finalizer_state),
                    terminal: false,
                };
                finalizer_state.record(DbPhase::PreparedCreateFinalize, DbEventKind::Poll);
                let result = plan.finalize(heap);
                finalizer_state.record(DbPhase::PreparedCreateFinalize, DbEventKind::Ready);
                drop_guard.terminal = true;
                result
            }))
        }))
    }

    fn insert_many_result<'a>(
        &'a self,
        _type_name: &'a str,
        _values: Vec<DbDocument>,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("insert_many_result")
    }

    fn update_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("update_one")
    }

    fn update_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _change: DbRuntimeChange,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.legacy_runtime("update_one_runtime")
    }

    fn update_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("update_many")
    }

    fn upsert_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _insert: DbDocument,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("upsert_by_key")
    }

    fn replace_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: DbDocument,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("replace_one")
    }

    fn replace_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: &'a RuntimeValue,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.legacy_runtime("replace_one_runtime")
    }

    fn delete_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("delete_one")
    }

    fn delete_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("delete_many")
    }

    fn count<'a>(&'a self, _type_name: &'a str, _query: DbQuery) -> DbCapabilityFuture<'a, u64> {
        self.unexpected("count")
    }

    fn exists_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("exists_by_key")
    }

    fn exists_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("exists_by_query")
    }

    fn claim_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>> {
        Box::pin(self.state.claim.take(&self.state, DbPhase::Claim))
    }

    fn renew_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, bool> {
        Box::pin(self.state.renew.take(&self.state, DbPhase::Renew))
    }

    fn release_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, ()> {
        Box::pin(self.state.release.take(&self.state, DbPhase::Release))
    }

    fn read_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<serde_json::Value>> {
        Box::pin(self.state.read.take(&self.state, DbPhase::Read))
    }

    fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        let future = self.state.lease_lost.take(&self.state, DbPhase::LeaseLost);
        Box::pin(async move {
            future
                .await
                .unwrap_or_else(|error| panic!("lease_lost failed: {error}"))
        })
    }

    fn insert_skiff_file_record<'a>(
        &'a self,
        _record: FileCapabilityRecord,
    ) -> DbCapabilityFuture<'a, ()> {
        self.unexpected("insert_skiff_file_record")
    }

    fn find_skiff_file_by_id<'a>(
        &'a self,
        _id: &'a str,
    ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>> {
        self.unexpected("find_skiff_file_by_id")
    }

    fn delete_skiff_file_by_id<'a>(&'a self, _id: &'a str) -> DbCapabilityFuture<'a, u64> {
        self.unexpected("delete_skiff_file_by_id")
    }
}
