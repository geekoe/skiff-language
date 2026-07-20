mod async_stream_cancel;
mod boundary_materialization;
mod callback_native;
mod ordinary;
mod projection;

use skiff_artifact_model::{BoundaryCancellationContract, BoundaryStreamContract};
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, CallIr, LinkedPackageDirectCall,
};
use skiff_runtime_model::runtime_value::{CallbackCapabilityCarrier, RuntimeValue};

use crate::{
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    RuntimeAssemblyEvalSeamError,
};

#[allow(unused_imports)]
pub(crate) use callback_native::CallbackNativeCapabilityHooks;
pub(crate) use projection::{RuntimeAssemblyExecutionProjection, RuntimeExecutionProjection};

pub(crate) async fn dispatch_package_direct(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: &LinkedPackageDirectCall,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    context
        .context
        .runtime_assembly_target()?
        .ensure_package_direct_target(target)?;
    ordinary::execute_package_direct(context, call, target, args).await
}

pub(crate) async fn dispatch_service_call(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    instruction: &ActivationRelativeServiceCall,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let target = context
        .context
        .runtime_assembly_target()?
        .resolve_service_call(instruction)?;
    match &target.descriptor().contract.stream {
        BoundaryStreamContract::Unsupported { reason } => Err(RuntimeError::Unsupported(format!(
            "canonical service operation {} has unsupported stream semantics: {reason:?}",
            target.descriptor().operation_id
        ))),
        BoundaryStreamContract::ServerStream { .. } => {
            async_stream_cancel::execute_service_call(context, call, target, args).await
        }
        BoundaryStreamContract::Unary => match target.descriptor().contract.cancellation {
            BoundaryCancellationContract::Unsupported { reason } => {
                Err(RuntimeError::Unsupported(format!(
                    "canonical service operation {} has unsupported cancellation semantics: {reason:?}",
                    target.descriptor().operation_id
                )))
            }
            BoundaryCancellationContract::Cooperative => {
                async_stream_cancel::execute_service_call(context, call, target, args).await
            }
            BoundaryCancellationContract::NotCancellable
                if target.descriptor().contract.may_suspend =>
            {
                async_stream_cancel::execute_service_call(context, call, target, args).await
            }
            BoundaryCancellationContract::NotCancellable => {
                ordinary::execute_service_call(context, call, target, args).await
            }
        },
    }
}

pub(crate) async fn dispatch_callback_capability(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    carrier: &CallbackCapabilityCarrier,
    method_abi_id: &str,
    slot: u32,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    context.context.runtime_assembly_target()?;
    callback_native::execute_interface_call(context, call, carrier, method_abi_id, slot, args).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssemblyExecutionLaneKind {
    OrdinaryError,
    AsyncStreamCancel,
    CallbackNative,
}

impl AssemblyExecutionLaneKind {
    const fn label(self) -> &'static str {
        match self {
            Self::OrdinaryError => "ordinary/error",
            Self::AsyncStreamCancel => "async/stream/cancel",
            Self::CallbackNative => "callback/native",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "runtime assembly execution hook {hook} in lane {lane} is not available at the shared kernel checkpoint"
)]
pub(crate) struct AssemblyExecutionHandoffError {
    lane: &'static str,
    hook: &'static str,
}

impl AssemblyExecutionHandoffError {
    pub(crate) fn unavailable(lane: AssemblyExecutionLaneKind) -> RuntimeError {
        Self::unavailable_at(lane, "lane")
    }

    pub(crate) fn unavailable_at(
        lane: AssemblyExecutionLaneKind,
        hook: &'static str,
    ) -> RuntimeError {
        RuntimeError::ProviderUnavailable {
            target: format!("in-process {} {hook}", lane.label()),
            reason: Self {
                lane: lane.label(),
                hook,
            }
            .to_string(),
        }
    }
}

impl From<RuntimeAssemblyEvalSeamError> for RuntimeError {
    fn from(error: RuntimeAssemblyEvalSeamError) -> Self {
        RuntimeError::InvalidArtifact(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembly_execution_handoff_lanes_are_distinct_and_fail_closed() {
        for (lane, expected) in [
            (AssemblyExecutionLaneKind::OrdinaryError, "ordinary/error"),
            (
                AssemblyExecutionLaneKind::AsyncStreamCancel,
                "async/stream/cancel",
            ),
            (AssemblyExecutionLaneKind::CallbackNative, "callback/native"),
        ] {
            let error = AssemblyExecutionHandoffError::unavailable(lane);
            assert!(matches!(error, RuntimeError::ProviderUnavailable { .. }));
            assert!(error.to_string().contains(expected));
        }
    }

    #[test]
    fn assembly_execution_handoff_missing_target_is_structured() {
        let error = RuntimeError::from(RuntimeAssemblyEvalSeamError::MissingExecutionTarget);
        assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
        assert!(error.to_string().contains("no runtime assembly target"));
    }
}
