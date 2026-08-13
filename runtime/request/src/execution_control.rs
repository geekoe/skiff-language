use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, EffectiveDeadline, ExecutionBudgetFailure,
    ExecutionBudgetReason, ExecutionControlError, ExecutionControlResult, ExecutionScope,
    ExecutionScopeDeriveError, ExecutionScopeTerminal, RequestAbortSignal,
};

use crate::execution_budget::{ExecutionBudget, ExecutionWinner};

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
            Err(ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.poll_execution_budget_at(Instant::now())
    }

    pub fn poll_execution_budget_at(&self, now: Instant) -> ExecutionControlResult<()> {
        if let Some(terminal) = self.scope_terminal_at(now) {
            return match terminal {
                ExecutionScopeTerminal::AncestorCancelled => Err(ExecutionControlError::Cancelled),
                ExecutionScopeTerminal::LocalDeadlineExceeded(deadline)
                | ExecutionScopeTerminal::InheritedDeadlineExceeded(deadline) => {
                    let _ = deadline;
                    Err(self.deadline_failure())
                }
            };
        }

        self.execution_budget
            .settlement()
            .map_or(Ok(()), |settlement| self.map_winner(settlement.winner()))
    }

    fn map_winner(&self, winner: ExecutionWinner) -> ExecutionControlResult<()> {
        match winner {
            ExecutionWinner::Succeeded | ExecutionWinner::Failed => Ok(()),
            ExecutionWinner::Cancelled | ExecutionWinner::InternalStop => {
                Err(ExecutionControlError::Cancelled)
            }
            ExecutionWinner::DeadlineExceeded | ExecutionWinner::InstructionLimitExceeded => {
                let stats = self.execution_budget.stats_snapshot();
                let reason = if winner == ExecutionWinner::DeadlineExceeded {
                    ExecutionBudgetReason::DeadlineExceeded
                } else {
                    ExecutionBudgetReason::InstructionLimitExceeded
                };
                Err(ExecutionControlError::BudgetExceeded(
                    ExecutionBudgetFailure {
                        reason,
                        instruction_count: stats.instruction_count,
                        limit: stats.budget_limit,
                        elapsed_ms: stats.elapsed_ms,
                    },
                ))
            }
            ExecutionWinner::AccountingFailure => Err(ExecutionControlError::BudgetExceeded(
                ExecutionBudgetFailure {
                    reason: ExecutionBudgetReason::InstructionLimitExceeded,
                    instruction_count: self.execution_budget.stats_snapshot().instruction_count,
                    limit: self.execution_budget.stats_snapshot().budget_limit,
                    elapsed_ms: self.execution_budget.stats_snapshot().elapsed_ms,
                },
            )),
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
