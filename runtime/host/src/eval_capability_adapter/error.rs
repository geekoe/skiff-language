use super::*;
use skiff_runtime_eval::error::RuntimeErrorPayload;

pub(super) trait IntoEvalResult<T> {
    fn into_eval_result(self) -> Result<T>;
}

impl<T> IntoEvalResult<T> for root_error::Result<T> {
    fn into_eval_result(self) -> Result<T> {
        self.map_err(root_error_into_eval)
    }
}

pub(crate) fn root_error_into_eval(error: root_error::RuntimeError) -> RuntimeError {
    match error {
        root_error::RuntimeError::ExternalErrorPayload {
            code,
            message,
            status,
            details,
        } => RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
            code,
            message,
            status,
            details,
        }),
        root_error::RuntimeError::Cancelled => RuntimeError::Cancelled,
        root_error::RuntimeError::ExecutionBudgetExceeded {
            reason,
            instruction_count,
            limit,
            elapsed_ms,
        } => RuntimeError::ExecutionBudgetExceeded {
            reason: match reason {
                skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled => {
                    skiff_runtime_eval::error::BudgetReason::Cancelled
                }
                skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded => {
                    skiff_runtime_eval::error::BudgetReason::DeadlineExceeded
                }
                skiff_runtime_capability_context::ExecutionBudgetReason::InstructionLimitExceeded => {
                    skiff_runtime_eval::error::BudgetReason::InstructionLimitExceeded
                }
            },
            instruction_count,
            limit,
            elapsed_ms,
        },
        root_error::RuntimeError::Diagnosed(diagnosed) => root_diagnosed_into_eval(diagnosed),
        root_error::RuntimeError::Opaque(error) => RuntimeError::from_wire_payload(error),
        error => RuntimeError::from_wire_payload(Box::new(
            root_error::OrdinaryRuntimeError::try_new(error)
                .expect("Host cancellation was split before eval trait erasure"),
        )),
    }
}

pub(super) fn ordinary_root_error_into_capability(
    error: root_error::RuntimeError,
) -> capability_contract::CapabilityError {
    if let root_error::RuntimeError::TaskSubmitRejected { code, message } = error {
        return capability_contract::CapabilityError::task_submit_rejected(code, message);
    }
    if let root_error::RuntimeError::TaskControlRejected { code, message } = error {
        return capability_contract::CapabilityError::task_control_rejected(code, message);
    }
    capability_contract::CapabilityError::opaque(
        root_error::OrdinaryRuntimeError::try_new(error)
            .expect("synchronous capability adapter cannot carry cancellation"),
    )
}

pub(super) async fn root_result_into_capability<T>(
    result: root_error::Result<T>,
) -> capability_contract::CapabilityResult<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) if error.is_cancellation_terminal() => {
            // Root execution owns the terminal and polls its cancellation lane
            // before the operation lane. Keeping this erased capability future
            // pending prevents the ordinary-only CapabilityError channel from
            // materializing cancellation; the root owner immediately drops it.
            std::future::pending().await
        }
        Err(error) => Err(ordinary_root_error_into_capability(error)),
    }
}

fn root_diagnosed_into_eval(diagnosed: root_error::Diagnosed) -> RuntimeError {
    match diagnosed.try_into_runtime_parts() {
        Ok((inner, frames)) => {
            let mut error = root_error_into_eval(inner);
            for frame in frames.into_iter().rev() {
                error = match frame {
                    root_error::DiagnosticFrame::Source { source_id, frame } => {
                        error.with_source(source_id, *frame)
                    }
                    root_error::DiagnosticFrame::Diagnostic { frame } => {
                        error.with_diagnostic_frame(*frame)
                    }
                };
            }
            error
        }
        Err(diagnosed) => RuntimeError::from_wire_payload(Box::new(
            root_error::OrdinaryRuntimeError::try_new(root_error::RuntimeError::Diagnosed(
                diagnosed,
            ))
            .expect("wire-backed diagnosis is ordinary"),
        )),
    }
}

#[cfg(test)]
mod tests;
