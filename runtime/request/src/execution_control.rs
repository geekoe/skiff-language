use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, EffectiveDeadline, ExecutionBudgetFailure,
    ExecutionBudgetReason, ExecutionControlError, ExecutionControlResult, ExecutionDeadlineSource,
    ExecutionScope, ExecutionScopeDeriveError, ExecutionScopeTerminal, RequestAbortSignal,
};

use crate::execution_budget::ExecutionBudget;

#[derive(Clone)]
pub struct ExecutionControl<'a> {
    cancellation: CancellationToken,
    execution_budget: &'a Arc<ExecutionBudget>,
    scope: ExecutionScope,
}

impl<'a> ExecutionControl<'a> {
    pub fn new(
        cancellation: CancellationToken,
        execution_budget: &'a Arc<ExecutionBudget>,
    ) -> Self {
        let scope = ExecutionScope::request(cancellation.clone(), execution_budget.deadline());
        Self {
            cancellation,
            execution_budget,
            scope,
        }
    }

    fn from_scope(
        cancellation: CancellationToken,
        execution_budget: &'a Arc<ExecutionBudget>,
        scope: ExecutionScope,
    ) -> Self {
        Self {
            cancellation,
            execution_budget,
            scope,
        }
    }

    pub fn abort_signal(&self) -> RequestAbortSignal<'_> {
        RequestAbortSignal::from_token(self.cancellation.clone())
    }

    pub fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl {
            cancellation: self.cancellation.clone(),
            cancel_flag: self.cancellation.cancel_flag(),
            execution_budget: self.execution_budget.clone(),
            scope: self.scope.clone(),
        }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancellation.cancel_flag()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns only the current monotonic absolute instant.
    ///
    /// New scoped consumers should retain `effective_deadline` so source and
    /// nesting are not discarded.
    pub fn deadline(&self) -> Option<Instant> {
        self.effective_deadline().map(EffectiveDeadline::at)
    }

    pub fn effective_deadline(&self) -> Option<&EffectiveDeadline> {
        self.scope.effective_deadline()
    }

    pub fn scope_nesting(&self) -> u32 {
        self.scope.nesting()
    }

    pub fn execution_scope(&self) -> &ExecutionScope {
        &self.scope
    }

    pub fn cancellation_signals(&self) -> CancellationSignals<'static> {
        self.scope.cancellation_signals()
    }

    pub fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeDeriveError> {
        let scope = self.scope.derive(local_deadline, site)?;
        Ok(OwnedExecutionControl {
            cancellation: self.cancellation.clone(),
            cancel_flag: self.cancellation.cancel_flag(),
            execution_budget: self.execution_budget.clone(),
            scope,
        })
    }

    pub fn scope_terminal_at(&self, now: Instant) -> Option<ExecutionScopeTerminal> {
        self.scope.terminal_at(now)
    }

    pub fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.scope.is_ancestor_cancelled() {
            self.execution_budget.record_cancelled();
            Err(ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        if self.execution_budget.add_units(units) {
            self.poll_execution_budget()
        } else {
            Ok(())
        }
    }

    pub fn add_instruction_units_at(&self, units: u64, now: Instant) -> ExecutionControlResult<()> {
        if self.execution_budget.add_units(units) {
            self.poll_execution_budget_at(now)?;
        }
        Ok(())
    }

    pub fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.poll_execution_budget_at(Instant::now())
    }

    pub fn poll_execution_budget_at(&self, now: Instant) -> ExecutionControlResult<()> {
        if let Some(terminal) = self.scope_terminal_at(now) {
            return match terminal {
                ExecutionScopeTerminal::AncestorCancelled => {
                    self.execution_budget.record_cancelled();
                    Err(ExecutionControlError::Cancelled)
                }
                ExecutionScopeTerminal::LocalDeadlineExceeded(deadline)
                | ExecutionScopeTerminal::InheritedDeadlineExceeded(deadline) => {
                    if matches!(deadline.source(), ExecutionDeadlineSource::Request) {
                        self.map_budget_poll(self.execution_budget.poll(false, now))
                    } else {
                        self.execution_budget.record_scoped_poll();
                        Err(self.deadline_failure())
                    }
                }
            };
        }

        self.map_budget_poll(self.execution_budget.poll(false, now))
    }

    fn map_budget_poll(
        &self,
        result: Result<(), ExecutionBudgetReason>,
    ) -> ExecutionControlResult<()> {
        match result {
            Ok(()) => Ok(()),
            Err(ExecutionBudgetReason::Cancelled) => Err(ExecutionControlError::Cancelled),
            Err(reason) => {
                let stats = self.execution_budget.stats_snapshot();
                Err(ExecutionControlError::BudgetExceeded(
                    ExecutionBudgetFailure {
                        reason,
                        instruction_count: stats.instruction_count,
                        limit: stats.budget_limit,
                        elapsed_ms: stats.elapsed_ms,
                    },
                ))
            }
        }
    }

    fn deadline_failure(&self) -> ExecutionControlError {
        let stats = self.execution_budget.stats_snapshot();
        ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
            reason: ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: stats.instruction_count,
            limit: stats.budget_limit,
            elapsed_ms: stats.elapsed_ms,
        })
    }
}

#[derive(Clone)]
pub struct OwnedExecutionControl {
    cancellation: CancellationToken,
    cancel_flag: Arc<AtomicBool>,
    execution_budget: Arc<ExecutionBudget>,
    scope: ExecutionScope,
}

#[cfg(test)]
mod tests;

impl OwnedExecutionControl {
    pub fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::from_scope(
            self.cancellation.clone(),
            &self.execution_budget,
            self.scope.clone(),
        )
    }

    pub fn cancelled(&self) -> &AtomicBool {
        self.cancel_flag.as_ref()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.effective_deadline().map(EffectiveDeadline::at)
    }

    pub fn effective_deadline(&self) -> Option<&EffectiveDeadline> {
        self.scope.effective_deadline()
    }

    pub fn scope_nesting(&self) -> u32 {
        self.scope.nesting()
    }

    pub fn execution_scope(&self) -> &ExecutionScope {
        &self.scope
    }

    pub fn cancellation_signals(&self) -> CancellationSignals<'static> {
        self.scope.cancellation_signals()
    }

    pub fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<Self, ExecutionScopeDeriveError> {
        self.borrow().derive_scope(local_deadline, site)
    }

    pub fn scope_terminal_at(&self, now: Instant) -> Option<ExecutionScopeTerminal> {
        self.scope.terminal_at(now)
    }
}
