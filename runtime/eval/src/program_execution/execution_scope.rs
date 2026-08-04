use std::{
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_capability_context::{
    ExecutionBudgetReason, ExecutionControl, ExecutionControlApi, ExecutionControlError,
    ExecutionScope, ExecutionScopeAccessError, FileSourceStreamContext, OwnedExecutionControl,
    StreamRuntime,
};

use crate::error::{Result, RuntimeError, ScopeTerminalCarrier};

use super::ProgramExecutionContext;

#[derive(Clone)]
pub(super) struct ExecutionClock {
    inner: Arc<dyn EvalMonotonicClock>,
}

impl ExecutionClock {
    pub(super) fn production() -> Self {
        Self::new(ProductionMonotonicClock)
    }

    pub(super) fn new(clock: impl EvalMonotonicClock + 'static) -> Self {
        Self {
            inner: Arc::new(clock),
        }
    }

    fn now(&self) -> Instant {
        self.inner.now()
    }
}

pub(super) trait EvalMonotonicClock: Send + Sync {
    fn now(&self) -> Instant;
}

struct ProductionMonotonicClock;

impl EvalMonotonicClock for ProductionMonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExecutionCheckpointKind {
    FunctionEntry,
    LoopCondition,
    LoopBackedge,
    LaneStart,
    LaneEnd,
    TailStart,
    GeneratedChunk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionCheckpoint {
    kind: ExecutionCheckpointKind,
    units: u64,
}

impl ExecutionCheckpoint {
    pub(crate) fn new(kind: ExecutionCheckpointKind, units: u64) -> Self {
        Self { kind, units }
    }

    pub(crate) fn kind(self) -> ExecutionCheckpointKind {
        self.kind
    }

    pub(crate) fn units(self) -> u64 {
        self.units
    }
}

impl ProgramExecutionContext<'_> {
    pub(crate) fn with_execution_control(mut self, execution: OwnedExecutionControl) -> Self {
        let execution_control = borrow_owned_execution_control(&execution);
        self.execution = execution;
        self.execution_control = execution_control;
        self
    }

    pub(crate) fn execution_scope(&self) -> Result<ExecutionScope> {
        self.execution.execution_scope().map_err(scope_access_error)
    }

    pub(crate) fn derive_timeout_child(
        &self,
        duration_ms: u64,
        site: InstructionSourceSite,
    ) -> Result<Self> {
        let deadline = deadline_after_duration_ms(self.execution_clock.now(), duration_ms);
        let execution = self
            .execution
            .derive_scope(deadline, site)
            .map_err(scope_access_error)?;
        Ok(self.clone().with_execution_control(execution))
    }

    pub(crate) fn checkpoint(&self, checkpoint: ExecutionCheckpoint) -> Result<()> {
        self.execution()
            .add_instruction_units(checkpoint.units())
            .map_err(|error| recover_execution_control_error(self, error))
    }

    /// Full deadline/cancel/instruction-limit check for key safety points.
    ///
    /// Per-node checkpoints only count instruction units and defer this check
    /// to interval crossings. Callers that sit on a control-flow boundary
    /// (async waits, derived-scope exits, provider starts) must use this when
    /// the bounded overshoot of a cheap checkpoint is not acceptable.
    pub(crate) fn poll_execution_scope(&self) -> Result<()> {
        let now = self.execution_clock.now();
        let execution = self.execution();
        let scope = execution.execution_scope().map_err(scope_access_error)?;
        if let Some(terminal) = scope.terminal_at(now) {
            return Err(ScopeTerminalCarrier::runtime_error(terminal));
        }
        execution
            .poll_execution_budget()
            .map_err(|error| recover_execution_control_error(self, error))
    }

    #[cfg(test)]
    pub(super) fn with_execution_clock(mut self, clock: ExecutionClock) -> Self {
        self.execution_clock = clock;
        self
    }
}

fn recover_execution_control_error(
    context: &ProgramExecutionContext<'_>,
    error: ExecutionControlError,
) -> RuntimeError {
    match error {
        ExecutionControlError::Cancelled => RuntimeError::Cancelled,
        ExecutionControlError::BudgetExceeded(failure)
            if failure.reason == ExecutionBudgetReason::DeadlineExceeded =>
        {
            let execution = context.execution();
            let scope = match execution.execution_scope() {
                Ok(scope) => scope,
                Err(error) => return scope_access_error(error),
            };
            match scope.terminal_at(context.execution_clock.now()) {
                Some(terminal) => ScopeTerminalCarrier::runtime_error(terminal),
                None => RuntimeError::InvalidArtifact(
                    "execution control reported a deadline without a current scope terminal"
                        .to_string(),
                ),
            }
        }
        error => RuntimeError::from(error),
    }
}

fn scope_access_error(error: ExecutionScopeAccessError) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!("current execution scope is unavailable: {error}"))
}

pub(super) fn deadline_after_duration_ms(now: Instant, duration_ms: u64) -> Instant {
    let requested = Duration::from_millis(duration_ms);
    if let Some(deadline) = now.checked_add(requested) {
        return deadline;
    }

    let mut low = 0_u64;
    let mut high = duration_ms;
    while low < high {
        let midpoint = low + (high - low).div_ceil(2);
        if now.checked_add(Duration::from_millis(midpoint)).is_some() {
            low = midpoint;
        } else {
            high = midpoint - 1;
        }
    }
    now.checked_add(Duration::from_millis(low))
        .expect("zero-duration monotonic deadline is always representable")
}

#[derive(Clone)]
struct OwnedExecutionControlBridge {
    execution: OwnedExecutionControl,
}

pub(super) fn borrow_owned_execution_control(
    execution: &OwnedExecutionControl,
) -> ExecutionControl<'static> {
    ExecutionControl::new(OwnedExecutionControlBridge {
        execution: execution.clone(),
    })
}

impl ExecutionControlApi for OwnedExecutionControlBridge {
    fn owned(&self) -> OwnedExecutionControl {
        self.execution.clone()
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.execution.borrow().cancel_flag()
    }

    fn cancellation_token(&self) -> skiff_runtime_capability_context::CancellationToken {
        self.execution.cancellation_token()
    }

    fn deadline(&self) -> Option<Instant> {
        self.execution.deadline()
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        self.execution.execution_scope()
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        self.execution.derive_scope(local_deadline, site)
    }

    fn check_cancelled(&self) -> skiff_runtime_capability_context::ExecutionControlResult<()> {
        self.execution.borrow().check_cancelled()
    }

    fn add_instruction_units(
        &self,
        units: u64,
    ) -> skiff_runtime_capability_context::ExecutionControlResult<()> {
        self.execution.borrow().add_instruction_units(units)
    }

    fn poll_execution_budget(
        &self,
    ) -> skiff_runtime_capability_context::ExecutionControlResult<()> {
        self.execution.borrow().poll_execution_budget()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        self.execution
            .borrow()
            .file_source_stream_context(stream_runtime)
    }
}
