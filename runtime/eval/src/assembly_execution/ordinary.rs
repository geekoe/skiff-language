use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCallbackLifetime, BoundaryCancellationContract,
    BoundaryOperationDescriptor, BoundaryStreamContract, PackageBuildId,
};
use skiff_runtime_linked_program::{CallIr, LinkedPackageDirectCall};
use skiff_runtime_model::runtime_value::{RuntimeValue, RuntimeValueCarrier};

use super::{
    boundary_materialization::CanonicalServiceBoundaryPlan,
    callback_native::CallbackNativeCapabilityHooks,
    service_error_channel::{CanonicalServiceErrorChannel, ServiceErrorExportContext},
};
use crate::{
    env::Env,
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    RuntimeAssemblyServiceCallTarget,
};

pub(crate) async fn execute_package_direct(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: &LinkedPackageDirectCall,
    args: Vec<RuntimeValueCarrier>,
) -> Result<RuntimeValueCarrier> {
    context
        .interpreter
        .call_program_executable_carriers(
            context
                .context
                .clone()
                .with_local_call_site(call.site.clone()),
            context.heap,
            context.env,
            context.addr,
            target.executable_addr(),
            &call.type_args,
            args,
        )
        .await
}

pub(crate) async fn execute_service_call(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
    caller_package_build_id: Option<&PackageBuildId>,
) -> Result<RuntimeValue> {
    validate_ordinary_operation(target.descriptor(), call)?;
    let boundary = CanonicalServiceBoundaryPlan::new(
        target.descriptor(),
        target.schema_records().as_ref(),
        args.len(),
    )?;
    let mut provider_heap = boundary.fresh_provider_heap(context.context.request_heap_limits());
    let caller_hooks = CallbackNativeCapabilityHooks::new(&context.context);
    let provider_args =
        boundary.materialize_parameters(&args, context.heap, &mut provider_heap, &caller_hooks)?;

    let provider_eval_target = context
        .context
        .runtime_assembly_target()?
        .with_request_activation(target.provider_request().clone())?;
    let provider_context = context
        .context
        .clone()
        .with_runtime_assembly_target(provider_eval_target)
        .with_provider_service_stack_scope();
    let provider_env = Env::new();
    let provider_type_args = Default::default();
    let provider_result = context
        .interpreter
        .call_program_executable(
            provider_context.clone(),
            &mut provider_heap,
            &provider_env,
            target.executable_addr(),
            target.executable_addr(),
            &provider_type_args,
            provider_args,
        )
        .await;
    match provider_result {
        Ok(value) => {
            let provider_hooks = CallbackNativeCapabilityHooks::new(&provider_context);
            boundary.materialize_success(&value, &provider_heap, context.heap, &provider_hooks)
        }
        Err(error) => {
            let provider_target = provider_context.runtime_assembly_target()?;
            record_ordinary_provider_failure(
                provider_target.request_activation().generation(),
                target.provider_activation().activation_id().as_str(),
                &error,
            );
            let fixed = CanonicalServiceErrorChannel::export_provider_failure(
                &error,
                ServiceErrorExportContext {
                    execution_image: provider_target.execution_image().as_ref(),
                    type_view: provider_target.execution_projection().type_view(),
                    provider_heap: &provider_heap,
                    provider_package_build_id: target
                        .provider_activation()
                        .implementation_package_build_id(),
                    caller_package_build_id,
                    provider_service_id: target.contract().service_id.as_str(),
                    operation_id: target.descriptor().operation_id.as_str(),
                },
                || provider_context.next_exception_correlation(),
            )?;
            Err(RuntimeError::FixedServiceFailure(fixed))
        }
    }
}

fn validate_ordinary_operation(
    operation: &BoundaryOperationDescriptor,
    call: &CallIr,
) -> Result<()> {
    if !call.type_args.is_empty() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} cannot carry package-local type arguments",
            operation.operation_id
        )));
    }
    if !matches!(operation.contract.stream, BoundaryStreamContract::Unary)
        || !matches!(
            operation.contract.cancellation,
            BoundaryCancellationContract::NotCancellable
        )
        || operation.contract.may_suspend
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} is not an ordinary unary operation",
            operation.operation_id
        )));
    }
    match &operation.contract.callbacks {
        BoundaryCallbackContract::None => {}
        BoundaryCallbackContract::RequestScoped { lifetime, .. }
            if *lifetime == BoundaryCallbackLifetime::TopLevelRequest => {}
        BoundaryCallbackContract::RequestScoped { .. } => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "canonical ordinary service operation {} cannot use stream-scoped callbacks",
                operation.operation_id
            )));
        }
        BoundaryCallbackContract::Unsupported { reason } => {
            return Err(RuntimeError::Unsupported(format!(
                "canonical service operation {} has unsupported callback semantics: {reason:?}",
                operation.operation_id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrdinaryProviderFailureRecord {
    pub activation_id: String,
    pub fixed_before_export: bool,
    pub source: skiff_artifact_model::InstructionSourceSite,
    pub stack: Vec<skiff_runtime_model::service_error::ExceptionStackFrame>,
}

#[cfg(test)]
static ORDINARY_PROVIDER_FAILURE_SPY: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<u64, Vec<OrdinaryProviderFailureRecord>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

fn record_ordinary_provider_failure(
    request_generation: u64,
    activation_id: &str,
    error: &RuntimeError,
) {
    #[cfg(test)]
    if let Ok(mut probes) = ORDINARY_PROVIDER_FAILURE_SPY.lock() {
        if let Some(records) = probes.get_mut(&request_generation) {
            if let Some(exception) = crate::exceptions::user_exception_for_catch(error) {
                records.push(OrdinaryProviderFailureRecord {
                    activation_id: activation_id.to_string(),
                    fixed_before_export: exception.request().fixed_service_error().is_some(),
                    source: exception.request().source().clone(),
                    stack: exception.request().stack().to_vec(),
                });
            }
        }
    }
    #[cfg(not(test))]
    let _ = (request_generation, activation_id, error);
}

#[cfg(test)]
pub(crate) fn start_ordinary_provider_failure_probe_for_test(request_generation: u64) {
    if let Ok(mut probes) = ORDINARY_PROVIDER_FAILURE_SPY.lock() {
        probes.insert(request_generation, Vec::new());
    }
}

#[cfg(test)]
pub(crate) fn take_ordinary_provider_failure_records_for_test(
    request_generation: u64,
) -> Vec<OrdinaryProviderFailureRecord> {
    ORDINARY_PROVIDER_FAILURE_SPY
        .lock()
        .ok()
        .and_then(|mut probes| probes.remove(&request_generation))
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests;
