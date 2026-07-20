use skiff_runtime_linked_program::CallIr;
use skiff_runtime_model::runtime_value::RuntimeValue;

use super::{AssemblyExecutionHandoffError, AssemblyExecutionLaneKind};
use crate::{error::Result, eval_context::EvalContext, RuntimeAssemblyServiceCallTarget};

pub(crate) async fn execute_service_call(
    _context: &mut EvalContext<'_>,
    _call: &CallIr,
    _target: RuntimeAssemblyServiceCallTarget,
    _args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    Err(AssemblyExecutionHandoffError::unavailable(
        AssemblyExecutionLaneKind::AsyncStreamCancel,
    ))
}
