use skiff_runtime_linked_program::LinkedConcurrentPlanIr;

use super::*;

impl EvalContext<'_> {
    pub(super) async fn exec_concurrent_statement(
        &mut self,
        _plan: &LinkedConcurrentPlanIr,
    ) -> Result<Flow> {
        Err(pending_concurrent_error("statement concurrent"))
    }

    pub(super) async fn eval_concurrent_value(
        &mut self,
        _plan: &LinkedConcurrentPlanIr,
    ) -> Result<RuntimeValueCarrier> {
        Err(pending_concurrent_error("expression concurrent"))
    }
}

fn pending_concurrent_error(kind: &str) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "F445H-E4 evaluator integration is required for {kind}"
    ))
}

#[cfg(test)]
mod tests;
