use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_capability_context::{
    CancellationToken, ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControl,
    ExecutionControlApi, ExecutionControlError, ExecutionControlResult, ExecutionScope,
    ExecutionScopeAccessError, ExecutionScopeTerminal, FileSourceStreamContext,
    OwnedExecutionControl, OwnedExecutionControlApi, StreamRuntime,
};

#[derive(Clone)]
struct LaneExecutionControl {
    parent: OwnedExecutionControl,
    scope: ExecutionScope,
    lane_cancellation: CancellationToken,
    lane_cancel_flag: Arc<AtomicBool>,
}

pub(super) fn execution_control_for_lane(
    parent: OwnedExecutionControl,
    scope: ExecutionScope,
    lane_cancellation: CancellationToken,
) -> OwnedExecutionControl {
    let lane_cancel_flag = lane_cancellation.cancel_flag();
    OwnedExecutionControl::new(LaneExecutionControl {
        parent,
        scope,
        lane_cancellation,
        lane_cancel_flag,
    })
}

impl LaneExecutionControl {
    fn borrowed(&self) -> ExecutionControl<'static> {
        ExecutionControl::new(self.clone())
    }

    fn current_terminal(&self) -> Option<ExecutionControlError> {
        match self.scope.terminal_at(Instant::now()) {
            Some(ExecutionScopeTerminal::AncestorCancelled) => {
                Some(ExecutionControlError::Cancelled)
            }
            Some(
                ExecutionScopeTerminal::LocalDeadlineExceeded(_)
                | ExecutionScopeTerminal::InheritedDeadlineExceeded(_),
            ) => Some(ExecutionControlError::BudgetExceeded(
                ExecutionBudgetFailure {
                    reason: ExecutionBudgetReason::DeadlineExceeded,
                    instruction_count: 0,
                    limit: None,
                    elapsed_ms: 0.0,
                },
            )),
            None => None,
        }
    }

    fn check_current_terminal(&self) -> ExecutionControlResult<()> {
        match self.current_terminal() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl ExecutionControlApi for LaneExecutionControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.lane_cancel_flag.clone()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.lane_cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        let scope = self
            .scope
            .derive(local_deadline, site)
            .map_err(ExecutionScopeAccessError::from)?;
        Ok(execution_control_for_lane(
            self.parent.clone(),
            scope,
            self.lane_cancellation.clone(),
        ))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        self.check_current_terminal()?;
        self.parent.borrow().check_cancelled()
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.check_current_terminal()?;
        self.parent.borrow().add_instruction_units(units)
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_current_terminal()?;
        self.parent.borrow().poll_execution_budget()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        FileSourceStreamContext::new(stream_runtime, self.borrowed())
    }
}

impl OwnedExecutionControlApi for LaneExecutionControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        self.borrowed()
    }

    fn cancelled(&self) -> &AtomicBool {
        // The lane cancellation token is the narrow compatibility signal for
        // legacy consumers. Scope-aware consumers observe both it and all
        // parent cancellation signals through `execution_scope`.
        self.lane_cancel_flag.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.lane_cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}
