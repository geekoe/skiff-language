use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use skiff_runtime_capability_context::{
    CancellationToken, ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControlError,
    ExecutionControlResult, RequestAbortSignal,
};

use crate::execution_budget::ExecutionBudget;

#[derive(Clone)]
pub struct ExecutionControl<'a> {
    cancellation: CancellationToken,
    execution_budget: &'a Arc<ExecutionBudget>,
}

impl<'a> ExecutionControl<'a> {
    pub fn new(
        cancellation: CancellationToken,
        execution_budget: &'a Arc<ExecutionBudget>,
    ) -> Self {
        Self {
            cancellation,
            execution_budget,
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
        }
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancellation.cancel_flag()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
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
    cancellation: CancellationToken,
    cancel_flag: Arc<AtomicBool>,
    execution_budget: Arc<ExecutionBudget>,
}

impl OwnedExecutionControl {
    pub fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.cancellation.clone(), &self.execution_budget)
    }

    pub fn cancelled(&self) -> &AtomicBool {
        self.cancel_flag.as_ref()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
