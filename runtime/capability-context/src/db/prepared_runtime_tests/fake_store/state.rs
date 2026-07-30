use std::{
    any::Any,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use skiff_runtime_boundary::recoverable::FailClosedRecoverableBehaviorHooks;
use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
};
use tokio::sync::oneshot;

use crate::db::{
    DbCapabilityLeaseHold, DbCapabilityLeaseHoldHandle, DbRecoverableRuntimeContext,
    DbRecoverableRuntimeExpectedPlans,
};

#[derive(Default)]
pub(crate) struct TestStoreState {
    legacy_runtime_calls: AtomicUsize,
    raw_calls: AtomicUsize,
    wait_starts: AtomicUsize,
    finalize_calls: AtomicUsize,
    create_finalize_fails: AtomicBool,
    replace_wait_fails: AtomicBool,
    create_gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl TestStoreState {
    pub(crate) fn new(gate: Option<oneshot::Receiver<()>>) -> Self {
        Self {
            create_gate: Mutex::new(gate),
            ..Self::default()
        }
    }

    pub(crate) fn record_legacy_runtime_call(&self) {
        self.legacy_runtime_calls.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn legacy_runtime_calls(&self) -> usize {
        self.legacy_runtime_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn record_raw_call(&self) {
        self.raw_calls.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn raw_calls(&self) -> usize {
        self.raw_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn record_wait_start(&self) {
        self.wait_starts.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn wait_starts(&self) -> usize {
        self.wait_starts.load(Ordering::SeqCst)
    }

    pub(crate) fn record_finalize(&self) {
        self.finalize_calls.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn finalize_calls(&self) -> usize {
        self.finalize_calls.load(Ordering::SeqCst)
    }

    pub(crate) fn set_create_finalize_fails(&self, fails: bool) {
        self.create_finalize_fails.store(fails, Ordering::SeqCst);
    }

    pub(crate) fn create_finalize_fails(&self) -> bool {
        self.create_finalize_fails.load(Ordering::SeqCst)
    }

    pub(crate) fn set_replace_wait_fails(&self, fails: bool) {
        self.replace_wait_fails.store(fails, Ordering::SeqCst);
    }

    pub(crate) fn replace_wait_fails(&self) -> bool {
        self.replace_wait_fails.load(Ordering::SeqCst)
    }

    pub(crate) fn take_create_gate(&self) -> Option<oneshot::Receiver<()>> {
        self.create_gate.lock().expect("create gate lock").take()
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

pub(super) fn test_hold() -> DbCapabilityLeaseHold {
    DbCapabilityLeaseHold::new(Arc::new(TestLeaseHold(7)))
}

pub(crate) fn runtime_context() -> DbRecoverableRuntimeContext {
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

pub(crate) async fn wait_until_started(state: &TestStoreState, expected: usize) {
    for _ in 0..32 {
        if state.wait_starts() == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(state.wait_starts(), expected);
}
