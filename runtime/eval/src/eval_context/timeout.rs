use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_linked_program::ExprRefIr;

use super::*;

impl EvalContext<'_> {
    pub(super) async fn exec_timeout_statement(
        &mut self,
        _duration_ms: u64,
        _body: &str,
        _site: &InstructionSourceSite,
    ) -> Result<Flow> {
        Err(pending_timeout_error("statement timeout"))
    }

    pub(super) async fn eval_timeout_expression(
        &mut self,
        _duration_ms: u64,
        _value: ExprRefIr,
        _site: &InstructionSourceSite,
    ) -> Result<RuntimeValueCarrier> {
        Err(pending_timeout_error("expression timeout"))
    }
}

fn pending_timeout_error(kind: &str) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "F445H-E4 evaluator integration is required for {kind}"
    ))
}
