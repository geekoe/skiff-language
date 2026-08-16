use skiff_artifact_model::{DbOperandRole, DbOperationKind, DbOperationReference};

use super::{PreparedNativeCall, RuntimeNativeInvocation};
use crate::capability::NativeDbCapability;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};

pub(super) struct DbNativeDispatch;

impl DbNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(target, "std.db.operation" | "db.operation")
    }

    pub(super) fn prepare(
        db_context: &impl NativeDbCapability,
        invocation: RuntimeNativeInvocation,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<PreparedNativeCall<'static>> {
        let binding_key = invocation.binding_key().to_string();
        let operation = invocation
            .require_plan()?
            .db_operation()
            .cloned()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "{} resolved db call is missing structured db operation",
                    invocation.target_name()
                ))
            })?;
        validate_operation(&binding_key, &operation, args.len())?;
        db_context.prepare_db_operation(&operation, args, heap)
    }
}

fn validate_operation(
    binding_key: &str,
    operation: &DbOperationReference,
    arg_count: usize,
) -> Result<()> {
    if operation.op != DbOperationKind::Write {
        return Err(RuntimeError::Unsupported(format!(
            "{binding_key} only supports normalized single write in this contract generation"
        )));
    }
    if operation.operand_roles != [DbOperandRole::ObjectFields] {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{binding_key} only supports a single ObjectFields operand in this contract generation"
        )));
    }
    if arg_count != 1 {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{binding_key} expects exactly one ObjectFields argument, got {arg_count}"
        )));
    }
    Ok(())
}
