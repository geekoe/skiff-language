mod async_stream_cancel;
mod boundary_materialization;
mod callback_native;
mod ingress;
pub(crate) mod ordinary;
mod projection;
pub(crate) mod service_error_channel;
#[cfg(test)]
mod service_error_convergence;

use skiff_artifact_model::{
    BoundaryFeatureUnavailableReason, BoundaryStreamContract, ContractOperationId,
};
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, CallIr, LinkedPackageDirectCall,
};
use skiff_runtime_model::runtime_value::{
    CallbackCapabilityCarrier, RuntimeValue, RuntimeValueCarrier,
};

use crate::{
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    RuntimeAssemblyEvalSeamError, RuntimeAssemblyServiceCallTarget,
};
use service_error_channel::{CanonicalServiceErrorChannel, ServiceErrorImportContext};

pub(crate) use async_stream_cancel::is_canonical_boundary_stream_sink;
pub(crate) use callback_native::prepare_interface_call as prepare_callback_capability_call;
#[allow(unused_imports)]
pub(crate) use callback_native::CallbackNativeCapabilityHooks;
pub use ingress::{dispatch_ingress_via_in_process_boundary, InProcessBoundaryIngressResponse};
pub(crate) use projection::{RuntimeAssemblyExecutionProjection, RuntimeExecutionProjection};

pub(crate) async fn dispatch_package_direct(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: &LinkedPackageDirectCall,
    args: Vec<RuntimeValueCarrier>,
) -> Result<RuntimeValueCarrier> {
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
    dispatch_in_process_boundary(
        context,
        call,
        target,
        args,
        InProcessBoundaryDispatchOrigin::InternalServiceCall,
    )
    .await
}

/// The single resolved-target dispatcher for every canonical in-process service boundary.
///
/// Caller adaptation and target lookup happen before this symbol. All contract lane selection,
/// provider activation switching and detached materialization remain behind this one owner.
async fn dispatch_in_process_boundary(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
    origin: InProcessBoundaryDispatchOrigin,
) -> Result<RuntimeValue> {
    record_in_process_boundary_dispatch(origin, &target);
    let remote_service_id = target.contract().service_id.clone();
    let remote_operation_id = target.descriptor().operation_id.clone();
    let result = match &target.descriptor().contract.stream {
        BoundaryStreamContract::Unsupported { reason } => Err(unsupported_stream_error(
            &target.descriptor().operation_id,
            reason,
        )),
        BoundaryStreamContract::Unary | BoundaryStreamContract::ServerStream { .. } => {
            async_stream_cancel::execute_service_call(context, call, target, args).await
        }
    };

    let Err(RuntimeError::FixedServiceFailure(error)) = result else {
        return result;
    };
    match origin {
        InProcessBoundaryDispatchOrigin::Ingress => Err(RuntimeError::FixedServiceFailure(error)),
        InProcessBoundaryDispatchOrigin::InternalServiceCall => {
            let caller_stack_at_site = context.context.exception_stack_for_site(call.site.clone());
            let caller_target = context.context.runtime_assembly_target()?;
            let exception = CanonicalServiceErrorChannel::import_caller_failure(
                error,
                ServiceErrorImportContext {
                    execution_image: caller_target.execution_image().as_ref(),
                    type_view: caller_target.execution_projection().type_view(),
                    caller_heap: context.heap.heap_mut(),
                    caller_package_build_id: caller_target
                        .activation_context()
                        .implementation_package_build_id(),
                    caller_executable_addr: context.addr,
                    call_site: &call.site,
                    caller_stack_at_site: &caller_stack_at_site,
                    remote_service_id: remote_service_id.as_str(),
                    remote_operation_id: remote_operation_id.as_str(),
                },
            )?;
            record_in_process_boundary_failure_import(
                caller_target.request_activation().generation(),
                caller_target.activation_context().activation_id().as_str(),
                &exception,
            );
            Err(RuntimeError::UserException(exception))
        }
    }
}

fn unsupported_stream_error(
    operation_id: &ContractOperationId,
    reason: &BoundaryFeatureUnavailableReason,
) -> RuntimeError {
    RuntimeError::Unsupported(format!(
        "canonical service operation {operation_id} has unsupported stream semantics: {reason:?}"
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InProcessBoundaryDispatchOrigin {
    Ingress,
    InternalServiceCall,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InProcessBoundaryDispatchRecord {
    pub origin: &'static str,
    pub contract_operation: String,
}

#[cfg(any(test, feature = "test-support"))]
static IN_PROCESS_BOUNDARY_DISPATCH_SPY: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<u64, Vec<InProcessBoundaryDispatchRecord>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

fn record_in_process_boundary_dispatch(
    origin: InProcessBoundaryDispatchOrigin,
    target: &RuntimeAssemblyServiceCallTarget,
) {
    #[cfg(any(test, feature = "test-support"))]
    if let Ok(mut probes) = IN_PROCESS_BOUNDARY_DISPATCH_SPY.lock() {
        if let Some(records) = probes.get_mut(&target.provider_request().generation()) {
            records.push(InProcessBoundaryDispatchRecord {
                origin: match origin {
                    InProcessBoundaryDispatchOrigin::Ingress => "ingress",
                    InProcessBoundaryDispatchOrigin::InternalServiceCall => "internal",
                },
                contract_operation: target.descriptor().operation_id.as_str().to_string(),
            });
        }
    }
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = (origin, target);
}

#[cfg(any(test, feature = "test-support"))]
pub fn start_in_process_boundary_dispatch_probe_for_test(request_generation: u64) {
    if let Ok(mut probes) = IN_PROCESS_BOUNDARY_DISPATCH_SPY.lock() {
        probes.insert(request_generation, Vec::new());
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn take_in_process_boundary_dispatch_records_for_test(
    request_generation: u64,
) -> Vec<InProcessBoundaryDispatchRecord> {
    IN_PROCESS_BOUNDARY_DISPATCH_SPY
        .lock()
        .ok()
        .and_then(|mut probes| probes.remove(&request_generation))
        .unwrap_or_default()
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InProcessBoundaryFailureImportRecord {
    pub caller_activation_id: String,
    pub encoded_error: Vec<u8>,
    pub source: skiff_artifact_model::InstructionSourceSite,
    pub stack: Vec<skiff_runtime_model::service_error::ExceptionStackFrame>,
}

#[cfg(test)]
static IN_PROCESS_BOUNDARY_FAILURE_IMPORT_SPY: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<u64, Vec<InProcessBoundaryFailureImportRecord>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

fn record_in_process_boundary_failure_import(
    request_generation: u64,
    caller_activation_id: &str,
    exception: &crate::error::UserException,
) {
    #[cfg(test)]
    if let Ok(mut probes) = IN_PROCESS_BOUNDARY_FAILURE_IMPORT_SPY.lock() {
        if let Some(records) = probes.get_mut(&request_generation) {
            if let Some(error) = exception.request().fixed_service_error() {
                records.push(InProcessBoundaryFailureImportRecord {
                    caller_activation_id: caller_activation_id.to_string(),
                    encoded_error: error.encoded_bytes().to_vec(),
                    source: exception.request().source().clone(),
                    stack: exception.request().stack().to_vec(),
                });
            }
        }
    }
    #[cfg(not(test))]
    let _ = (request_generation, caller_activation_id, exception);
}

#[cfg(test)]
pub(crate) fn start_in_process_boundary_failure_import_probe_for_test(request_generation: u64) {
    if let Ok(mut probes) = IN_PROCESS_BOUNDARY_FAILURE_IMPORT_SPY.lock() {
        probes.insert(request_generation, Vec::new());
    }
}

#[cfg(test)]
pub(crate) fn take_in_process_boundary_failure_import_records_for_test(
    request_generation: u64,
) -> Vec<InProcessBoundaryFailureImportRecord> {
    IN_PROCESS_BOUNDARY_FAILURE_IMPORT_SPY
        .lock()
        .ok()
        .and_then(|mut probes| probes.remove(&request_generation))
        .unwrap_or_default()
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) use async_stream_cancel::provider_stream_tasks_active_for_test;

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
mod tests;
