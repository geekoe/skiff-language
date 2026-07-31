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

use crate::{
    ExecutionBudget, ExecutionControl, RequestError, RequestResult,
    RuntimeAssemblyWebSocketConnectTarget, RuntimeWebSocketConnectIngress,
};

pub struct RuntimeWebSocketConnectExecutionInput {
    pub target: RuntimeAssemblyWebSocketConnectTarget,
    pub request: RuntimeWebSocketConnectIngress,
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
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    validate_request(&target, &request)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(RequestError::Cancelled);
    }
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    let interpreter = if request.test_effects_enabled {
        Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
            Default::default(),
            handles.eval_adapter.runtime_factory(),
        )
    } else {
        Interpreter::for_runtime_assembly(handles.eval_adapter.runtime_factory())
    };
    let context = handles.eval_adapter.execution_context(
        RuntimeWebSocketConnectEvalExecutionInputParts {
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits,
        },
        &interpreter,
        target.eval(),
    );
    let eval_request = eval_request(&request);
    let body_result = interpreter
        .execute_runtime_websocket_connect(context, &eval_request, &target)
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
    request: &RuntimeWebSocketConnectIngress,
) -> RequestResult<()> {
    validate_request_facts(
        RuntimeWebSocketConnectRequestTargetFacts {
            gateway_entry_key: target.gateway_entry_key(),
            selector: target.selector(),
            assembly_identity: target.eval().execution_image().assembly_identity(),
            assembly_generation: target
                .eval()
                .activation_context()
                .identity()
                .assembly_generation,
            deployment: target.owner(),
            gateway_entry_identity: target.gateway_entry_identity(),
            websocket_entry_id: target.websocket_entry_id(),
        },
        request,
    )
}

struct RuntimeWebSocketConnectRequestTargetFacts<'a> {
    gateway_entry_key: &'a skiff_artifact_model::GatewayEntryKey,
    selector: &'a skiff_artifact_model::IngressSelector,
    assembly_identity: &'a skiff_artifact_model::AssemblyIdentity,
    assembly_generation: u64,
    deployment: &'a skiff_artifact_model::ServiceDeploymentRef,
    gateway_entry_identity: &'a skiff_artifact_model::GatewayEntryIdentity,
    websocket_entry_id: &'a skiff_artifact_model::WebSocketEntryId,
}

fn validate_request_facts(
    target: RuntimeWebSocketConnectRequestTargetFacts<'_>,
    request: &RuntimeWebSocketConnectIngress,
) -> RequestResult<()> {
    let selector = target.selector;
    if selector.protocol != IngressProtocol::WebSocket
        || selector.method.is_some()
        || selector.path != request.ingress_path
        || request.pin.assembly_identity != *target.assembly_identity
        || request.pin.assembly_generation != target.assembly_generation
        || &request.pin.deployment != target.deployment
        || request.pin.gateway_entry_identity != *target.gateway_entry_identity
        || request.connect_gateway_entry_identity != *target.gateway_entry_identity
        || request.websocket_entry_id != *target.websocket_entry_id
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key.as_str(),
            "WebSocket connect routing does not match the exact pinned activation entry",
        ));
    }
    Ok(())
}

fn eval_request(request: &RuntimeWebSocketConnectIngress) -> RuntimeWebSocketConnectRequest {
    RuntimeWebSocketConnectRequest {
        connection_id: request.connection_id.clone(),
        url: request.url.clone(),
        query: name_values(&request.query),
        headers: name_values(&request.headers),
        cookies: name_values(&request.cookies),
        version: request.version.clone(),
        websocket_entry_id: request.websocket_entry_id.clone(),
        gateway_entry_identity: request.connect_gateway_entry_identity.clone(),
    }
}

fn name_values(values: &[crate::HttpNameValue]) -> Vec<RuntimeWebSocketNameValue> {
    values
        .iter()
        .map(|value| RuntimeWebSocketNameValue {
            name: value.name.clone(),
            value: value.value.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
