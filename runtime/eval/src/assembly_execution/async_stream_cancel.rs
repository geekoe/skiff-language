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
    error::{is_deadline_or_scope_terminal, stream_runtime_error_from_eval, Result, RuntimeError},
    eval_context::EvalContext,
    heap_access::HeapAccess,
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
    let provider_args = boundary.materialize_parameters(
        &args,
        context.heap.heap_mut(),
        &mut provider_heap,
        &hooks,
    )?;
    let mut provider_context = provider_execution_context(&context.context, &target)?;
    let provider_stream_item_type = provider_stream_item_execution_plan(
        context.interpreter,
        &provider_context,
        context.addr,
        target.executable_addr(),
        context.env,
        &call.type_args,
    )?;
    let owned_provider = Arc::new(OwnedProgramExecutionContext::capture(&provider_context));
    let provider_stream_runtime_owner = provider_context.take_stream_runtime_owner();
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
    let receiver_value = match runtime_from_wire(&stream_value, context.heap.heap_mut()) {
        Ok(value) => value,
        Err(error) => {
            stream_runtime.cancel(&stream_value);
            return Err(error);
        }
    };

    let producer_env = provider_stream_producer_env(
        context.env,
        sink.clone(),
        // This plan only serializes the provider File-IR value passed to `emit`.
        // BoundaryStreamSink remains the semantic owner and applies the
        // canonical contract plan before publication.
        provider_stream_item_type,
    );
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
        #[cfg(test)]
        depth_probe: None,
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

/// Builds the detached provider-stream task env. Only owned call-site
/// capabilities and type substitutions are copied (same rule as the unary
/// path); caller slots/self may contain heap handles and must never reach the
/// spawned provider task.
fn provider_stream_producer_env(
    caller: &Env,
    sink: StreamSink,
    item_type: Option<RuntimeTypePlan>,
) -> Env {
    let mut producer_env = prepared_unary::detached_provider_invocation_env(caller);
    producer_env.stream_sink = Some(sink);
    producer_env.current_stream_item_type = item_type;
    producer_env
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
    #[cfg(test)]
    depth_probe: Option<Arc<ProviderStreamTaskDepthProbe>>,
}

fn spawn_provider_stream(producer: ProviderStreamTask) {
    tokio::spawn(async move {
        run_provider_stream(producer).await;
    });
}

async fn run_provider_stream(mut producer: ProviderStreamTask) {
    let _active = ProviderStreamTaskGuard::for_task(&producer);
    let args = std::mem::take(&mut producer.args);
    let (terminal, provider_heap) = {
        let provider_context = producer.provider_context.borrow_for_scheduled_task();
        #[cfg(test)]
        if let Some(probe) = &producer.depth_probe {
            probe.record_callable_entry(&provider_context);
        }
        let mut provider_access = HeapAccess::private(std::mem::take(&mut producer.provider_heap));
        let provider_future = call_provider_callable(
            &producer.interpreter,
            provider_context,
            &mut provider_access,
            &producer.provider_env,
            &producer.caller_addr,
            &producer.provider_addr,
            producer.provider_receiver_const.as_ref(),
            &producer.type_args,
            args,
        );
        let terminal = await_provider_stream_terminal(
            &producer.execution,
            &producer.stream_cancel,
            provider_future,
        )
        .await;
        (terminal, provider_access.into_owned_heap())
    };

    producer.provider_heap = provider_heap;
    finish_provider_stream(producer, terminal).await;
}

#[allow(clippy::too_many_arguments)]
fn call_provider_callable<'call, 'ctx>(
    interpreter: &'call crate::Interpreter,
    context: ProgramExecutionContext<'ctx>,
    heap: &'call mut HeapAccess,
    env: &'call Env,
    caller_addr: &'call ExecutableAddr,
    provider_addr: &'call ExecutableAddr,
    receiver_const: Option<&'call ConstAddr>,
    type_args: &'call BTreeMap<String, LinkedTypeRef>,
    args: Vec<RuntimeValue>,
) -> Pin<Box<dyn Future<Output = Result<RuntimeValue>> + Send + 'call>>
where
    'ctx: 'call,
{
    // Keep ordinary service calls on their existing future directly. An `async fn` wrapper here
    // adds another poll stack frame to every provider call, including receiver-free calls.
    let Some(receiver_const) = receiver_const else {
        return Box::pin(interpreter.call_program_executable(
            context,
            heap,
            env,
            caller_addr,
            provider_addr,
            type_args,
            args,
        ));
    };
    Box::pin(async move {
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
    })
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
        ProviderTerminal::Provider(Err(error)) if is_deadline_or_scope_terminal(&error) => {
            publish_provider_deadline_terminal(&producer, error).await;
        }
        ProviderTerminal::Provider(Err(error)) => {
            let provider_context = producer.provider_context.borrow();
            #[cfg(test)]
            if let Some(probe) = &producer.depth_probe {
                probe.record_error_export(&provider_context);
            }
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
    receiver.clone().switch_activation_owner(
        provider_target,
        crate::program_execution::ActivationExecutionOperation::service_call(
            target.descriptor().operation_id.clone(),
        ),
    )
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

#[cfg(test)]
struct ProviderStreamTaskDepthProbe {
    callable_entry: AtomicUsize,
    error_export: AtomicUsize,
}

#[cfg(test)]
impl Default for ProviderStreamTaskDepthProbe {
    fn default() -> Self {
        Self {
            callable_entry: AtomicUsize::new(usize::MAX),
            error_export: AtomicUsize::new(usize::MAX),
        }
    }
}

#[cfg(test)]
impl ProviderStreamTaskDepthProbe {
    fn record_callable_entry(&self, context: &ProgramExecutionContext<'_>) {
        self.callable_entry
            .store(context.program_call_depth_for_test(), Ordering::Release);
    }

    fn record_error_export(&self, context: &ProgramExecutionContext<'_>) {
        self.error_export
            .store(context.program_call_depth_for_test(), Ordering::Release);
    }

    fn callable_entry(&self) -> Option<usize> {
        let depth = self.callable_entry.load(Ordering::Acquire);
        (depth != usize::MAX).then_some(depth)
    }

    fn error_export(&self) -> Option<usize> {
        let depth = self.error_export.load(Ordering::Acquire);
        (depth != usize::MAX).then_some(depth)
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
mod tests;
