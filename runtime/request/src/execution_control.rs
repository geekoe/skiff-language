use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use skiff_runtime_capability_context::{
    ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControlError, ExecutionControlResult,
    RequestAbortSignal,
};

use crate::execution_budget::ExecutionBudget;

#[derive(Clone, Copy)]
pub struct ExecutionControl<'a> {
    cancelled: &'a Arc<AtomicBool>,
    execution_budget: &'a Arc<ExecutionBudget>,
}

impl<'a> ExecutionControl<'a> {
    pub fn new(cancelled: &'a Arc<AtomicBool>, execution_budget: &'a Arc<ExecutionBudget>) -> Self {
        Self {
            cancelled,
            execution_budget,
        }
    }

    pub fn abort_signal(&self) -> RequestAbortSignal<'_> {
        RequestAbortSignal::from_borrowed_flag(self.cancelled.as_ref())
    }

    pub fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl {
            cancelled: self.cancelled.clone(),
            execution_budget: self.execution_budget.clone(),
        }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    pub fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.abort_signal().is_cancelled() {
            self.execution_budget.record_cancelled();
            Err(ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        if self.execution_budget.add_units(units) {
            self.poll_execution_budget()?;
        }
        Ok(())
    }

    pub fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        match self
            .execution_budget
            .poll(self.abort_signal().is_cancelled(), Instant::now())
        {
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
}

#[derive(Clone)]
pub struct OwnedExecutionControl {
    cancelled: Arc<AtomicBool>,
    execution_budget: Arc<ExecutionBudget>,
}

impl OwnedExecutionControl {
    pub fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(&self.cancelled, &self.execution_budget)
    }

    pub fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }
}
