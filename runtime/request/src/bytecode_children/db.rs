//! D6R-owned DB child leaf.
//!
//! The leaf consumes the D6R capability contexts and prepared runtime store
//! APIs, while transaction admission uses the K6 request transaction ledger.
//! A DB operation is only reachable when the exact
//! [`DbCapabilityTarget`] fact is present; missing facts fail closed before
//! any provider call and the leaf never reconstructs a target from a type
//! name.

use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityStore, DbCapabilityTarget, DbKey, DbOneSelector,
    DbRecoverableRuntimeContext, DbRuntimeChange, FieldPath, PreparedDbOptionalRuntimeOperation,
    PreparedDbValueRuntimeOperation,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};
use skiff_runtime_scheduler::{ChildHeapCarrier, ChildHeapCleanupError};

use crate::{
    memory_ledger::{RequestTransactionToken, TransactionLedgerError},
    RequestMemoryLedger,
};

/// Exact runtime identity for one linked DB object target.
///
/// This is the capability-context mirror of the artifact-linked
/// `DbObjectTargetId(PackageArtifactRef, FileIrRef, typeIndex)` identity.
pub type DbObjectTargetId = skiff_runtime_capability_context::DbCapabilityTargetId;

/// Typed, bounded rejection from the D6R DB child leaf.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeDbChildError {
    #[error("db child exact target fact is missing; F6 must emit DbObjectTargetId")]
    MissingExactTarget,
    #[error("db child capability context is missing")]
    MissingCapabilityContext,
    #[error("db child recoverable context is missing")]
    MissingRecoverableContext,
    #[error("db child provider operation failed: {message}")]
    Provider { message: String },
    #[error("db child transaction ledger rejected operation: {0}")]
    TransactionLedger(#[from] TransactionLedgerError),
    #[error("db child commit has no active transaction")]
    CommitWithoutActiveTransaction,
    #[error("db child pending cleanup could not be attached: {0}")]
    PendingCleanup(ChildHeapCleanupError),
}

/// Request-scoped DB child registration.
///
/// The capability context, recoverable context and exact target are all
/// mandatory before a provider operation is reachable. `type_name` is carried
/// only as diagnostic metadata on [`DbCapabilityTarget`]; lookup uses the
/// exact target key.
#[derive(Clone, Default)]
pub struct BytecodeDbChildComposition {
    pub capability_context: Option<DbCapabilityContext>,
    pub recoverable_context: Option<DbRecoverableRuntimeContext>,
    pub exact_target: Option<DbCapabilityTarget>,
}

impl BytecodeDbChildComposition {
    pub fn is_available(&self) -> bool {
        self.capability_context.is_some()
            && self.recoverable_context.is_some()
            && self.exact_target.is_some()
    }

    pub fn exact_target(&self) -> Result<&DbCapabilityTarget, BytecodeDbChildError> {
        self.exact_target
            .as_ref()
            .ok_or(BytecodeDbChildError::MissingExactTarget)
    }

    pub fn recoverable_context(&self) -> Result<DbRecoverableRuntimeContext, BytecodeDbChildError> {
        self.recoverable_context
            .clone()
            .ok_or(BytecodeDbChildError::MissingRecoverableContext)
    }

    pub fn require_store(&self) -> Result<DbCapabilityStore, BytecodeDbChildError> {
        let target = self.exact_target()?;
        let context = self
            .capability_context
            .as_ref()
            .ok_or(BytecodeDbChildError::MissingCapabilityContext)?;
        context
            .require_store(
                target.lookup_key(),
                "exact DB target is not admitted by the request capability context",
            )
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    /// Prepares one exact-key read through the existing prepared DB runtime
    /// lane.
    pub fn prepared_read_one_by_key(
        &self,
        heap: &mut RequestHeap,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
    ) -> Result<PreparedDbOptionalRuntimeOperation, BytecodeDbChildError> {
        let target = self.exact_target()?.clone();
        let store = self.require_store()?;
        let context = self.recoverable_context()?;
        store
            .prepare_find_one_by_key_runtime(target.lookup_key(), key, projection, heap, context)
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    /// Prepares one create/write through the existing prepared DB runtime
    /// lane.
    pub fn prepared_create(
        &self,
        heap: &mut RequestHeap,
        value: &RuntimeValue,
    ) -> Result<PreparedDbValueRuntimeOperation, BytecodeDbChildError> {
        let target = self.exact_target()?.clone();
        let store = self.require_store()?;
        let context = self.recoverable_context()?;
        store
            .prepare_create_runtime(target.lookup_key(), value, heap, context)
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    pub fn prepared_update_one(
        &self,
        heap: &mut RequestHeap,
        selector: DbOneSelector,
        change: DbRuntimeChange,
    ) -> Result<PreparedDbOptionalRuntimeOperation, BytecodeDbChildError> {
        let target = self.exact_target()?.clone();
        let store = self.require_store()?;
        let context = self.recoverable_context()?;
        store
            .prepare_update_one_runtime(target.lookup_key(), selector, change, heap, context)
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    pub fn prepared_replace_one(
        &self,
        heap: &mut RequestHeap,
        selector: DbOneSelector,
        value: &RuntimeValue,
    ) -> Result<PreparedDbOptionalRuntimeOperation, BytecodeDbChildError> {
        let target = self.exact_target()?.clone();
        let store = self.require_store()?;
        let context = self.recoverable_context()?;
        store
            .prepare_replace_one_runtime(target.lookup_key(), selector, value, heap, context)
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    /// Begins the sole request transaction through the K6 request ledger and
    /// then the provider store.
    pub async fn begin_transaction(
        &self,
        memory_ledger: &RequestMemoryLedger,
    ) -> Result<DbTransactionSession, BytecodeDbChildError> {
        self.begin_transaction_with_cleanup(memory_ledger, || {})
            .await
    }

    pub async fn begin_transaction_with_cleanup(
        &self,
        memory_ledger: &RequestMemoryLedger,
        cleanup: impl FnOnce() + Send + 'static,
    ) -> Result<DbTransactionSession, BytecodeDbChildError> {
        let target = self.exact_target()?.clone();
        let store = self.require_store()?;
        let recoverable_context = self.recoverable_context()?;
        let token = memory_ledger.begin_transaction(cleanup)?;
        if let Err(error) = store.begin_transaction().await {
            token.finish();
            return Err(BytecodeDbChildError::Provider {
                message: format!("db begin transaction failed: {error}"),
            });
        }
        Ok(DbTransactionSession {
            store,
            target,
            recoverable_context,
            token: Some(token),
        })
    }
}

/// One request-scoped DB transaction session.
///
/// The affine K6 [`RequestTransactionToken`] is held here. Commit and abort
/// consume it exactly once; dropping an unattached session without an explicit
/// terminal still releases the request ledger through the token's cleanup
/// hook.
#[must_use = "a db transaction session must be committed, aborted, or attached to pending cleanup"]
pub struct DbTransactionSession {
    store: DbCapabilityStore,
    target: DbCapabilityTarget,
    recoverable_context: DbRecoverableRuntimeContext,
    token: Option<RequestTransactionToken>,
}

impl DbTransactionSession {
    pub fn exact_target(&self) -> &DbCapabilityTarget {
        &self.target
    }

    pub fn prepared_read_one_by_key(
        &self,
        heap: &mut RequestHeap,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
    ) -> Result<PreparedDbOptionalRuntimeOperation, BytecodeDbChildError> {
        self.store
            .prepare_find_one_by_key_runtime(
                self.target.lookup_key(),
                key,
                projection,
                heap,
                self.recoverable_context.clone(),
            )
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    pub fn prepared_create(
        &self,
        heap: &mut RequestHeap,
        value: &RuntimeValue,
    ) -> Result<PreparedDbValueRuntimeOperation, BytecodeDbChildError> {
        self.store
            .prepare_create_runtime(
                self.target.lookup_key(),
                value,
                heap,
                self.recoverable_context.clone(),
            )
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    pub fn prepared_update_one(
        &self,
        heap: &mut RequestHeap,
        selector: DbOneSelector,
        change: DbRuntimeChange,
    ) -> Result<PreparedDbOptionalRuntimeOperation, BytecodeDbChildError> {
        self.store
            .prepare_update_one_runtime(
                self.target.lookup_key(),
                selector,
                change,
                heap,
                self.recoverable_context.clone(),
            )
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    pub fn prepared_replace_one(
        &self,
        heap: &mut RequestHeap,
        selector: DbOneSelector,
        value: &RuntimeValue,
    ) -> Result<PreparedDbOptionalRuntimeOperation, BytecodeDbChildError> {
        self.store
            .prepare_replace_one_runtime(
                self.target.lookup_key(),
                selector,
                value,
                heap,
                self.recoverable_context.clone(),
            )
            .map_err(|error| BytecodeDbChildError::Provider {
                message: error.to_string(),
            })
    }

    pub async fn commit(&mut self) -> Result<(), BytecodeDbChildError> {
        let token = self
            .token
            .take()
            .ok_or(BytecodeDbChildError::CommitWithoutActiveTransaction)?;
        let result = self.store.commit_transaction().await;
        token.finish();
        result.map_err(|error| BytecodeDbChildError::Provider {
            message: format!("db commit transaction failed: {error}"),
        })
    }

    pub async fn abort(&mut self) {
        if let Some(token) = self.token.take() {
            self.store.abort_transaction().await;
            token.finish();
        }
    }

    /// Moves the transaction token cleanup into a child heap carrier.
    ///
    /// This is the D6R seam for actual `Pending` commit/abort: once the owner
    /// publishes a pending DB completion, the carrier owns the token and the
    /// Phase 4 pending owner graph runs the bounded cleanup at terminal.
    pub fn attach_pending_cleanup(
        mut self,
        child_heap: &mut ChildHeapCarrier,
    ) -> Result<(), BytecodeDbChildError> {
        let token = self
            .token
            .take()
            .ok_or(BytecodeDbChildError::CommitWithoutActiveTransaction)?;
        child_heap
            .attach_pending_cleanup(Box::new(move || token.finish()))
            .map_err(BytecodeDbChildError::PendingCleanup)
    }

    /// Runs the exact successful pending cleanup path when the carrier still
    /// owns it.
    pub fn finish_pending_cleanup(child_heap: &mut ChildHeapCarrier) {
        if let Some(cleanup) = child_heap.take_pending_cleanup() {
            cleanup();
        }
    }
}

#[cfg(test)]
pub(crate) fn db_child_required_fact() -> String {
    "F6 must emit a VM DB child/effect carrying exact DbObjectTargetId and result plan; \
     the D6R leaf resolves that exact target through DbCapabilityTarget and uses the K6 \
     RequestTransactionLedger/ChildHeapCarrier pending cleanup seam"
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use serde_json::{json, Value};
    use skiff_artifact_model::{
        FileIrRef, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
    };
    use skiff_runtime_boundary::recoverable::FailClosedRecoverableBehaviorHooks;
    use skiff_runtime_capability_context::{
        DbCapabilityContext, DbCapabilityContextApi, DbCapabilityError, DbCapabilityFuture,
        DbCapabilityLeaseHandle, DbCapabilityLeaseHold, DbCapabilityResult, DbCapabilityStore,
        DbCapabilityStoreApi, DbCapabilityTarget, DbCapabilityTargetId, DbDocument, DbKey,
        DbOneSelector, DbOrderEntry, DbPageResult, DbQuery, DbRecoverableRuntimeContext,
        DbRecoverableRuntimeExpectedPlans, DbRuntimeChange, DbRuntimeFinalizer, DbWriteResult,
        FieldPath, FileCapabilityRecord, PreparedDbOptionalRuntimeOperation,
        PreparedDbValueRuntimeOperation, ServiceDbChange, ServiceDbFindOptions,
    };
    use skiff_runtime_model::{
        recoverable::{
            RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
            RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
        },
        request_heap::{RequestHeap, RequestHeapLimits},
        runtime_value::RuntimeValue,
    };
    use skiff_runtime_scheduler::{
        BytecodeSchedulerPorts, ChildHeapCarrier, RequestExecutionContext,
    };
    use skiff_runtime_vm::VmFiber;

    use super::*;
    use crate::vm_heap::RequestVmHeap;

    fn test_target() -> DbCapabilityTarget {
        DbCapabilityTarget::new(
            DbCapabilityTargetId {
                package_artifact_ref: PackageArtifactRef {
                    package_id: "test.local/db".to_string(),
                    package_version: "1.0.0".to_string(),
                    package_build_id: PackageBuildId::new("build:db"),
                    package_local_abi_identity: PackageLocalAbiIdentity::new("abi:db"),
                },
                file_ir_ref: FileIrRef::new("file:db", "test/main.skiff"),
                type_index: 0,
            },
            "Doc",
        )
    }

    fn recoverable_context() -> DbRecoverableRuntimeContext {
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

    #[derive(Clone, Default)]
    struct RecordingDbStore {
        begins: Arc<AtomicUsize>,
        commits: Arc<AtomicUsize>,
        aborts: Arc<AtomicUsize>,
        targets: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingDbStore {
        fn target_was(&self, expected: &str) -> bool {
            self.targets
                .lock()
                .expect("target recording lock")
                .iter()
                .any(|target| target == expected)
        }
    }

    struct TestDbCapabilityContext {
        store: RecordingDbStore,
    }

    impl DbCapabilityContextApi for TestDbCapabilityContext {
        fn require_store(
            &self,
            _target: &str,
            _unavailable_reason: &str,
        ) -> DbCapabilityResult<DbCapabilityStore> {
            Ok(DbCapabilityStore::new(self.store.clone()))
        }
    }

    fn test_composition(store: RecordingDbStore) -> BytecodeDbChildComposition {
        BytecodeDbChildComposition {
            capability_context: Some(DbCapabilityContext::new(TestDbCapabilityContext { store })),
            recoverable_context: Some(recoverable_context()),
            exact_target: Some(test_target()),
        }
    }

    macro_rules! unavailable_async {
        ($method:ident, $ret:ty $(, $arg:ident: $ty:ty)*) => {
            fn $method<'a>(&'a self, $($arg: $ty),*) -> $ret {
                let _ = ($($arg,)*);
                Box::pin(async move {
                    Err(DbCapabilityError::provider_unavailable(
                        "serviceDb",
                        "unused test store method",
                    ))
                })
            }
        };
    }

    #[allow(unused_variables)]
    impl DbCapabilityStoreApi for RecordingDbStore {
        fn begin_transaction(
            &self,
        ) -> skiff_runtime_capability_context::DbCapabilityFuture<'_, ()> {
            self.begins.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn commit_transaction(
            &self,
        ) -> skiff_runtime_capability_context::DbCapabilityFuture<'_, ()> {
            self.commits.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.aborts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }

        fn prepare_find_one_by_key_runtime(
            &self,
            target: &str,
            _key: DbKey,
            _projection: Option<Vec<FieldPath>>,
            _heap: &mut RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
            self.targets
                .lock()
                .expect("target recording lock")
                .push(target.to_string());
            Ok(PreparedDbOptionalRuntimeOperation::new(async {
                Ok(DbRuntimeFinalizer::new(|heap| {
                    let handle = heap
                        .alloc_array(vec![RuntimeValue::String("read".to_string())])
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                    Ok(Some(RuntimeValue::Heap(handle)))
                }))
            }))
        }

        fn prepare_create_runtime(
            &self,
            target: &str,
            value: &RuntimeValue,
            _heap: &mut RequestHeap,
            _context: DbRecoverableRuntimeContext,
        ) -> DbCapabilityResult<PreparedDbValueRuntimeOperation> {
            self.targets
                .lock()
                .expect("target recording lock")
                .push(target.to_string());
            let value = value.clone();
            Ok(PreparedDbValueRuntimeOperation::new(async move {
                Ok(DbRuntimeFinalizer::new(move |heap| {
                    let handle = heap
                        .alloc_array(vec![value])
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                    Ok(RuntimeValue::Heap(handle))
                }))
            }))
        }

        unavailable_async!(find_one_by_key, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, key: DbKey, projection: Option<Vec<FieldPath>>);
        unavailable_async!(find_one_by_key_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, key: DbKey, projection: Option<Vec<FieldPath>>, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
        unavailable_async!(find_one_by_query, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, query: DbQuery, order: Vec<DbOrderEntry>, projection: Option<Vec<FieldPath>>);
        unavailable_async!(find_one_by_query_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, query: DbQuery, order: Vec<DbOrderEntry>, projection: Option<Vec<FieldPath>>, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
        unavailable_async!(find_many_page, DbCapabilityFuture<'a, DbPageResult>, type_name: &'a str, query: DbQuery, options: ServiceDbFindOptions, projection: Option<Vec<FieldPath>>);
        unavailable_async!(find_many_page_runtime, DbCapabilityFuture<'a, Vec<RuntimeValue>>, type_name: &'a str, query: DbQuery, options: ServiceDbFindOptions, projection: Option<Vec<FieldPath>>, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
        unavailable_async!(create, DbCapabilityFuture<'a, DbDocument>, type_name: &'a str, value: DbDocument);
        unavailable_async!(create_runtime, DbCapabilityFuture<'a, RuntimeValue>, type_name: &'a str, value: &'a RuntimeValue, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
        unavailable_async!(insert_many_result, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, values: Vec<DbDocument>);
        unavailable_async!(update_one, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, selector: DbOneSelector, change: ServiceDbChange);
        unavailable_async!(update_one_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, selector: DbOneSelector, change: DbRuntimeChange, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
        unavailable_async!(update_many, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, query: DbQuery, change: ServiceDbChange);
        unavailable_async!(upsert_by_key, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, key: DbKey, insert: DbDocument, change: ServiceDbChange);
        unavailable_async!(replace_one, DbCapabilityFuture<'a, Option<DbDocument>>, type_name: &'a str, selector: DbOneSelector, value: DbDocument);
        unavailable_async!(replace_one_runtime, DbCapabilityFuture<'a, Option<RuntimeValue>>, type_name: &'a str, selector: DbOneSelector, value: &'a RuntimeValue, heap: &'a mut RequestHeap, context: DbRecoverableRuntimeContext);
        unavailable_async!(delete_one, DbCapabilityFuture<'a, bool>, type_name: &'a str, selector: DbOneSelector);
        unavailable_async!(delete_many, DbCapabilityFuture<'a, DbWriteResult>, type_name: &'a str, query: DbQuery);
        unavailable_async!(count, DbCapabilityFuture<'a, u64>, type_name: &'a str, query: DbQuery);
        unavailable_async!(exists_by_key, DbCapabilityFuture<'a, bool>, type_name: &'a str, key: DbKey);
        unavailable_async!(exists_by_query, DbCapabilityFuture<'a, bool>, type_name: &'a str, query: DbQuery);
        unavailable_async!(claim_lease, DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>>, type_name: &'a str, key: DbKey, slot: &'a str);
        unavailable_async!(renew_lease, DbCapabilityFuture<'a, bool>, hold: &'a DbCapabilityLeaseHold);
        unavailable_async!(release_lease, DbCapabilityFuture<'a, ()>, hold: &'a DbCapabilityLeaseHold);
        unavailable_async!(read_lease, DbCapabilityFuture<'a, Option<Value>>, type_name: &'a str, key: DbKey, slot: &'a str);
        unavailable_async!(insert_skiff_file_record, DbCapabilityFuture<'a, ()>, record: FileCapabilityRecord);
        unavailable_async!(find_skiff_file_by_id, DbCapabilityFuture<'a, Option<FileCapabilityRecord>>, id: &'a str);
        unavailable_async!(delete_skiff_file_by_id, DbCapabilityFuture<'a, u64>, id: &'a str);

        fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
            Box::pin(async { false })
        }
    }

    #[test]
    fn db_child_composition_defaults_to_fail_closed_on_missing_target_facts() {
        let composition = BytecodeDbChildComposition::default();
        assert!(!composition.is_available());
        let mut heap = RequestHeap::default();
        let error =
            match composition.prepared_read_one_by_key(&mut heap, DbKey::new(json!("one")), None) {
                Ok(_) => panic!("missing exact target must fail before provider"),
                Err(error) => error,
            };
        assert_eq!(error, BytecodeDbChildError::MissingExactTarget);
        assert!(
            db_child_required_fact().contains("DbObjectTargetId"),
            "missing F6 exact target requirement: {}",
            db_child_required_fact()
        );
    }

    #[tokio::test]
    async fn db_child_read_uses_exact_target_key_and_prepared_runtime() {
        let store = RecordingDbStore::default();
        let composition = test_composition(store.clone());
        let target = test_target();
        let mut heap = RequestHeap::default();

        let operation = composition
            .prepared_read_one_by_key(&mut heap, DbKey::new(json!("one")), None)
            .expect("read prepare through exact target");
        let found = operation
            .into_wait()
            .await
            .expect("read wait")
            .finalize(&mut heap)
            .expect("read finalize");
        assert!(found.is_some());
        assert!(store.target_was(target.lookup_key()));
    }

    #[tokio::test]
    async fn db_child_write_uses_exact_target_key_and_prepared_runtime() {
        let store = RecordingDbStore::default();
        let composition = test_composition(store.clone());
        let target = test_target();
        let mut heap = RequestHeap::default();
        let value = RuntimeValue::String("write".to_string());

        let operation = composition
            .prepared_create(&mut heap, &value)
            .expect("create prepare through exact target");
        let created = operation
            .into_wait()
            .await
            .expect("create wait")
            .finalize(&mut heap)
            .expect("create finalize");
        assert!(matches!(created, RuntimeValue::Heap(_)));
        assert!(store.target_was(target.lookup_key()));
    }

    #[tokio::test]
    async fn db_transaction_token_lifecycle_rejects_nested_and_commit_without_active() {
        let store = RecordingDbStore::default();
        let composition = test_composition(store.clone());
        let ledger = RequestMemoryLedger::new(1024);
        let mut session = composition
            .begin_transaction(&ledger)
            .await
            .expect("first transaction begins");

        assert!(ledger.transaction_ledger().has_active());
        let nested = match composition.begin_transaction(&ledger).await {
            Ok(_) => panic!("nested or reentrant begin must fail closed"),
            Err(error) => error,
        };
        assert_eq!(
            nested,
            BytecodeDbChildError::TransactionLedger(
                crate::memory_ledger::TransactionLedgerError::AlreadyActive
            )
        );

        session.commit().await.expect("commit active transaction");
        assert!(!ledger.transaction_ledger().has_active());
        let without_active = session
            .commit()
            .await
            .expect_err("commit without active transaction must fail closed");
        assert_eq!(
            without_active,
            BytecodeDbChildError::CommitWithoutActiveTransaction
        );
        assert_eq!(store.begins.load(Ordering::SeqCst), 1);
        assert_eq!(store.commits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn db_transaction_commit_and_abort_run_cleanup_exactly_once() {
        let store = RecordingDbStore::default();
        let composition = test_composition(store.clone());
        let ledger = RequestMemoryLedger::new(1024);
        let commit_cleanups = Arc::new(AtomicUsize::new(0));
        let mut commit_session = composition
            .begin_transaction_with_cleanup(&ledger, {
                let commit_cleanups = Arc::clone(&commit_cleanups);
                move || {
                    commit_cleanups.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await
            .expect("commit transaction begins");
        commit_session
            .commit()
            .await
            .expect("commit transaction succeeds");
        assert_eq!(commit_cleanups.load(Ordering::SeqCst), 1);

        let abort_cleanups = Arc::new(AtomicUsize::new(0));
        let mut abort_session = composition
            .begin_transaction_with_cleanup(&ledger, {
                let abort_cleanups = Arc::clone(&abort_cleanups);
                move || {
                    abort_cleanups.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await
            .expect("abort transaction begins");
        abort_session.abort().await;
        assert_eq!(abort_cleanups.load(Ordering::SeqCst), 1);
        assert!(!ledger.transaction_ledger().has_active());
        assert_eq!(store.begins.load(Ordering::SeqCst), 2);
        assert_eq!(store.aborts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn db_pending_cleanup_follows_child_heap_carrier_owner_graph() {
        let store = RecordingDbStore::default();
        let composition = test_composition(store.clone());
        let ledger = RequestMemoryLedger::new(1024);
        let cleanups = Arc::new(AtomicUsize::new(0));
        let session = composition
            .begin_transaction_with_cleanup(&ledger, {
                let cleanups = Arc::clone(&cleanups);
                move || {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await
            .expect("transaction begins");
        let mut carrier = child_heap_carrier();

        session
            .attach_pending_cleanup(&mut carrier)
            .expect("pending cleanup attaches to child heap carrier");
        assert!(ledger.transaction_ledger().has_active());
        let cleanup = carrier
            .take_pending_cleanup()
            .expect("cleanup is owned by the child heap carrier");
        assert!(ledger.transaction_ledger().has_active());
        cleanup();
        assert!(!ledger.transaction_ledger().has_active());
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);

        // Re-attach through a fresh transaction so the Drop path is exercised.
        let session = composition
            .begin_transaction_with_cleanup(&ledger, {
                let cleanups = Arc::clone(&cleanups);
                move || {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await
            .expect("second transaction begins");
        session
            .attach_pending_cleanup(&mut carrier)
            .expect("second pending cleanup attaches");
        drop(carrier);

        assert!(!ledger.transaction_ledger().has_active());
        assert_eq!(cleanups.load(Ordering::SeqCst), 2);
        assert_eq!(store.begins.load(Ordering::SeqCst), 2);
    }

    fn child_heap_carrier() -> ChildHeapCarrier {
        let ledger = RequestMemoryLedger::new(1024);
        let (domain, epoch, memory_lease) =
            ledger.mint_child_heap(1).expect("mint child heap lease");
        let context = RequestExecutionContext::<VmFiber>::create(
            BytecodeSchedulerPorts::<VmFiber>::default(),
        );
        let owner_lease = context
            .child_heap_registration()
            .mint_lease()
            .expect("mint child heap owner");
        let heap = RequestVmHeap::with_domain(
            u8::try_from(domain.get()).expect("test domain fits u8"),
            epoch.get(),
            RequestHeapLimits::default(),
        );
        ChildHeapCarrier::new(Box::new(heap), domain, epoch, memory_lease, owner_lease)
    }
}
