use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCallbackLifetime, BoundaryCancellationContract,
    BoundaryOperationDescriptor, BoundaryStreamContract, InstructionSourceSite, PackageBuildId,
    SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_program::{CallIr, LinkedPackageDirectCall};
use skiff_runtime_model::runtime_value::{RuntimeValue, RuntimeValueCarrier};

use super::{
    boundary_materialization::CanonicalServiceBoundaryPlan,
    callback_native::CallbackNativeCapabilityHooks,
    service_error_channel::{
        CanonicalServiceErrorChannel, RestrictedServiceDiagnosticExportContext,
        ServiceErrorExportContext,
    },
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
            let telemetry = provider_context.telemetry_context();
            let fallback_source = InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::RuntimeBoundaryDispatch,
            };
            let fallback_stack = provider_context.exception_stack_for_site(fallback_source.clone());
            let fixed = CanonicalServiceErrorChannel::export_provider_failure_with_diagnostic(
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
                RestrictedServiceDiagnosticExportContext {
                    telemetry: &telemetry,
                    provider_activation_id: target.provider_activation().activation_id().as_str(),
                    request_generation: provider_target.request_activation().generation(),
                    fallback_source: &fallback_source,
                    fallback_stack: &fallback_stack,
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
pub(crate) mod tests;
