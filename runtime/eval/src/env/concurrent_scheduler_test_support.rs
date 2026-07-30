use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use super::{
    concurrent_scheduler::{
        ConcurrentLaneExecutor, ConcurrentLaneFuture, ConcurrentOuterExecution,
    },
    slot_store::{RuntimeSlotBinding, RuntimeSlotLayout},
    ConcurrentPlan, ConcurrentPlanKind, Env, LaneCompletion, LaneEvaluation, LaneExecutionState,
    ProjectedLane,
};
use crate::{
    error::{Result, RuntimeError},
    program_execution::ExecutionCheckpointKind,
};
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    CancellationSource, CancellationToken, ExecutionControl, ExecutionControlApi,
    ExecutionControlError, ExecutionControlResult, ExecutionScope, ExecutionScopeAccessError,
    ExecutionScopeLifecycleSnapshot, ExecutionScopeTerminal, FileSourceStreamContext,
    OwnedExecutionControl, OwnedExecutionControlApi, StreamRuntime,
};

pub(super) type StartLane =
    dyn Fn(ProjectedLane, LaneExecutionState) -> ConcurrentLaneFuture<'static> + Send + Sync;

pub(super) struct TestExecutor {
    start: Arc<StartLane>,
}

impl TestExecutor {
    pub(super) fn new(
        start: impl Fn(ProjectedLane, LaneExecutionState) -> ConcurrentLaneFuture<'static>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            start: Arc::new(start),
        }
    }
}

impl<'a> ConcurrentLaneExecutor<'a> for TestExecutor {
    fn start_lane(
        &'a self,
        lane: ProjectedLane,
        state: LaneExecutionState,
    ) -> ConcurrentLaneFuture<'a> {
        (self.start)(lane, state)
    }
}

pub(super) fn boxed_lane(
    future: impl Future<Output = LaneCompletion> + Send + 'static,
) -> ConcurrentLaneFuture<'static> {
    Box::pin(future)
}

pub(super) struct TestOuter {
    control: OwnedExecutionControl,
    instruction_units: Arc<AtomicU64>,
    pub(super) scope: ExecutionScope,
    pub(super) cancellation: CancellationSource,
    pub(super) checkpoints: Mutex<Vec<ExecutionCheckpointKind>>,
}

impl TestOuter {
    pub(super) fn new() -> Self {
        Self::with_request_deadline(None)
    }

    pub(super) fn with_deadline(deadline: Instant) -> Self {
        Self::with_request_deadline(Some(deadline))
    }

    fn with_request_deadline(deadline: Option<Instant>) -> Self {
        let cancellation = CancellationSource::new();
        let scope = ExecutionScope::request(cancellation.token(), deadline);
        let instruction_units = Arc::new(AtomicU64::new(0));
        let control = OwnedExecutionControl::new(TestExecutionControl::new(
            scope.clone(),
            cancellation.token(),
            instruction_units.clone(),
        ));
        Self {
            control,
            instruction_units,
            scope,
            cancellation,
            checkpoints: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn instruction_units(&self) -> u64 {
        self.instruction_units.load(Ordering::Acquire)
    }

    pub(super) fn parent_cancel_flag_address(&self) -> usize {
        Arc::as_ptr(&self.control.borrow().cancel_flag()) as usize
    }
}

impl ConcurrentOuterExecution for TestOuter {
    fn owned_execution_control(&self) -> OwnedExecutionControl {
        self.control.clone()
    }

    fn concurrent_checkpoint(&self, kind: ExecutionCheckpointKind) -> Result<()> {
        self.checkpoints.lock().unwrap().push(kind);
        match self.scope.terminal_at(Instant::now()) {
            Some(ExecutionScopeTerminal::AncestorCancelled) => Err(RuntimeError::Cancelled),
            Some(terminal) => Err(RuntimeError::InvalidArtifact(format!(
                "unexpected test scope deadline: {terminal:?}"
            ))),
            None => Ok(()),
        }
    }
}

#[derive(Clone)]
struct TestExecutionControl {
    scope: ExecutionScope,
    cancellation: CancellationToken,
    cancelled: Arc<AtomicBool>,
    instruction_units: Arc<AtomicU64>,
}

impl TestExecutionControl {
    fn new(
        scope: ExecutionScope,
        cancellation: CancellationToken,
        instruction_units: Arc<AtomicU64>,
    ) -> Self {
        Self {
            scope,
            cancelled: cancellation.cancel_flag(),
            cancellation,
            instruction_units,
        }
    }
}

impl ExecutionControlApi for TestExecutionControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        source: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        let scope = self.scope.derive(local_deadline, source)?;
        Ok(OwnedExecutionControl::new(Self {
            scope,
            cancellation: self.cancellation.clone(),
            cancelled: self.cancelled.clone(),
            instruction_units: self.instruction_units.clone(),
        }))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.scope.is_ancestor_cancelled() {
            Err(ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.instruction_units.fetch_add(units, Ordering::Relaxed);
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        _stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        panic!("scheduler tests do not construct stream contexts")
    }
}

impl OwnedExecutionControlApi for TestExecutionControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        ExecutionControlApi::execution_scope(self)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        source: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, source)
    }
}

pub(super) fn statement_plan(lanes: Vec<ProjectedLane>) -> ConcurrentPlan {
    ConcurrentPlan::for_test(ConcurrentPlanKind::Statement, lanes)
}

pub(super) fn statement_lane(
    source_order: usize,
    dependencies: Vec<usize>,
    export_slot: Option<usize>,
) -> ProjectedLane {
    ProjectedLane::for_test(
        source_order,
        dependencies,
        LaneEvaluation::Statement {
            body: format!("lane-{source_order}"),
        },
        export_slot,
    )
}

pub(super) fn tail_lane(source_order: usize, dependencies: Vec<usize>) -> ProjectedLane {
    ProjectedLane::for_test(
        source_order,
        dependencies,
        LaneEvaluation::Tail {
            expression: skiff_runtime_linked_program::ExprRefIr { expression: 0 },
        },
        None,
    )
}

pub(super) fn env_with_slots(count: usize) -> Env {
    Env::with_slot_layout(&RuntimeSlotLayout {
        count,
        bindings: (0..count)
            .map(|slot| RuntimeSlotBinding {
                slot,
                name: format!("slot-{slot}"),
                kind: "local".to_string(),
                scope: None,
            })
            .collect(),
        self_slot: None,
        parameter_slots: HashMap::new(),
    })
}

pub(super) fn assert_clean_scope(outer: &TestOuter) {
    assert_eq!(
        outer.scope.lifecycle_snapshot(),
        ExecutionScopeLifecycleSnapshot::default()
    );
}

pub(super) fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}
