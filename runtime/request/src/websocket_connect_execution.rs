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
            gateway_entry_identity: target.gateway_entry_identity(),
            websocket_entry_id: target.websocket_entry_id(),
        },
        header,
    )
}

struct RuntimeWebSocketConnectRequestTargetFacts<'a> {
    gateway_entry_key: &'a skiff_artifact_model::GatewayEntryKey,
    selector: &'a skiff_artifact_model::IngressSelector,
    assembly_identity: &'a skiff_artifact_model::AssemblyIdentity,
    assembly_generation: u64,
    gateway_entry_identity: &'a skiff_artifact_model::GatewayEntryIdentity,
    websocket_entry_id: &'a skiff_artifact_model::WebSocketEntryId,
}

fn validate_request_facts(
    target: RuntimeWebSocketConnectRequestTargetFacts<'_>,
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
) -> RequestResult<()> {
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION
        || header.frame_type != "request.start"
        || header.mode != "unary"
        || header.caller.kind != "gateway"
        || header.routing.kind != "runtimeAssembly"
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key.as_str(),
            "WebSocket connect request is not the canonical runtimeAssembly request.start shape",
        ));
    }
    let selector = target.selector;
    if selector.protocol != IngressProtocol::WebSocket
        || selector.host != header.routing.ingress.host
        || selector.method.is_some()
        || selector.path != header.routing.ingress.path
        || header.routing.assembly_identity != *target.assembly_identity
        || header.routing.assembly_generation != target.assembly_generation
        || header.routing.gateway_entry_identity != *target.gateway_entry_identity
        || header.websocket_connect.gateway_entry_identity != *target.gateway_entry_identity
        || header.websocket_connect.websocket_entry_id != *target.websocket_entry_id
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key.as_str(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        AssemblyIdentity, GatewayEntryIdentity, GatewayEntryKey, IngressSelector, WebSocketEntryId,
        WEBSOCKET_GATEWAY_ENTRY_KEY,
    };
    use skiff_runtime_transport::runtime_assembly_request::{
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressProtocol,
        RuntimeAssemblyWebSocketConnectRequestFrameHeader,
        RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
    };

    struct Fixture {
        key: GatewayEntryKey,
        selector: IngressSelector,
        assembly: AssemblyIdentity,
        gateway_identity: GatewayEntryIdentity,
        websocket_entry_id: WebSocketEntryId,
        header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    }

    impl Fixture {
        fn new() -> Self {
            let assembly = AssemblyIdentity::new("assembly:websocket");
            let gateway_identity = GatewayEntryIdentity::parse(format!(
                "skiff-gateway-entry-v2:sha256:{}",
                "1".repeat(64)
            ))
            .unwrap();
            let websocket_entry_id = WebSocketEntryId::parse(format!(
                "skiff-websocket-entry-v1:sha256:{}",
                "2".repeat(64)
            ))
            .unwrap();
            let selector = IngressSelector {
                protocol: IngressProtocol::WebSocket,
                host: "websocket.test".to_string(),
                method: None,
                path: "/connect".to_string(),
            };
            let header = RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                frame_type: "request.start".to_string(),
                request_id: "request-1".to_string(),
                mode: "unary".to_string(),
                caller: RuntimeAssemblyRequestCallerFrameHeader {
                    kind: "gateway".to_string(),
                },
                routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
                    kind: "runtimeAssembly".to_string(),
                    assembly_identity: assembly.clone(),
                    assembly_generation: 7,
                    gateway_entry_identity: gateway_identity.clone(),
                    ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader {
                        protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                        host: selector.host.clone(),
                        method: (),
                        path: selector.path.clone(),
                    },
                },
                client_session: None,
                deadline: None,
                trace: RuntimeAssemblyRequestTraceFrameHeader {
                    trace_id: "trace-1".to_string(),
                    span_id: "span-1".to_string(),
                    parent_span_id: None,
                    sampled: None,
                },
                websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader {
                    connection_id: "connection-1".to_string(),
                    url: "wss://websocket.test/connect".to_string(),
                    query: Vec::new(),
                    headers: Vec::new(),
                    cookies: Vec::new(),
                    version: None,
                    websocket_entry_id: websocket_entry_id.clone(),
                    gateway_entry_identity: gateway_identity.clone(),
                },
                test_effects_enabled: false,
            };
            Self {
                key: GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap(),
                selector,
                assembly,
                gateway_identity,
                websocket_entry_id,
                header,
            }
        }

        fn facts(&self) -> RuntimeWebSocketConnectRequestTargetFacts<'_> {
            RuntimeWebSocketConnectRequestTargetFacts {
                gateway_entry_key: &self.key,
                selector: &self.selector,
                assembly_identity: &self.assembly,
                assembly_generation: 7,
                gateway_entry_identity: &self.gateway_identity,
                websocket_entry_id: &self.websocket_entry_id,
            }
        }
    }

    #[test]
    fn websocket_connect_request_header_matches_exact_activation_entry() {
        let fixture = Fixture::new();
        validate_request_facts(fixture.facts(), &fixture.header)
            .expect("exact request facts should validate");
    }

    #[test]
    fn websocket_connect_request_rejects_header_activation_and_generation_mismatches() {
        let fixture = Fixture::new();
        let mut mutations = Vec::new();

        let mut wrong_routing_identity = fixture.header.clone();
        wrong_routing_identity.routing.gateway_entry_identity = GatewayEntryIdentity::parse(
            format!("skiff-gateway-entry-v2:sha256:{}", "3".repeat(64)),
        )
        .unwrap();
        mutations.push(wrong_routing_identity);

        let mut wrong_connect_identity = fixture.header.clone();
        wrong_connect_identity
            .websocket_connect
            .gateway_entry_identity = GatewayEntryIdentity::parse(format!(
            "skiff-gateway-entry-v2:sha256:{}",
            "4".repeat(64)
        ))
        .unwrap();
        mutations.push(wrong_connect_identity);

        let mut wrong_entry_id = fixture.header.clone();
        wrong_entry_id.websocket_connect.websocket_entry_id = WebSocketEntryId::parse(format!(
            "skiff-websocket-entry-v1:sha256:{}",
            "5".repeat(64)
        ))
        .unwrap();
        mutations.push(wrong_entry_id);

        let mut wrong_assembly = fixture.header.clone();
        wrong_assembly.routing.assembly_identity = AssemblyIdentity::new("assembly:other");
        mutations.push(wrong_assembly);

        let mut stale_generation = fixture.header.clone();
        stale_generation.routing.assembly_generation = 6;
        mutations.push(stale_generation);

        let mut wrong_host = fixture.header.clone();
        wrong_host.routing.ingress.host = "other.test".to_string();
        mutations.push(wrong_host);

        for mutation in mutations {
            assert!(validate_request_facts(fixture.facts(), &mutation).is_err());
        }
    }
}
