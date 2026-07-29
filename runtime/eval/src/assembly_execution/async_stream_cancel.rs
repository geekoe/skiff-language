#[cfg(test)]
use std::time::Instant;
use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use serde_json::Value;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef, InstructionSourceSite, PackageBuildId,
    SyntheticInstructionSiteReason,
};
use skiff_runtime_activation::RequestStreamLease;
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableContractPlan, ServiceLinkableMaterializationError,
    ServiceLinkableMaterializationScope,
};
use skiff_runtime_capability_context::{
    CancellationToken, ExecutionControl, OwnedExecutionControl, StreamCancelSignal,
    StreamInternalItem, StreamLifetimeGuard, StreamLifetimeGuardApi, StreamRuntimeError,
    StreamRuntimeResult,
};
use skiff_runtime_linked_program::{CallIr, ConstAddr, ExecutableAddr, LinkedTypeRef};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{ExceptionStackFrame, OpaqueServiceError},
    type_plan::RuntimeTypePlan,
};

use super::{
    boundary_materialization::CanonicalServiceBoundaryPlan,
    callback_native::CallbackNativeCapabilityHooks,
    service_error_channel::{
        CanonicalServiceErrorChannel, RestrictedServiceDiagnosticExportContext,
        ServiceErrorExportContext,
    },
    AssemblyExecutionHandoffError, AssemblyExecutionLaneKind, RuntimeExecutionProjection,
};
use crate::{
    capabilities::{StreamRuntimeOwner, StreamSink, StreamSinkApi},
    env::Env,
    error::{stream_runtime_error_from_eval, Result, RuntimeError},
    eval_context::EvalContext,
    program_execution::{
        ExecutionCheckpoint, ExecutionCheckpointKind, OwnedProgramExecutionContext,
        ProgramExecutionContext,
    },
    program_stream::{executable_body_contains_emit, linked_stream_item_type},
    runtime_ops::{runtime_from_wire, runtime_from_wire_required_plan, runtime_to_wire},
    type_projection::EvalTypeProjection,
    RuntimeAssemblyServiceCallTarget,
};

mod activation_relative;
mod prepared_unary;
#[allow(unused_imports)]
pub(crate) use prepared_unary::{execute_provider_unary, prepare_provider_unary};
mod current_scope;

static PROVIDER_STREAM_TASKS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
#[path = "async_stream_cancel/current_scope_tests.rs"]
mod current_scope_tests;
#[cfg(test)]
mod prepared_unary_tests;

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn provider_stream_tasks_active_for_test() -> usize {
    PROVIDER_STREAM_TASKS_ACTIVE.load(Ordering::Acquire)
}

pub(crate) async fn execute_service_call(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    if let Err(error) = context.context.checkpoint(ExecutionCheckpoint::new(
        ExecutionCheckpointKind::GeneratedChunk,
        0,
    )) {
        target.provider_request().cancel();
        return Err(error);
    }
    match AsyncStreamSpawn::for_target(&target) {
        AsyncStreamSpawn::ProviderUnary => {
            execute_provider_unary(context, call, target, args).await
        }
        AsyncStreamSpawn::ProviderStreamProducer => {
            validate_supported_callback_contract(
                &target.descriptor().operation_id,
                &target.descriptor().contract.callbacks,
            )?;
            start_provider_stream(context, call, target, args)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AsyncStreamSpawn {
    ProviderUnary,
    ProviderStreamProducer,
}

impl AsyncStreamSpawn {
    fn for_target(target: &RuntimeAssemblyServiceCallTarget) -> Self {
        match target.descriptor().contract.stream {
            BoundaryStreamContract::ServerStream { .. } => Self::ProviderStreamProducer,
            BoundaryStreamContract::Unary | BoundaryStreamContract::Unsupported { .. } => {
                Self::ProviderUnary
            }
        }
    }
}

enum ProviderUnaryWaitTerminal {
    Provider(Result<RuntimeValue>),
    CallerCancelled,
    DeadlineExceeded(RuntimeError),
}

impl ProviderUnaryWaitTerminal {
    fn into_result(self) -> Result<RuntimeValue> {
        match self {
            Self::Provider(result) => result,
            Self::CallerCancelled => Err(RuntimeError::Cancelled),
            Self::DeadlineExceeded(error) => Err(error),
        }
    }
}

async fn await_provider_unary<F>(
    execution: &ExecutionControl<'_>,
    provider_request: &skiff_runtime_activation::RequestActivationContext,
    provider_future: F,
) -> ProviderUnaryWaitTerminal
where
    F: Future<Output = Result<RuntimeValue>>,
{
    let scope = match current_scope::from_execution(execution) {
        Ok(scope) => scope,
        Err(error) => {
            provider_request.cancel();
            return ProviderUnaryWaitTerminal::DeadlineExceeded(error);
        }
    };
    match current_scope::wait(scope, provider_future).await {
        Ok(result) => ProviderUnaryWaitTerminal::Provider(result),
        Err(error) if error.is_cancelled() => {
            provider_request.cancel();
            ProviderUnaryWaitTerminal::CallerCancelled
        }
        Err(error) => {
            provider_request.cancel();
            ProviderUnaryWaitTerminal::DeadlineExceeded(error)
        }
    }
}

fn start_provider_stream(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let BoundaryStreamContract::ServerStream {
        item_type,
        item_value_plan,
    } = &target.descriptor().contract.stream
    else {
        return Err(AssemblyExecutionHandoffError::unavailable_at(
            AssemblyExecutionLaneKind::AsyncStreamCancel,
            "stream-contract",
        ));
    };
    let boundary = CanonicalServiceBoundaryPlan::new(
        target.descriptor(),
        target.schema_records().as_ref(),
        args.len(),
    )?;
    let caller_package_build_id = context
        .context
        .runtime_assembly_target()?
        .activation_context()
        .implementation_package_build_id()
        .clone();
    let call_site = call.site.clone();
    let caller_stack_at_site = context.context.exception_stack_for_site(call_site.clone());
    let provider_package_build_id = target
        .provider_activation()
        .implementation_package_build_id()
        .clone();
    let provider_service_id = target
        .provider_activation()
        .identity()
        .deployment
        .service_id
        .clone();
    let operation_id = target.descriptor().operation_id.as_str().to_string();
    canonical_scope(
        item_value_plan,
        BoundaryValueOwner::Provider,
        BoundaryValueLifetime::Stream,
    )?;
    let execution = context.execution.owned();
    let request = target.provider_request().clone();
    // Open the request's stream lifetime before parameter materialization: T06 may register a
    // stream-scoped callback while projecting a parameter, and registration must observe the
    // already-live stream lease. Every preparation error below drops this lease immediately.
    let lease = request
        .open_stream()
        .ok_or_else(|| RuntimeError::ProviderUnavailable {
            target: target.descriptor().operation_id.to_string(),
            reason: "request stream lifetime is already terminal".to_string(),
        })?;

    let mut provider_heap = boundary.fresh_provider_heap(context.context.request_heap_limits());
    let hooks = CallbackNativeCapabilityHooks::new(&context.context);
    let provider_args =
        boundary.materialize_parameters(&args, context.heap, &mut provider_heap, &hooks)?;
    let mut provider_context = provider_execution_context(&context.context, &target)?;
    let provider_stream_item_type = provider_stream_item_execution_plan(
        context.interpreter,
        &provider_context,
        context.addr,
        target.executable_addr(),
        context.env,
        &call.type_args,
    )?;
    let provider_stream_runtime_owner = provider_context.take_stream_runtime_owner();
    let owned_provider = Arc::new(OwnedProgramExecutionContext::capture(&provider_context));
    let stream_runtime = context.context.stream_runtime();
    let (stream_value, concrete_sink) = stream_runtime.channel_stream_with_lifetime(
        StreamLifetimeGuard::new(ProviderStreamLifetime { _lease: lease }),
    );
    let stream_cancel = concrete_sink.cancel_signal();
    let sink = StreamSink::new(BoundaryStreamSink {
        inner: concrete_sink,
        item_type: item_type.clone(),
        item_value_plan: item_value_plan.clone(),
        schema_records: Arc::clone(target.schema_records()),
        execution_item_type: provider_stream_item_type.clone(),
        provider_context: Arc::clone(&owned_provider),
        execution: execution.clone(),
        request: request.clone(),
    });
    let receiver_value = match runtime_from_wire(&stream_value, context.heap) {
        Ok(value) => value,
        Err(error) => {
            stream_runtime.cancel(&stream_value);
            return Err(error);
        }
    };

    let mut producer_env = context.env.clone();
    producer_env.stream_sink = Some(sink.clone());
    // This plan only serializes the provider File-IR value passed to `emit`. BoundaryStreamSink
    // remains the semantic owner and applies the canonical contract plan before publication.
    producer_env.current_stream_item_type = provider_stream_item_type;
    let producer = ProviderStreamTask {
        interpreter: context.interpreter.clone_for_stream_producer(),
        provider_context: owned_provider,
        provider_heap,
        provider_env: producer_env,
        caller_addr: context.addr.clone(),
        caller_package_build_id,
        call_site,
        caller_stack_at_site,
        provider_addr: target.executable_addr().clone(),
        provider_receiver_const: target.receiver_const().cloned(),
        provider_package_build_id,
        provider_service_id,
        operation_id,
        type_args: call.type_args.clone(),
        args: provider_args,
        stream_value,
        sink,
        stream_cancel,
        execution,
        request,
        _stream_runtime_owner: provider_stream_runtime_owner,
        #[cfg(test)]
        activity_probe: None,
    };
    spawn_provider_stream(producer);
    Ok(receiver_value)
}

fn provider_stream_item_execution_plan(
    interpreter: &crate::Interpreter,
    provider_context: &ProgramExecutionContext<'_>,
    caller_addr: &skiff_runtime_linked_program::ExecutableAddr,
    provider_addr: &skiff_runtime_linked_program::ExecutableAddr,
    caller_env: &Env,
    type_args: &BTreeMap<String, skiff_runtime_linked_program::LinkedTypeRef>,
) -> Result<Option<RuntimeTypePlan>> {
    let projection = RuntimeExecutionProjection::for_context(interpreter, provider_context)?;
    let resolved = projection.resolve_nested_executable(provider_addr)?;
    if !executable_body_contains_emit(resolved.executable) {
        return Ok(None);
    }
    let item_type =
        linked_stream_item_type(resolved.executable.return_type.as_ref()).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "canonical server-stream provider {} emits values but does not return Stream<T>",
                resolved.executable.symbol
            ))
        })?;
    let type_projection = EvalTypeProjection::from_execution_projection(projection.clone());
    let substitutions = type_projection.call_type_substitutions(
        caller_addr,
        &caller_env.type_substitutions,
        resolved.executable,
        type_args,
    );
    type_projection
        .plan_from_linked_nested_ref_with_substitutions(item_type, &resolved.addr, &substitutions)
        .map(Some)
}

struct ProviderStreamTask {
    interpreter: crate::Interpreter,
    provider_context: Arc<OwnedProgramExecutionContext>,
    provider_heap: RequestHeap,
    provider_env: Env,
    caller_addr: skiff_runtime_linked_program::ExecutableAddr,
    caller_package_build_id: PackageBuildId,
    call_site: InstructionSourceSite,
    caller_stack_at_site: Vec<ExceptionStackFrame>,
    provider_addr: skiff_runtime_linked_program::ExecutableAddr,
    provider_receiver_const: Option<ConstAddr>,
    provider_package_build_id: PackageBuildId,
    provider_service_id: String,
    operation_id: String,
    type_args: BTreeMap<String, skiff_runtime_linked_program::LinkedTypeRef>,
    args: Vec<RuntimeValue>,
    stream_value: Value,
    sink: StreamSink,
    stream_cancel: StreamCancelSignal,
    execution: OwnedExecutionControl,
    request: skiff_runtime_activation::RequestActivationContext,
    _stream_runtime_owner: Option<StreamRuntimeOwner>,
    #[cfg(test)]
    activity_probe: Option<Arc<ProviderStreamTaskActivityProbe>>,
}

fn spawn_provider_stream(producer: ProviderStreamTask) {
    tokio::spawn(async move {
        run_provider_stream(producer).await;
    });
}

async fn run_provider_stream(mut producer: ProviderStreamTask) {
    let _active = ProviderStreamTaskGuard::for_task(&producer);
    let args = std::mem::take(&mut producer.args);
    let terminal = {
        let provider_context = producer.provider_context.borrow();
        let provider_future = call_provider_callable(
            &producer.interpreter,
            provider_context,
            &mut producer.provider_heap,
            &producer.provider_env,
            &producer.caller_addr,
            &producer.provider_addr,
            producer.provider_receiver_const.as_ref(),
            &producer.type_args,
            args,
        );
        await_provider_stream_terminal(
            &producer.execution,
            &producer.stream_cancel,
            provider_future,
        )
        .await
    };

    finish_provider_stream(producer, terminal).await;
}

#[allow(clippy::too_many_arguments)]
async fn call_provider_callable(
    interpreter: &crate::Interpreter,
    context: ProgramExecutionContext<'_>,
    heap: &mut RequestHeap,
    env: &Env,
    caller_addr: &ExecutableAddr,
    provider_addr: &ExecutableAddr,
    receiver_const: Option<&ConstAddr>,
    type_args: &BTreeMap<String, LinkedTypeRef>,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let Some(receiver_const) = receiver_const else {
        return interpreter
            .call_program_executable(
                context,
                heap,
                env,
                caller_addr,
                provider_addr,
                type_args,
                args,
            )
            .await;
    };
    let receiver = interpreter
        .eval_program_const_addr(context.clone(), heap, env, receiver_const)
        .await?;
    interpreter
        .call_program_executable_with_self_direct_carriers(
            context,
            heap,
            env,
            caller_addr,
            provider_addr,
            type_args,
            receiver,
            args.into_iter()
                .map(RuntimeValueCarrier::from)
                .collect::<Vec<_>>(),
        )
        .await
        .map(RuntimeValueCarrier::into_value)
}

async fn finish_provider_stream(producer: ProviderStreamTask, terminal: ProviderTerminal) {
    match terminal {
        ProviderTerminal::Provider(Ok(_)) => {
            publish_provider_terminal(&producer, ProviderStreamPublication::End).await;
        }
        ProviderTerminal::Provider(Err(error)) if error.is_cancelled() => {
            producer.request.cancel();
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
        ProviderTerminal::Provider(Err(error)) if is_deadline_exceeded(&error) => {
            publish_provider_deadline_terminal(&producer, error).await;
        }
        ProviderTerminal::Provider(Err(error)) => {
            let provider_context = producer.provider_context.borrow();
            let terminal = match export_provider_failure(
                &producer.interpreter,
                &provider_context,
                &producer.provider_heap,
                &producer.caller_package_build_id,
                &producer.provider_package_build_id,
                &producer.provider_service_id,
                &producer.operation_id,
                &error,
            ) {
                Ok(error) => StreamRuntimeError::fixed_service_failure_with_import(
                    error,
                    producer.caller_package_build_id.clone(),
                    producer.caller_addr.clone(),
                    producer.call_site.clone(),
                    producer.caller_stack_at_site.clone(),
                    producer.provider_service_id.clone(),
                    producer.operation_id.clone(),
                ),
                Err(error) => stream_runtime_error_from_eval(error),
            };
            publish_provider_terminal(&producer, ProviderStreamPublication::Error(terminal)).await;
        }
        ProviderTerminal::ConsumerCancelled => {
            producer.request.cancel();
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
        ProviderTerminal::RequestCancelled => {
            producer.request.cancel();
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
        ProviderTerminal::DeadlineExceeded(error) => {
            publish_provider_deadline_terminal(&producer, error).await;
        }
    }
}

async fn publish_provider_terminal(
    producer: &ProviderStreamTask,
    publication: ProviderStreamPublication,
) {
    let publication = async {
        match publication {
            ProviderStreamPublication::End => producer.sink.end().await,
            ProviderStreamPublication::Error(error) => producer.sink.fail(error).await,
        }
    };
    match await_provider_publication(
        &producer.execution,
        &producer.stream_cancel,
        &producer.request,
        publication,
    )
    .await
    {
        ProviderPublication::Published => {}
        ProviderPublication::ConsumerCancelled | ProviderPublication::RequestCancelled => {
            producer.request.cancel();
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
        ProviderPublication::DeadlineExceeded(error) => {
            publish_provider_deadline_terminal(producer, error).await;
        }
    }
}

async fn publish_provider_deadline_terminal(producer: &ProviderStreamTask, error: RuntimeError) {
    // Once the deadline branch wins, it is the producer's semantic terminal. Cancel provider
    // work, but do not race publication against the now-cancelled request or the same expired
    // deadline: either would downgrade the consumer-visible timeout to cancellation.
    producer.request.cancel();
    let publication = producer.sink.fail(stream_runtime_error_from_eval(error));
    tokio::pin!(publication);
    tokio::select! {
        biased;
        _ = producer.stream_cancel.wait_cancelled() => {
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
        _ = &mut publication => {}
    }
}

fn is_deadline_exceeded(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::ExecutionBudgetExceeded {
            reason: crate::error::BudgetReason::DeadlineExceeded,
            ..
        }
        | RuntimeError::ScopeTerminal(_) => true,
        RuntimeError::WithSource { error, .. }
        | RuntimeError::WithDiagnosticFrame { error, .. } => is_deadline_exceeded(error),
        _ => false,
    }
}

enum ProviderTerminal {
    Provider(Result<RuntimeValue>),
    ConsumerCancelled,
    RequestCancelled,
    DeadlineExceeded(RuntimeError),
}

async fn await_provider_stream_terminal<F>(
    execution: &OwnedExecutionControl,
    stream_cancel: &StreamCancelSignal,
    provider_future: F,
) -> ProviderTerminal
where
    F: Future<Output = Result<RuntimeValue>>,
{
    let scope = match current_scope::from_owned_execution(execution) {
        Ok(scope) => scope,
        Err(error) => return ProviderTerminal::DeadlineExceeded(error),
    };
    let provider = current_scope::wait(scope, provider_future);
    tokio::pin!(provider);
    tokio::select! {
        biased;
        _ = stream_cancel.wait_cancelled() => ProviderTerminal::ConsumerCancelled,
        result = &mut provider => match result {
            Ok(result) => ProviderTerminal::Provider(result),
            Err(error) if error.is_cancelled() => ProviderTerminal::RequestCancelled,
            Err(error) => ProviderTerminal::DeadlineExceeded(error),
        }
    }
}

enum ProviderPublication {
    Published,
    ConsumerCancelled,
    RequestCancelled,
    DeadlineExceeded(RuntimeError),
}

async fn await_provider_publication<F>(
    execution: &OwnedExecutionControl,
    stream_cancel: &StreamCancelSignal,
    provider_request: &skiff_runtime_activation::RequestActivationContext,
    publication: F,
) -> ProviderPublication
where
    F: Future<Output = ()>,
{
    let scope = match current_scope::from_owned_execution(execution) {
        Ok(scope) => scope,
        Err(error) => return ProviderPublication::DeadlineExceeded(error),
    };
    let publication = current_scope::wait(scope, publication);
    tokio::pin!(publication);
    tokio::select! {
        biased;
        _ = stream_cancel.wait_cancelled() => ProviderPublication::ConsumerCancelled,
        result = &mut publication => match result {
            Ok(()) => ProviderPublication::Published,
            Err(error) if error.is_cancelled() => {
                provider_request.cancel();
                ProviderPublication::RequestCancelled
            }
            Err(error) => {
                provider_request.cancel();
                ProviderPublication::DeadlineExceeded(error)
            }
        }
    }
}

enum ProviderStreamPublication {
    End,
    Error(StreamRuntimeError),
}

fn validate_supported_callback_contract(
    operation_id: &skiff_artifact_model::ContractOperationId,
    callbacks: &BoundaryCallbackContract,
) -> Result<()> {
    if let BoundaryCallbackContract::Unsupported { reason } = callbacks {
        return Err(RuntimeError::Unsupported(format!(
            "canonical service operation {} has unsupported callback semantics: {reason:?}",
            operation_id
        )));
    }
    Ok(())
}

#[cfg(test)]
fn deadline_error(execution: &ExecutionControl<'_>) -> RuntimeError {
    match execution.poll_execution_budget() {
        Err(
            error @ skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
                skiff_runtime_capability_context::ExecutionBudgetFailure {
                    reason:
                        skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
                    ..
                },
            ),
        ) => error.into(),
        Ok(())
        | Err(skiff_runtime_capability_context::ExecutionControlError::Cancelled)
        | Err(skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(_)) => {
            RuntimeError::ExecutionBudgetExceeded {
                reason: crate::error::BudgetReason::DeadlineExceeded,
                instruction_count: 0,
                limit: None,
                elapsed_ms: 0.0,
            }
        }
    }
}

async fn await_stream_item_publication<T, F>(
    execution: &OwnedExecutionControl,
    provider_request: &skiff_runtime_activation::RequestActivationContext,
    provider_future: F,
) -> StreamRuntimeResult<T>
where
    F: Future<Output = StreamRuntimeResult<T>>,
{
    let scope =
        current_scope::from_owned_execution(execution).map_err(stream_runtime_error_from_eval)?;
    match current_scope::wait(scope, provider_future).await {
        Ok(result) => result,
        Err(error) => {
            provider_request.cancel();
            Err(stream_runtime_error_from_eval(error))
        }
    }
}

fn provider_execution_context<'a>(
    receiver: &ProgramExecutionContext<'a>,
    target: &RuntimeAssemblyServiceCallTarget,
) -> Result<ProgramExecutionContext<'a>> {
    let provider_target = receiver
        .runtime_assembly_target()?
        .with_request_activation(target.provider_request().clone())?;
    let provider = target.provider_activation();
    let service_id = provider.identity().deployment.service_id.as_str();
    let websocket_entry_id = provider.websocket_entry_id().map(|entry| entry.as_str());
    Ok(receiver
        .clone()
        .with_provider_websocket_capability(service_id, websocket_entry_id)?
        .with_runtime_assembly_target(provider_target)
        .with_provider_service_stack_scope())
}

#[allow(clippy::too_many_arguments)]
fn export_provider_failure(
    interpreter: &crate::Interpreter,
    provider_context: &ProgramExecutionContext<'_>,
    provider_heap: &RequestHeap,
    caller_package_build_id: &PackageBuildId,
    provider_package_build_id: &PackageBuildId,
    provider_service_id: &str,
    operation_id: &str,
    error: &RuntimeError,
) -> Result<OpaqueServiceError> {
    let target = provider_context.runtime_assembly_target()?;
    let projection = RuntimeExecutionProjection::for_context(interpreter, provider_context)?;
    let telemetry = provider_context.telemetry_context();
    let fallback_source = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeBoundaryDispatch,
    };
    let fallback_stack = provider_context.exception_stack_for_site(fallback_source.clone());
    CanonicalServiceErrorChannel::export_provider_failure_with_diagnostic(
        error,
        ServiceErrorExportContext {
            execution_image: target.execution_image().as_ref(),
            type_view: projection.type_view(),
            provider_heap,
            provider_package_build_id,
            caller_package_build_id: Some(caller_package_build_id),
            provider_service_id,
            operation_id,
        },
        RestrictedServiceDiagnosticExportContext {
            telemetry: &telemetry,
            provider_activation_id: target.activation_context().activation_id().as_str(),
            request_generation: target.request_activation().generation(),
            fallback_source: &fallback_source,
            fallback_stack: &fallback_stack,
        },
        || provider_context.next_exception_correlation(),
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_value(
    stage: &str,
    ty: &ContractTypeRef,
    schema_records: &PackageSchemaRecords,
    value_plan: &BoundaryValuePlan,
    value: &RuntimeValue,
    source_heap: &RequestHeap,
    destination_heap: &mut RequestHeap,
    scope: ServiceLinkableMaterializationScope,
    hooks: &dyn skiff_runtime_boundary::service_linkable::ServiceLinkableCapabilityHooks,
) -> Result<RuntimeValue> {
    ServiceLinkableContractPlan::new(ty, schema_records, value_plan)
        .and_then(|plan| plan.materialize(value, source_heap, destination_heap, scope, hooks))
        .map_err(|error| materialization_error(stage, error))
}

fn canonical_scope(
    value_plan: &BoundaryValuePlan,
    detached_owner: BoundaryValueOwner,
    detached_lifetime: BoundaryValueLifetime,
) -> Result<ServiceLinkableMaterializationScope> {
    match value_plan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            owner,
            lifetime,
            ..
        } if *owner == detached_owner && *lifetime == detached_lifetime => {
            Ok(ServiceLinkableMaterializationScope {
                owner: *owner,
                lifetime: *lifetime,
            })
        }
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            owner,
            lifetime,
            ..
        } if *owner == BoundaryValueOwner::CapabilityOwner
            && matches!(
                lifetime,
                BoundaryValueLifetime::Request | BoundaryValueLifetime::Stream
            ) => Ok(ServiceLinkableMaterializationScope {
            owner: *owner,
            lifetime: *lifetime,
        }),
        BoundaryValuePlan::Unsupported { .. } | BoundaryValuePlan::Linkable { .. } => {
            Err(RuntimeError::InvalidArtifact(format!(
                "canonical boundary value plan does not match {detached_owner:?}/{detached_lifetime:?} lane ownership"
            )))
        }
    }
}

fn materialization_error(stage: &str, error: ServiceLinkableMaterializationError) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "canonical in-process {stage} materialization failed: {error}"
    ))
}

#[derive(Debug)]
struct ProviderStreamLifetime {
    _lease: RequestStreamLease,
}

impl StreamLifetimeGuardApi for ProviderStreamLifetime {}

struct BoundaryStreamSink {
    inner: StreamSink,
    item_type: ContractTypeRef,
    item_value_plan: BoundaryValuePlan,
    schema_records: crate::AdmittedPackageSchemaRecords,
    execution_item_type: Option<RuntimeTypePlan>,
    provider_context: Arc<OwnedProgramExecutionContext>,
    execution: OwnedExecutionControl,
    request: skiff_runtime_activation::RequestActivationContext,
}

pub(crate) fn is_canonical_boundary_stream_sink(sink: &StreamSink) -> bool {
    sink.downcast_ref::<BoundaryStreamSink>().is_some()
}

impl BoundaryStreamSink {
    fn materialize_item(&self, item: Value) -> StreamRuntimeResult<Value> {
        let mut source_heap = RequestHeap::default();
        let source = runtime_from_wire_required_plan(
            &item,
            self.execution_item_type.as_ref(),
            "provider stream emit item",
            &mut source_heap,
        )
        .map_err(stream_runtime_error_from_eval)?;
        let mut receiver_heap = RequestHeap::default();
        let provider_context = self.provider_context.borrow();
        let hooks = CallbackNativeCapabilityHooks::new(&provider_context);
        let materialized = materialize_value(
            "stream item",
            &self.item_type,
            self.schema_records.as_ref(),
            &self.item_value_plan,
            &source,
            &source_heap,
            &mut receiver_heap,
            canonical_scope(
                &self.item_value_plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .map_err(stream_runtime_error_from_eval)?,
            &hooks,
        )
        .map_err(stream_runtime_error_from_eval)?;
        runtime_to_wire(&materialized, &receiver_heap).map_err(stream_runtime_error_from_eval)
    }

    fn project_internal_item(
        &self,
        item: RuntimeValue,
        source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        if !matches!(
            self.item_value_plan,
            BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::CallbackCapability,
                ..
            }
        ) {
            return Ok(None);
        }
        let provider_context = self.provider_context.borrow();
        let mut receiver_heap = provider_context.request_heap();
        let hooks = CallbackNativeCapabilityHooks::new(&provider_context);
        let materialized = materialize_value(
            "stream item",
            &self.item_type,
            self.schema_records.as_ref(),
            &self.item_value_plan,
            &item,
            source_heap,
            &mut receiver_heap,
            canonical_scope(
                &self.item_value_plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .map_err(stream_runtime_error_from_eval)?,
            &hooks,
        )
        .map_err(stream_runtime_error_from_eval)?;
        Ok(Some(StreamInternalItem::new(materialized, receiver_heap)))
    }
}

impl fmt::Debug for BoundaryStreamSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoundaryStreamSink")
    }
}

impl StreamSinkApi for BoundaryStreamSink {
    fn project_runtime_item(
        &self,
        item: RuntimeValue,
        source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        self.project_internal_item(item, source_heap)
    }

    fn send_internal_with_cancellation<'a>(
        &'a self,
        item: StreamInternalItem,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            await_stream_item_publication(
                &self.execution,
                &self.request,
                self.inner
                    .send_internal_with_cancellation(item, signals, cancel_tokens),
            )
            .await
        })
    }

    fn send<'a>(
        &'a self,
        item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let item = self.materialize_item(item)?;
            await_stream_item_publication(&self.execution, &self.request, self.inner.send(item))
                .await
        })
    }

    fn send_with_cancel<'a>(
        &'a self,
        item: Value,
        cancel_flags: &'a [Arc<std::sync::atomic::AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let item = self.materialize_item(item)?;
            await_stream_item_publication(
                &self.execution,
                &self.request,
                self.inner.send_with_cancel(item, cancel_flags),
            )
            .await
        })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        item: Value,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let item = self.materialize_item(item)?;
            await_stream_item_publication(
                &self.execution,
                &self.request,
                self.inner
                    .send_with_cancellation(item, signals, cancel_tokens),
            )
            .await
        })
    }

    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.inner.end().await })
    }

    fn fail<'a>(
        &'a self,
        error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.inner.fail(error).await })
    }

    fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    fn is_same_stream(&self, other: &StreamSink) -> bool {
        other
            .downcast_ref::<Self>()
            .map(|other| self.inner.is_same_stream(&other.inner))
            .unwrap_or_else(|| self.inner.is_same_stream(other))
    }

    fn cancel_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.inner.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        self.inner.cancel_signal()
    }
}

#[cfg(test)]
#[derive(Default)]
struct ProviderStreamTaskActivityProbe {
    entered: AtomicUsize,
    active: AtomicUsize,
}

#[cfg(test)]
impl ProviderStreamTaskActivityProbe {
    fn entered(&self) -> usize {
        self.entered.load(Ordering::Acquire)
    }

    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

struct ProviderStreamTaskGuard {
    #[cfg(test)]
    activity_probe: Option<Arc<ProviderStreamTaskActivityProbe>>,
}

impl ProviderStreamTaskGuard {
    fn for_task(task: &ProviderStreamTask) -> Self {
        PROVIDER_STREAM_TASKS_ACTIVE.fetch_add(1, Ordering::AcqRel);
        #[cfg(test)]
        {
            let activity_probe = task.activity_probe.clone();
            if let Some(probe) = &activity_probe {
                probe.entered.fetch_add(1, Ordering::AcqRel);
                probe.active.fetch_add(1, Ordering::AcqRel);
            }
            Self { activity_probe }
        }
        #[cfg(not(test))]
        {
            let _ = task;
            Self {}
        }
    }
}

impl Drop for ProviderStreamTaskGuard {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(probe) = &self.activity_probe {
            probe.active.fetch_sub(1, Ordering::AcqRel);
        }
        PROVIDER_STREAM_TASKS_ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        task::{Context, Poll, Wake, Waker},
    };

    use skiff_artifact_model::{
        ActivationPolicy, AssemblyIdentity, BoundaryFeatureUnavailableReason,
        DeploymentArtifactIdentity, DeploymentPolicy, DeploymentRevision, PackageBuildId,
        PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord,
        ResourcePolicy, ServiceDeploymentRef,
    };
    use skiff_runtime_activation::{
        ActivationContext, ActivationIdentity, ActivationOwnedBindings, RequestActivationContext,
    };
    use skiff_runtime_boundary::service_linkable::FailClosedServiceLinkableCapabilityHooks;
    use skiff_runtime_capability_context::{
        ExecutionDeadlineSource, ExecutionScope, ExecutionScopeTerminal, StreamCancelSignalApi,
        StreamPoll, StreamRuntime,
    };
    use skiff_runtime_linked_program::{LinkedCallTarget, LinkedExprIr};
    use skiff_runtime_model::{
        runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields},
        type_plan::RuntimeTypeNode,
    };

    use crate::{
        assembly_execution::{
            ordinary::tests::{
                service_error_consumer::{
                    ConsumerTopology, ProviderFailureKind, ServiceErrorConsumerFixture,
                },
                test_runtime,
            },
            service_error_channel::{
                start_restricted_service_diagnostic_probe_for_test,
                take_restricted_service_diagnostics_for_test,
            },
            RuntimeAssemblyExecutionProjection,
        },
        runtime_ops::runtime_to_wire_required_plan,
        Interpreter,
    };

    use super::*;

    #[derive(Debug)]
    struct TestStreamCancel(CancellationToken);

    impl StreamCancelSignalApi for TestStreamCancel {
        fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move { self.0.wait_cancelled().await })
        }
    }

    #[derive(Debug)]
    struct TestLifetimeDrop(Arc<AtomicUsize>);

    impl Drop for TestLifetimeDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    impl StreamLifetimeGuardApi for TestLifetimeDrop {}

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn first_poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        let waker = Waker::from(Arc::new(NoopWake));
        future.poll(&mut Context::from_waker(&waker))
    }

    fn assert_inherited_request_deadline(
        error: &RuntimeError,
        request_scope: &ExecutionScope,
        expected_deadline: Instant,
    ) {
        let RuntimeError::ScopeTerminal(carrier) = error else {
            panic!("request deadline must remain an internal scope terminal, got {error:?}");
        };
        let ExecutionScopeTerminal::InheritedDeadlineExceeded(deadline) = carrier.terminal() else {
            panic!(
                "request deadline must remain inherited rather than locally owned, got {:?}",
                carrier.terminal()
            );
        };
        assert_eq!(deadline.at(), expected_deadline);
        assert_eq!(deadline.source(), &ExecutionDeadlineSource::Request);
        assert_eq!(deadline.nesting(), 0);
        assert_eq!(request_scope.nesting(), 0);
        assert_eq!(request_scope.effective_deadline(), Some(deadline));
        assert!(
            !carrier.is_owned_by(request_scope),
            "a request deadline is not owned by a lexical timeout scope"
        );
    }

    fn test_stream_cancel() -> (CancellationToken, StreamCancelSignal) {
        let token = CancellationToken::new();
        (
            token.clone(),
            StreamCancelSignal::new(TestStreamCancel(token)),
        )
    }

    #[test]
    fn in_process_stream_spawn_matrix_is_exhaustive() {
        let variants = BTreeSet::from([
            AsyncStreamSpawn::ProviderUnary,
            AsyncStreamSpawn::ProviderStreamProducer,
        ]);
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn unsupported_callback_contract_remains_a_typed_runtime_error() {
        let error = validate_supported_callback_contract(
            &skiff_artifact_model::ContractOperationId::new("operation:unsupported-callback"),
            &BoundaryCallbackContract::Unsupported {
                reason: BoundaryFeatureUnavailableReason::UnknownSemantics,
            },
        )
        .expect_err("unsupported callback semantics must fail closed");
        assert!(matches!(error, RuntimeError::Unsupported(_)));
    }

    #[tokio::test]
    async fn ready_provider_unary_returns_without_forced_yield() {
        let execution = test_runtime::execution_control();
        let request = RequestActivationContext::begin(activation("ready", "ready-build")).unwrap();
        let value =
            await_provider_unary(&execution, &request, async { Ok(RuntimeValue::Bool(true)) })
                .await
                .into_result()
                .expect("ready provider should return during the initial poll");
        assert_eq!(value, RuntimeValue::Bool(true));
        assert!(request.open_stream().is_some());
    }

    #[tokio::test]
    async fn pending_provider_unary_wakes_from_provider_completion() {
        let execution = test_runtime::execution_control();
        let request =
            RequestActivationContext::begin(activation("provider", "provider-build")).unwrap();
        let (complete, completed) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn({
            let execution = execution.clone();
            let request = request.clone();
            async move {
                await_provider_unary(&execution, &request, async move {
                    completed.await.expect("provider completion sender");
                    Ok(RuntimeValue::Bool(true))
                })
                .await
                .into_result()
            }
        });
        tokio::task::yield_now().await;
        complete.send(()).unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), RuntimeValue::Bool(true));
    }

    #[tokio::test]
    async fn pending_provider_unary_wakes_from_request_cancellation() {
        let execution = test_runtime::execution_control();
        let cancellation = execution.cancellation_token();
        let request =
            RequestActivationContext::begin(activation("cancel", "cancel-build")).unwrap();
        let waiter = tokio::spawn({
            let execution = execution.clone();
            let request = request.clone();
            async move {
                await_provider_unary(&execution, &request, std::future::pending())
                    .await
                    .into_result()
            }
        });

        cancellation.cancel();

        let error = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("cooperative cancellation should wake the pending provider")
            .expect("provider waiter should not panic")
            .expect_err("pending provider should terminate as cancelled");
        assert!(error.is_cancelled());
        assert!(
            request.open_stream().is_none(),
            "caller cancellation must cancel the provider request"
        );
    }

    #[tokio::test]
    async fn f445h_e4r7_stream_deadline_pending_unary_preserves_inherited_request_carrier() {
        let request_deadline = Instant::now() - std::time::Duration::from_millis(1);
        let execution = test_runtime::execution_control_with_deadline(Some(request_deadline));
        let request_scope = execution
            .execution_scope()
            .expect("test execution exposes its current request scope");
        let request =
            RequestActivationContext::begin(activation("deadline", "deadline-build")).unwrap();
        let error = await_provider_unary(&execution, &request, std::future::pending())
            .await
            .into_result()
            .expect_err("expired deadline should wake the pending provider");
        assert_inherited_request_deadline(&error, &request_scope, request_deadline);
        assert!(
            request.open_stream().is_none(),
            "deadline must emit the provider request cancellation signal"
        );
    }

    #[tokio::test]
    async fn request_cancel_precedes_expired_deadline_and_ready_provider() {
        let execution = test_runtime::execution_control_with_deadline(Some(
            Instant::now() - std::time::Duration::from_millis(1),
        ));
        let cancellation = execution.cancellation_token();
        cancellation.cancel();
        let request = RequestActivationContext::begin(activation("race", "race-build")).unwrap();
        let error = await_provider_unary(&execution, &request, async {
            Err(RuntimeError::FileError {
                message: "ready provider failure".to_string(),
            })
        })
        .await
        .into_result()
        .expect_err("pre-cancelled request should win the biased select");
        assert!(matches!(error, RuntimeError::Cancelled));
    }

    #[test]
    fn selected_deadline_does_not_downgrade_after_late_cancellation() {
        let execution = test_runtime::execution_control_with_deadline(Some(
            Instant::now() - std::time::Duration::from_millis(1),
        ));
        execution.cancellation_token().cancel();

        assert!(matches!(
            deadline_error(&execution),
            RuntimeError::ExecutionBudgetExceeded {
                reason: crate::error::BudgetReason::DeadlineExceeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn provider_stream_normal_completion_remains_provider_terminal() {
        let diagnostic_generation = u64::MAX - 1;
        start_restricted_service_diagnostic_probe_for_test(diagnostic_generation);
        let (_consumer_cancel, stream_cancel) = test_stream_cancel();
        let execution = test_runtime::execution_control().owned();
        let terminal = await_provider_stream_terminal(&execution, &stream_cancel, async {
            Ok(RuntimeValue::Bool(true))
        })
        .await;

        assert!(matches!(
            terminal,
            ProviderTerminal::Provider(Ok(RuntimeValue::Bool(true)))
        ));
        assert!(
            take_restricted_service_diagnostics_for_test(diagnostic_generation).is_empty(),
            "successful provider completion must not submit a failure diagnostic"
        );
    }

    #[tokio::test]
    async fn provider_stream_consumer_cancel_is_control_terminal() {
        let (consumer_cancel, stream_cancel) = test_stream_cancel();
        let execution = test_runtime::execution_control().owned();
        consumer_cancel.cancel();
        let terminal =
            await_provider_stream_terminal(&execution, &stream_cancel, std::future::pending())
                .await;

        assert!(matches!(terminal, ProviderTerminal::ConsumerCancelled));
    }

    #[tokio::test]
    async fn provider_stream_request_cancel_is_control_terminal() {
        let (_consumer_cancel, stream_cancel) = test_stream_cancel();
        let execution = test_runtime::execution_control().owned();
        execution.cancellation_token().cancel();
        let terminal =
            await_provider_stream_terminal(&execution, &stream_cancel, std::future::pending())
                .await;

        assert!(matches!(terminal, ProviderTerminal::RequestCancelled));
    }

    #[tokio::test]
    async fn provider_stream_control_ordering_precedes_ready_provider_error() {
        let (consumer_cancel, stream_cancel) = test_stream_cancel();
        let execution = test_runtime::execution_control_with_deadline(Some(
            Instant::now() - std::time::Duration::from_millis(1),
        ))
        .owned();
        consumer_cancel.cancel();
        execution.cancellation_token().cancel();
        let terminal = await_provider_stream_terminal(&execution, &stream_cancel, async {
            Err(RuntimeError::FileError {
                message: "ready provider error".to_string(),
            })
        })
        .await;
        assert!(matches!(terminal, ProviderTerminal::ConsumerCancelled));

        let (_consumer_cancel, stream_cancel) = test_stream_cancel();
        let execution = test_runtime::execution_control_with_deadline(Some(
            Instant::now() - std::time::Duration::from_millis(1),
        ))
        .owned();
        execution.cancellation_token().cancel();
        let terminal = await_provider_stream_terminal(&execution, &stream_cancel, async {
            Err(RuntimeError::FileError {
                message: "ready provider error".to_string(),
            })
        })
        .await;
        assert!(matches!(terminal, ProviderTerminal::RequestCancelled));
    }

    #[tokio::test]
    async fn f445h_e4r7_stream_deadline_helper_matrix_preserves_carrier_before_raw_boundary() {
        let request_deadline = Instant::now() - std::time::Duration::from_millis(1);
        let execution =
            test_runtime::execution_control_with_deadline(Some(request_deadline)).owned();
        let request_scope = execution
            .execution_scope()
            .expect("test execution exposes its current request scope");
        let item_request =
            RequestActivationContext::begin(activation("stream-deadline", "stream-build")).unwrap();
        let (_consumer_cancel, stream_cancel) = test_stream_cancel();

        let terminal =
            await_provider_stream_terminal(&execution, &stream_cancel, std::future::pending())
                .await;
        let ProviderTerminal::DeadlineExceeded(error) = terminal else {
            panic!("provider terminal must preserve the request deadline carrier");
        };
        assert_inherited_request_deadline(&error, &request_scope, request_deadline);

        let item_error = await_stream_item_publication(
            &execution,
            &item_request,
            std::future::pending::<StreamRuntimeResult<()>>(),
        )
        .await
        .expect_err("stream item publication must wake on deadline");
        assert!(matches!(item_error, StreamRuntimeError::Cancelled));
        assert!(
            item_request.open_stream().is_none(),
            "stream item deadline must cancel the provider request"
        );

        let publication_request =
            RequestActivationContext::begin(activation("publication-deadline", "stream-build"))
                .unwrap();
        let publication = await_provider_publication(
            &execution,
            &stream_cancel,
            &publication_request,
            std::future::pending(),
        )
        .await;
        let ProviderPublication::DeadlineExceeded(error) = publication else {
            panic!("terminal publication must preserve the request deadline carrier");
        };
        assert_inherited_request_deadline(&error, &request_scope, request_deadline);
        assert!(
            publication_request.open_stream().is_none(),
            "stream terminal publication deadline must cancel the provider request"
        );
    }

    #[tokio::test]
    async fn stream_item_and_publication_request_cancel_precede_expired_deadline() {
        let execution = test_runtime::execution_control_with_deadline(Some(
            Instant::now() - std::time::Duration::from_millis(1),
        ))
        .owned();
        execution.cancellation_token().cancel();
        let item_request =
            RequestActivationContext::begin(activation("item-cancel-race", "stream-build"))
                .unwrap();

        let item_error = await_stream_item_publication(
            &execution,
            &item_request,
            std::future::pending::<StreamRuntimeResult<()>>(),
        )
        .await
        .expect_err("request cancellation must wake a pending item publication");
        assert!(matches!(
            RuntimeError::from(item_error),
            RuntimeError::Cancelled
        ));
        assert!(
            item_request.open_stream().is_none(),
            "item cancellation must propagate to the provider request"
        );

        let publication_request =
            RequestActivationContext::begin(activation("publication-cancel-race", "stream-build"))
                .unwrap();
        let (_consumer_cancel, stream_cancel) = test_stream_cancel();
        let publication = await_provider_publication(
            &execution,
            &stream_cancel,
            &publication_request,
            std::future::pending(),
        )
        .await;
        assert!(matches!(publication, ProviderPublication::RequestCancelled));
        assert!(
            publication_request.open_stream().is_none(),
            "publication cancellation must propagate to the provider request"
        );
    }

    #[tokio::test]
    async fn f445h_e4r7_stream_deadline_provider_terminal_reaches_raw_consumer_as_cancelled() {
        let (mut task, _, stream_runtime, stream_value, _) = provider_stream_failure_task();
        stream_runtime.cancel(&stream_value);
        let lifetime_drops = Arc::new(AtomicUsize::new(0));
        let (stream_value, sink) = stream_runtime.channel_stream_with_lifetime(
            StreamLifetimeGuard::new(TestLifetimeDrop(Arc::clone(&lifetime_drops))),
        );
        task.stream_value = stream_value.clone();
        task.stream_cancel = sink.cancel_signal();
        task.sink = sink;
        let request_deadline = Instant::now() - std::time::Duration::from_millis(1);
        task.execution =
            test_runtime::execution_control_with_deadline(Some(request_deadline)).owned();
        let request_scope = task
            .execution
            .execution_scope()
            .expect("test execution exposes its current request scope");
        let provider_request = task.request.clone();
        let mut consumer = Box::pin(stream_runtime.next(&stream_value));
        assert!(
            matches!(first_poll(consumer.as_mut()), Poll::Pending),
            "raw consumer must attach and enter a real Pending poll before the terminal"
        );

        let terminal = await_provider_stream_terminal(
            &task.execution,
            &task.stream_cancel,
            std::future::pending(),
        )
        .await;
        let ProviderTerminal::DeadlineExceeded(error) = &terminal else {
            panic!("provider wait must preserve the request deadline carrier");
        };
        assert_inherited_request_deadline(error, &request_scope, request_deadline);
        finish_provider_stream(task, terminal).await;

        let error = consumer
            .await
            .expect_err("raw stream boundary must project the deadline carrier to cancellation");
        assert!(
            matches!(error, StreamRuntimeError::Cancelled),
            "unexpected stream terminal: {error:?}"
        );
        assert!(
            provider_request.open_stream().is_none(),
            "deadline terminal must cancel the provider request"
        );
        assert_eq!(
            lifetime_drops.load(Ordering::Acquire),
            1,
            "raw cancellation consumption must release the stream lifetime exactly once"
        );
    }

    #[tokio::test]
    async fn f445h_e4r7_stream_deadline_item_publication_reaches_attached_raw_consumer_as_cancelled(
    ) {
        let (mut task, _, stream_runtime, stream_value, _) = provider_stream_failure_task();
        let provider_request = task.request.clone();
        let mut consumer = Box::pin(stream_runtime.next(&stream_value));
        assert!(
            matches!(first_poll(consumer.as_mut()), Poll::Pending),
            "raw consumer must attach and enter a real Pending poll before the deadline"
        );

        task.execution = test_runtime::execution_control_with_deadline(Some(
            Instant::now() - std::time::Duration::from_millis(1),
        ))
        .owned();
        let item_error = await_stream_item_publication(
            &task.execution,
            &provider_request,
            std::future::pending::<StreamRuntimeResult<()>>(),
        )
        .await
        .expect_err("item publication must project its request deadline to raw cancellation");
        assert!(matches!(item_error, StreamRuntimeError::Cancelled));

        finish_provider_stream(
            task,
            ProviderTerminal::Provider(Err(RuntimeError::from(item_error))),
        )
        .await;

        let error = consumer
            .await
            .expect_err("attached raw consumer must observe internal cancellation");
        assert!(matches!(error, StreamRuntimeError::Cancelled));
        assert!(
            provider_request.open_stream().is_none(),
            "item deadline must cancel the provider request"
        );
    }

    #[tokio::test]
    async fn f445h_e4r7_stream_deadline_blocked_terminal_preserves_buffered_item_then_cancelled() {
        let (mut task, _, stream_runtime, stream_value, _) = provider_stream_failure_task();
        stream_runtime.cancel(&stream_value);
        let lifetime_drops = Arc::new(AtomicUsize::new(0));
        let (stream_value, sink) = stream_runtime.channel_stream_with_lifetime(
            StreamLifetimeGuard::new(TestLifetimeDrop(Arc::clone(&lifetime_drops))),
        );
        task.stream_value = stream_value.clone();
        task.stream_cancel = sink.cancel_signal();
        task.sink = sink;
        task.sink
            .send(serde_json::json!("buffered-before-terminal"))
            .await
            .expect("test stream buffer must be full before terminal publication");
        task.execution = test_runtime::execution_control_with_deadline(Some(
            Instant::now() - std::time::Duration::from_millis(1),
        ))
        .owned();
        let provider_request = task.request.clone();
        let mut publication = Box::pin(publish_provider_terminal(
            &task,
            ProviderStreamPublication::End,
        ));
        assert!(
            matches!(first_poll(publication.as_mut()), Poll::Pending),
            "the raw cancellation terminal must block behind the full stream buffer"
        );
        assert!(
            provider_request.open_stream().is_none(),
            "publication deadline must cancel the provider request"
        );

        let first = stream_runtime
            .next(&stream_value)
            .await
            .expect("buffered item must remain visible before the deadline terminal");
        assert!(matches!(
            first,
            StreamPoll::Item(value)
                if value == serde_json::json!("buffered-before-terminal")
        ));
        publication.await;

        let error = stream_runtime
            .next(&stream_value)
            .await
            .expect_err("deadline must replace the blocked End with raw cancellation");
        assert!(matches!(error, StreamRuntimeError::Cancelled));
        assert_eq!(
            lifetime_drops.load(Ordering::Acquire),
            1,
            "raw cancellation consumption must release the stream lifetime exactly once"
        );
    }

    #[tokio::test]
    async fn restricted_service_diagnostic_server_stream_failure_submits_once() {
        let (task, generation, stream_runtime, stream_value, _) = provider_stream_failure_task();

        start_restricted_service_diagnostic_probe_for_test(generation);
        run_provider_stream(task).await;

        let terminal = stream_runtime
            .next(&stream_value)
            .await
            .expect_err("provider stream should publish the fixed failure");
        let (fixed, _) = terminal
            .fixed_service_failure_parts()
            .expect("stream terminal retains typed fixed failure");
        let diagnostics = take_restricted_service_diagnostics_for_test(generation);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].correlation.trace_id,
            fixed.envelope().trace_id()
        );
        assert_eq!(
            diagnostics[0].correlation.error_id,
            fixed.envelope().error_id()
        );
    }

    #[tokio::test]
    async fn restricted_service_diagnostic_server_stream_request_cancel_submits_zero() {
        let (task, generation, _, _, cancellation) = provider_stream_failure_task();
        cancellation.cancel();
        start_restricted_service_diagnostic_probe_for_test(generation);

        run_provider_stream(task).await;

        assert!(
            take_restricted_service_diagnostics_for_test(generation).is_empty(),
            "request cancellation must bypass provider failure export"
        );
    }

    #[test]
    fn ordinary_service_call_rebinds_websocket_capability_to_provider_activation() {
        let fixture = ServiceErrorConsumerFixture::new(
            ProviderFailureKind::PublicRecord,
            ConsumerTopology::OneHop,
            true,
            false,
        );
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let receiver_target = fixture.caller_eval_target();
        let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(
            receiver_target.execution_image(),
        ));
        let caller = projection
            .resolve_executable(fixture.caller_addr())
            .expect("linked caller executable");
        let instruction = caller
            .executable
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                LinkedExprIr::Call { call } => match &call.target {
                    LinkedCallTarget::ActivationRelativeService { instruction } => {
                        Some(instruction)
                    }
                    _ => None,
                },
                _ => None,
            })
            .expect("linked ordinary service call");
        let target = receiver_target
            .resolve_service_call(instruction)
            .expect("resolved provider target");
        let receiver_context = fixture.execution_context(&interpreter, receiver_target);
        assert_eq!(
            receiver_context.websocket_context().service_id(),
            "test-service"
        );

        let provider_context =
            provider_execution_context(&receiver_context, &target).expect("provider context");
        assert_eq!(
            provider_context.websocket_context().service_id(),
            target
                .provider_activation()
                .identity()
                .deployment
                .service_id
                .as_str()
        );
        assert_eq!(
            provider_context.websocket_context().websocket_entry_id(),
            target
                .provider_activation()
                .websocket_entry_id()
                .map(|entry| entry.as_str())
        );
        assert_ne!(
            provider_context.websocket_context().service_id(),
            receiver_context.websocket_context().service_id()
        );

        let owned = OwnedProgramExecutionContext::capture(&provider_context);
        let borrowed = owned.borrow();
        assert_eq!(
            borrowed.websocket_context().service_id(),
            provider_context.websocket_context().service_id()
        );
        assert_eq!(
            borrowed.websocket_context().websocket_entry_id(),
            provider_context.websocket_context().websocket_entry_id()
        );
    }

    pub(super) fn provider_stream_failure_task() -> (
        ProviderStreamTask,
        u64,
        StreamRuntime,
        Value,
        CancellationToken,
    ) {
        let fixture = ServiceErrorConsumerFixture::new(
            ProviderFailureKind::PublicRecord,
            ConsumerTopology::OneHop,
            true,
            false,
        );
        let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
        let receiver_target = fixture.caller_eval_target();
        let generation = receiver_target.request_activation().generation();
        let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(
            receiver_target.execution_image(),
        ));
        let caller = projection
            .resolve_executable(fixture.caller_addr())
            .expect("linked caller executable");
        let call = caller
            .executable
            .body
            .expressions
            .iter()
            .find_map(|expression| match expression {
                LinkedExprIr::Call { call }
                    if matches!(
                        call.target,
                        LinkedCallTarget::ActivationRelativeService { .. }
                    ) =>
                {
                    Some(call.clone())
                }
                _ => None,
            })
            .expect("linked service call");
        let instruction = match &call.target {
            LinkedCallTarget::ActivationRelativeService { instruction } => instruction,
            _ => unreachable!("selected call is activation-relative"),
        };
        let target = receiver_target
            .resolve_service_call(instruction)
            .expect("resolved provider target");
        let receiver_context = fixture.execution_context(&interpreter, receiver_target);
        let mut provider_context =
            provider_execution_context(&receiver_context, &target).expect("provider context");
        let provider_stream_owner = provider_context.take_stream_runtime_owner();
        let stream_runtime = provider_context.stream_runtime();
        let (stream_value, sink) = stream_runtime.channel_stream();
        let stream_cancel = sink.cancel_signal();
        let caller_stack_at_site = receiver_context.exception_stack_for_site(call.site.clone());
        let execution = receiver_context.execution().owned();
        let cancellation = execution.cancellation_token();
        let task = ProviderStreamTask {
            interpreter: interpreter.clone_for_stream_producer(),
            provider_context: Arc::new(OwnedProgramExecutionContext::capture(&provider_context)),
            provider_heap: RequestHeap::default(),
            provider_env: Env::new(),
            caller_addr: fixture.caller_addr().clone(),
            caller_package_build_id: fixture.caller_build().clone(),
            call_site: call.site.clone(),
            caller_stack_at_site,
            provider_addr: target.executable_addr().clone(),
            provider_receiver_const: target.receiver_const().cloned(),
            provider_package_build_id: target
                .provider_activation()
                .implementation_package_build_id()
                .clone(),
            provider_service_id: target
                .provider_activation()
                .identity()
                .deployment
                .service_id
                .clone(),
            operation_id: target.descriptor().operation_id.as_str().to_string(),
            type_args: call.type_args.clone(),
            args: Vec::new(),
            stream_value: stream_value.clone(),
            sink,
            stream_cancel,
            execution,
            request: target.provider_request().clone(),
            _stream_runtime_owner: provider_stream_owner,
            activity_probe: None,
        };
        (task, generation, stream_runtime, stream_value, cancellation)
    }

    #[test]
    fn in_process_stream_task_probe_returns_to_zero_exactly_once() {
        let activity_probe = Arc::new(ProviderStreamTaskActivityProbe::default());
        let mut task = provider_stream_failure_task().0;
        task.activity_probe = Some(Arc::clone(&activity_probe));
        let guard = ProviderStreamTaskGuard::for_task(&task);
        assert_eq!(activity_probe.entered(), 1);
        assert_eq!(activity_probe.active(), 1);
        drop(guard);
        assert_eq!(activity_probe.active(), 0);
    }

    #[tokio::test]
    async fn activation_context_across_suspend_keeps_explicit_provider_then_restores_receiver() {
        let receiver = activation("receiver", "receiver-build");
        let provider = activation("provider", "provider-build");
        let receiver_request = RequestActivationContext::begin(Arc::clone(&receiver)).unwrap();
        let provider_request = receiver_request.switch_to(Arc::clone(&provider)).unwrap();
        let generation = receiver_request.generation();

        let resumed = tokio::spawn(async move {
            assert!(Arc::ptr_eq(provider_request.current(), &provider));
            tokio::task::yield_now().await;
            assert!(Arc::ptr_eq(provider_request.current(), &provider));
            provider_request.restore_receiver()
        })
        .await
        .expect("owned provider continuation should not panic");

        assert!(Arc::ptr_eq(resumed.current(), &receiver));
        assert_eq!(resumed.generation(), generation);
    }

    #[test]
    fn in_process_stream_items_use_canonical_detached_plan() {
        let ty = ContractTypeRef::Record {
            fields: BTreeMap::from([
                (
                    "first".to_string(),
                    ContractTypeRef::Builtin {
                        name: "Array".to_string(),
                        arguments: vec![ContractTypeRef::builtin("string")],
                    },
                ),
                (
                    "second".to_string(),
                    ContractTypeRef::Builtin {
                        name: "Array".to_string(),
                        arguments: vec![ContractTypeRef::builtin("string")],
                    },
                ),
            ]),
        };
        let plan = BoundaryValuePlan::Linkable {
            carrier: skiff_artifact_model::BoundaryValueCarrier::DetachedValueGraph,
            encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut provider_heap = RequestHeap::default();
        let shared = provider_heap
            .alloc_array(vec![RuntimeValue::String("provider".to_string())])
            .unwrap();
        let root = provider_heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("first".to_string(), RuntimeValue::Heap(shared)),
                ("second".to_string(), RuntimeValue::Heap(shared)),
            ])))
            .unwrap();
        let mut receiver_heap = RequestHeap::default();

        let item = materialize_value(
            "stream item",
            &ty,
            &BTreeMap::new(),
            &plan,
            &RuntimeValue::Heap(root),
            &provider_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .unwrap();
        let RuntimeValue::Heap(root) = item else {
            panic!("materialized stream item should remain a record")
        };
        let HeapNode::Object(object) = receiver_heap.get(root).unwrap() else {
            panic!("materialized stream item should remain an object")
        };
        let RuntimeValue::Heap(first) = object.fields()["first"] else {
            panic!("first stream field should be an array")
        };
        let RuntimeValue::Heap(second) = object.fields()["second"] else {
            panic!("second stream field should be an array")
        };
        assert_ne!(first, second, "stream item aliases must be detached");
        receiver_heap
            .set_array_item(first, 0, RuntimeValue::String("receiver".to_string()))
            .unwrap();
        let HeapNode::Array(provider_items) = provider_heap.get(shared).unwrap() else {
            panic!("provider array should remain allocated")
        };
        assert_eq!(
            provider_items,
            &[RuntimeValue::String("provider".to_string())],
            "consumer mutation must not reach the provider heap"
        );
    }

    #[test]
    fn in_process_stream_typed_emit_decodes_before_canonical_plan() {
        let execution_plan =
            RuntimeTypePlan::synthetic_named_builtin("Date", RuntimeTypeNode::Date, Vec::new());
        let mut provider_heap = RequestHeap::default();
        let wire = runtime_to_wire_required_plan(
            &RuntimeValue::Date(1_234),
            Some(&execution_plan),
            "provider stream emit item",
            &mut provider_heap,
        )
        .unwrap();
        let mut decoded_heap = RequestHeap::default();
        let decoded = runtime_from_wire_required_plan(
            &wire,
            Some(&execution_plan),
            "provider stream emit item",
            &mut decoded_heap,
        )
        .unwrap();
        let plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut receiver_heap = RequestHeap::default();

        let materialized = materialize_value(
            "stream item",
            &ContractTypeRef::builtin("Date"),
            &BTreeMap::new(),
            &plan,
            &decoded,
            &decoded_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .unwrap();

        assert_eq!(materialized, RuntimeValue::Date(1_234));
    }

    #[test]
    fn in_process_stream_named_item_uses_admitted_package_record() {
        let type_id = PackageSchemaTypeId::new("schema:stream-item");
        let ty =
            ContractTypeRef::package_schema("example.stream", "api.StreamItem", type_id.clone());
        let schema = BTreeMap::from([(
            type_id.clone(),
            Arc::new(PackageSchemaTypeRecord {
                package_id: "example.stream".to_string(),
                stable_schema_key: "api.StreamItem".to_string(),
                package_schema_type_id: type_id,
                canonical_descriptor: PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: skiff_artifact_model::ContractTypeDescriptor::Record {
                        fields: BTreeMap::new(),
                    },
                },
            }),
        )]);
        let plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut provider_heap = RequestHeap::default();
        let source = provider_heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::new()))
            .unwrap();
        let mut receiver_heap = RequestHeap::default();

        materialize_value(
            "stream item",
            &ty,
            &schema,
            &plan,
            &RuntimeValue::Heap(source),
            &provider_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .expect("admitted Package stream item should materialize");

        assert!(materialize_value(
            "stream item",
            &ty,
            &BTreeMap::new(),
            &plan,
            &RuntimeValue::Heap(source),
            &provider_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .is_err());
    }

    pub(super) fn activation(service: &str, package_build: &str) -> Arc<ActivationContext> {
        ActivationContext::new(
            ActivationIdentity {
                assembly_identity: AssemblyIdentity::new("assembly:async-stream"),
                assembly_generation: 7,
                runtime_replica_id: "replica:async-stream".to_string(),
                deployment: ServiceDeploymentRef {
                    service_id: service.to_string(),
                    contract_version: "1.0.0".to_string(),
                    deployment_revision: DeploymentRevision::new(format!("{service}-r1")),
                    deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                        "deployment:{service}"
                    )),
                },
            },
            PackageBuildId::new(package_build),
            ActivationOwnedBindings {
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                policy: DeploymentPolicy {
                    timeout_ms: Some(1_000),
                    resources: ResourcePolicy {
                        cpu_millis: 100,
                        memory_bytes: 1_024,
                    },
                    activation: ActivationPolicy {
                        max_concurrency: 1,
                        idle_timeout_ms: None,
                    },
                    principal: "test".to_string(),
                },
            },
            Vec::new(),
        )
        .unwrap()
    }
}
