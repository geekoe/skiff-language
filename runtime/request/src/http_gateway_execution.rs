use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use skiff_artifact_model::{
    AssemblyIdentity, GatewayAdapterKind, GatewayDispatchMode, GatewayEntryIdentity,
    GatewayProtocolSurface,
};
use skiff_runtime_capability_context::{
    BinaryHttpRequestContext, CancellationToken, HttpNameValueContext, RequestPayloadContext,
};
use skiff_runtime_eval::{
    capabilities::EvalRuntimeFactory, error::RuntimeError,
    program_execution::ProgramExecutionContext, Interpreter, RuntimeAssemblyEvalTarget,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_runtime_transport::{
    protocol::RUNTIME_FRAME_SCHEMA_VERSION,
    runtime_assembly_request::RuntimeAssemblyRequestStartFrameHeader,
};

use crate::{
    response_stream_writer::ResponseStreamWriter, BoundaryResponse, ExecutionBudget,
    ExecutionControl, HttpNameValue, HttpResponseMetadata, RequestError, RequestResult,
    ResponseEventSink, RuntimeAssemblyHttpGatewayTarget,
};

pub struct RuntimeHttpGatewayExecutionInput {
    pub target: RuntimeAssemblyHttpGatewayTarget,
    pub header: RuntimeAssemblyRequestStartFrameHeader,
    pub body: Vec<u8>,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: RuntimeHttpGatewayExecutionHandles,
}

pub struct RuntimeHttpGatewayExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub eval_adapter: Arc<dyn RuntimeHttpGatewayEvalAdapter>,
    pub response_events: Arc<dyn ResponseEventSink>,
}

/// Host capability-context builder for the already-validated gateway request.
///
/// The adapter receives no callable/schema/adapter facts to interpret; those remain inside the
/// exact target and eval execution seam.
pub trait RuntimeHttpGatewayEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeHttpGatewayEvalExecutionInputParts<'a>,
        request_context: RequestPayloadContext<'a>,
        interpreter: &'a Interpreter,
        eval_target: &'a RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a>;
}

pub struct RuntimeHttpGatewayEvalExecutionInputParts<'a> {
    pub header: &'a RuntimeAssemblyRequestStartFrameHeader,
    pub execution: ExecutionControl<'a>,
    pub cancellation: CancellationToken,
    pub cancelled: &'a AtomicBool,
    pub execution_budget: Arc<ExecutionBudget>,
    pub request_heap_limits: RequestHeapLimits,
}

pub async fn execute_runtime_http_gateway_request(
    input: RuntimeHttpGatewayExecutionInput,
) -> RequestResult<BoundaryResponse> {
    let RuntimeHttpGatewayExecutionInput {
        target,
        header,
        body,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    let lifecycle = RuntimeHttpGatewayRequestLifecycle::new(target, Arc::clone(&cancelled));
    let target = lifecycle.target();
    validate_request(target, &header)?;
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
    let request_context = request_context(target, &header, &body);
    let context = handles.eval_adapter.execution_context(
        RuntimeHttpGatewayEvalExecutionInputParts {
            header: &header,
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits,
        },
        request_context.clone(),
        &interpreter,
        target.eval(),
    );
    let GatewayProtocolSurface::Http(http) = &target.protocol_surface().protocol else {
        return Err(RequestError::protocol(
            target.gateway_entry_key().as_str(),
            "HTTP gateway execution requires an HTTP protocol surface",
        ));
    };
    let body_result = match http.dispatch_mode {
        GatewayDispatchMode::Unary => interpreter
            .execute_runtime_http_gateway_unary(context, request_context, target)
            .await
            .map(|response| {
                BoundaryResponse::http(
                    response.body,
                    HttpResponseMetadata::new(
                        response.status,
                        response
                            .headers
                            .into_iter()
                            .map(|header| HttpNameValue {
                                name: header.name,
                                value: header.value,
                            })
                            .collect(),
                    ),
                )
            })
            .map_err(RequestError::from),
        GatewayDispatchMode::ServerStream => {
            let mut writer = ResponseStreamWriter::new(
                header.request_id.clone(),
                Arc::clone(&handles.response_events),
            );
            interpreter
                .execute_runtime_http_gateway_server_stream(
                    context,
                    request_context,
                    target,
                    |event| {
                        writer.send_binary_http_event(event).map_err(|error| {
                            RuntimeError::Protocol {
                                target: target.gateway_entry_key().as_str().to_string(),
                                message: error.to_string(),
                            }
                        })
                    },
                )
                .await
                .map_err(RequestError::from)
                .and_then(|()| writer.require_exact_http_terminal())
                .map(|()| BoundaryResponse::StreamSent)
        }
    };
    let finalization_result = interpreter.finalize_test_case().map_err(RequestError::from);
    match (body_result, finalization_result) {
        (Err(body_error), _) => Err(body_error),
        (Ok(_), Err(finalization_error)) => Err(finalization_error),
        (Ok(response), Ok(())) => Ok(response),
    }
}

fn validate_request(
    target: &RuntimeAssemblyHttpGatewayTarget,
    header: &RuntimeAssemblyRequestStartFrameHeader,
) -> RequestResult<()> {
    let GatewayProtocolSurface::Http(http) = &target.protocol_surface().protocol else {
        return Err(RequestError::protocol(
            target.gateway_entry_key().as_str(),
            "HTTP gateway request validation requires an HTTP protocol surface",
        ));
    };
    validate_request_facts(
        HttpGatewayRequestValidationFacts {
            gateway_entry_key: target.gateway_entry_key().as_str(),
            assembly_identity: target.eval().execution_image().assembly_identity(),
            assembly_generation: target
                .eval()
                .activation_context()
                .identity()
                .assembly_generation,
            deployment: target.owner(),
            gateway_entry_identity: target.gateway_entry_identity(),
            dispatch_mode: http.dispatch_mode,
            surface_adapter_kind: http.adapter_kind,
            plan_adapter_kind: target.adapter_plan().kind,
        },
        header,
    )
}

struct HttpGatewayRequestValidationFacts<'a> {
    gateway_entry_key: &'a str,
    assembly_identity: &'a AssemblyIdentity,
    assembly_generation: u64,
    deployment: &'a skiff_artifact_model::ServiceDeploymentRef,
    gateway_entry_identity: &'a GatewayEntryIdentity,
    dispatch_mode: GatewayDispatchMode,
    surface_adapter_kind: GatewayAdapterKind,
    plan_adapter_kind: GatewayAdapterKind,
}

fn validate_request_facts(
    target: HttpGatewayRequestValidationFacts<'_>,
    header: &RuntimeAssemblyRequestStartFrameHeader,
) -> RequestResult<()> {
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION
        || header.frame_type != "request.start"
        || header.caller.kind != "gateway"
        || header.routing.kind != "runtimeAssembly"
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway request is not the canonical runtimeAssembly request.start shape",
        ));
    }
    if header.routing.assembly_identity != *target.assembly_identity
        || header.routing.assembly_generation != target.assembly_generation
        || &header.routing.deployment != target.deployment
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway request does not match the pinned assembly activation",
        ));
    }
    if header.routing.gateway_entry_identity != *target.gateway_entry_identity {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway request identity does not match the exact linked entry",
        ));
    }
    if header.routing.ingress.method != header.http_request.method
        || header.routing.ingress.path != header.http_request.path
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway routing metadata and binary HTTP context disagree",
        ));
    }
    let expected_mode = match target.dispatch_mode {
        GatewayDispatchMode::Unary => "unary",
        GatewayDispatchMode::ServerStream => "serverStream",
    };
    if header.mode != expected_mode
        || (target.surface_adapter_kind == GatewayAdapterKind::TypedJson
            && target.dispatch_mode != GatewayDispatchMode::Unary)
        || target.plan_adapter_kind != target.surface_adapter_kind
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway request mode does not match the exact linked adapter plan",
        ));
    }
    Ok(())
}

fn request_context<'a>(
    target: &'a RuntimeAssemblyHttpGatewayTarget,
    header: &'a RuntimeAssemblyRequestStartFrameHeader,
    body: &'a [u8],
) -> RequestPayloadContext<'a> {
    RequestPayloadContext::new(
        target.gateway_entry_key().as_str(),
        body,
        Some(BinaryHttpRequestContext::new(
            header.http_request.method.as_str(),
            header.http_request.url.as_str(),
            header.http_request.path.as_str(),
            header
                .http_request
                .query
                .iter()
                .map(|item| HttpNameValueContext::new(&item.name, &item.value))
                .collect(),
            header
                .http_request
                .headers
                .iter()
                .map(|item| HttpNameValueContext::new(&item.name, &item.value))
                .collect(),
            body,
        )),
    )
}

struct RuntimeHttpGatewayRequestLifecycle {
    target: RuntimeAssemblyHttpGatewayTarget,
    cancelled: Arc<AtomicBool>,
}

impl RuntimeHttpGatewayRequestLifecycle {
    fn new(target: RuntimeAssemblyHttpGatewayTarget, cancelled: Arc<AtomicBool>) -> Self {
        Self { target, cancelled }
    }

    fn target(&self) -> &RuntimeAssemblyHttpGatewayTarget {
        &self.target
    }
}

impl Drop for RuntimeHttpGatewayRequestLifecycle {
    fn drop(&mut self) {
        let request_activation = self.target.eval().request_activation();
        if self.cancelled.load(Ordering::Acquire) {
            request_activation.cancel();
        }
        request_activation.end_request();
    }
}

#[cfg(test)]
#[path = "http_gateway_execution/tests.rs"]
mod tests;
