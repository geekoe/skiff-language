use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use skiff_artifact_model::IngressProtocol;
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    capabilities::EvalRuntimeFactory, program_execution::ProgramExecutionContext, Interpreter,
    RuntimeAssemblyEvalTarget, RuntimeWebSocketConnectRequest, RuntimeWebSocketConnectResult,
    RuntimeWebSocketNameValue,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_runtime_transport::{
    protocol::RUNTIME_FRAME_SCHEMA_VERSION,
    runtime_assembly_request::RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
};

use crate::{
    ExecutionBudget, ExecutionControl, RequestError, RequestResult,
    RuntimeAssemblyWebSocketConnectTarget,
};

pub struct RuntimeWebSocketConnectExecutionInput {
    pub target: RuntimeAssemblyWebSocketConnectTarget,
    pub header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: RuntimeWebSocketConnectExecutionHandles,
}

pub struct RuntimeWebSocketConnectExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub eval_adapter: Arc<dyn RuntimeWebSocketConnectEvalAdapter>,
}

pub trait RuntimeWebSocketConnectEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketConnectEvalExecutionInputParts<'a>,
        interpreter: &'a Interpreter,
        eval_target: &'a RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a>;
}

pub struct RuntimeWebSocketConnectEvalExecutionInputParts<'a> {
    pub header: &'a RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    pub execution: ExecutionControl<'a>,
    pub cancellation: CancellationToken,
    pub cancelled: &'a AtomicBool,
    pub execution_budget: Arc<ExecutionBudget>,
    pub request_heap_limits: RequestHeapLimits,
}

pub async fn execute_runtime_websocket_connect(
    input: RuntimeWebSocketConnectExecutionInput,
) -> RequestResult<RuntimeWebSocketConnectResult> {
    let RuntimeWebSocketConnectExecutionInput {
        target,
        header,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    validate_request(&target, &header)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(RequestError::Cancelled);
    }
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    let interpreter = if header.test_effects_enabled {
        Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
            Default::default(),
            handles.eval_adapter.runtime_factory(),
        )
    } else {
        Interpreter::for_runtime_assembly(handles.eval_adapter.runtime_factory())
    };
    let context = handles.eval_adapter.execution_context(
        RuntimeWebSocketConnectEvalExecutionInputParts {
            header: &header,
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits,
        },
        &interpreter,
        target.eval(),
    );
    let request = eval_request(&header);
    let body_result = interpreter
        .execute_runtime_websocket_connect(context, &request, &target)
        .await
        .map_err(RequestError::from);
    let finalization_result = interpreter.finalize_test_case().map_err(RequestError::from);
    match (body_result, finalization_result) {
        (Err(body_error), _) => Err(body_error),
        (Ok(_), Err(finalization_error)) => Err(finalization_error),
        (Ok(response), Ok(())) => Ok(response),
    }
}

fn validate_request(
    target: &RuntimeAssemblyWebSocketConnectTarget,
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
) -> RequestResult<()> {
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION
        || header.frame_type != "request.start"
        || header.mode != "unary"
        || header.caller.kind != "gateway"
        || header.routing.kind != "runtimeAssembly"
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key().as_str(),
            "WebSocket connect request is not the canonical runtimeAssembly request.start shape",
        ));
    }
    let selector = target.selector();
    if selector.protocol != IngressProtocol::WebSocket
        || selector.host != header.routing.ingress.host
        || selector.method.is_some()
        || selector.path != header.routing.ingress.path
        || header.routing.assembly_identity != *target.eval().execution_image().assembly_identity()
        || header.routing.assembly_generation
            != target
                .eval()
                .activation_context()
                .identity()
                .assembly_generation
        || header.routing.gateway_entry_identity != *target.gateway_entry_identity()
        || header.websocket_connect.gateway_entry_identity != *target.gateway_entry_identity()
        || header.websocket_connect.websocket_entry_id != *target.websocket_entry_id()
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key().as_str(),
            "WebSocket connect routing does not match the exact pinned activation entry",
        ));
    }
    Ok(())
}

fn eval_request(
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
) -> RuntimeWebSocketConnectRequest {
    let request = &header.websocket_connect;
    RuntimeWebSocketConnectRequest {
        connection_id: request.connection_id.clone(),
        url: request.url.clone(),
        query: name_values(&request.query),
        headers: name_values(&request.headers),
        cookies: name_values(&request.cookies),
        version: request.version.clone(),
        websocket_entry_id: request.websocket_entry_id.clone(),
        gateway_entry_identity: request.gateway_entry_identity.clone(),
    }
}

fn name_values(
    values: &[skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestNameValueFrameHeader],
) -> Vec<RuntimeWebSocketNameValue> {
    values
        .iter()
        .map(|value| RuntimeWebSocketNameValue {
            name: value.name.clone(),
            value: value.value.clone(),
        })
        .collect()
}
