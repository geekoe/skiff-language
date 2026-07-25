use std::sync::{atomic::AtomicBool, Arc};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    dispatch_ingress_via_in_process_boundary, dispatch_websocket_ingress_via_in_process_boundary,
    InProcessBoundaryIngressResponse, Interpreter, TestEffectDouble,
};

mod websocket_request;

use crate::{
    invocation_builder::eval_websocket_adapter, request_payload_context_from_request,
    websocket_ingress::boundary_response_from_eval_websocket_adapter_result,
    AssemblyRequestEvalAdapter, BoundaryResponse, ExecutionBudget, ExecutionControl, HttpNameValue,
    HttpResponseMetadata, RequestEnvelope, RequestError, RequestEvalExecutionInputParts,
    RequestResult, RuntimeAssemblyRequestTarget, RuntimeOperation,
};

pub struct AssemblyRequestExecutionInput {
    pub target: RuntimeAssemblyRequestTarget,
    pub request: RequestEnvelope,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: AssemblyRequestExecutionHandles,
}

pub struct AssemblyRequestExecutionHandles {
    pub request_heap_limits: skiff_runtime_model::request_heap::RequestHeapLimits,
    pub eval_adapter: Arc<dyn AssemblyRequestEvalAdapter>,
}

/// Executes a canonical ingress through the production in-process boundary dispatcher.
pub async fn execute_runtime_assembly_request(
    input: AssemblyRequestExecutionInput,
) -> RequestResult<BoundaryResponse> {
    let AssemblyRequestExecutionInput {
        target,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    let lifecycle = AssemblyRequestLifecycle::new(target, Arc::clone(&cancelled));
    let target = lifecycle.target();
    validate_assembly_ingress_request(&request)?;
    target
        .ensure_execution_ready()
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    let adapter = &handles.eval_adapter;
    let interpreter = if request.test_effects_enabled || !request.test_effect_doubles.is_empty() {
        Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
            request
                .test_effect_doubles
                .iter()
                .map(|(target, sequence)| {
                    (
                        target.clone(),
                        sequence
                            .iter()
                            .map(|double| TestEffectDouble {
                                expect_request: double.expect_request.clone(),
                                response: double.response.clone(),
                            })
                            .collect(),
                    )
                })
                .collect(),
            adapter.runtime_factory(),
        )
    } else {
        Interpreter::for_runtime_assembly(adapter.runtime_factory())
    };
    let operation = canonical_runtime_operation(target, &request);
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    let request_context = request_payload_context_from_request(&request);
    let websocket_adapter = request
        .websocket_adapter
        .as_ref()
        .map(eval_websocket_adapter);
    let websocket_phase = request
        .websocket_adapter
        .as_ref()
        .map(|adapter| adapter.kind);
    let websocket_identity = request
        .websocket_adapter
        .as_ref()
        .map(|_| websocket_request::admitted_identity(&request))
        .transpose()?;
    let context = adapter.execution_context(
        RequestEvalExecutionInputParts {
            operation: &operation,
            request: &request,
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits.clone(),
        },
        request_context.clone(),
        &interpreter,
        target,
    );
    let mut heap = context.request_heap();
    if let Some(websocket_adapter) = websocket_adapter.as_ref() {
        let result = dispatch_websocket_ingress_via_in_process_boundary(
            &interpreter,
            context,
            &mut heap,
            target.boundary().clone(),
            &request_context,
            websocket_adapter,
            websocket_identity
                .as_ref()
                .expect("WebSocket admitted identity checked above"),
        )
        .await
        .map_err(RequestError::from)?;
        interpreter
            .ensure_test_effects_consumed()
            .map_err(RequestError::from)?;
        return boundary_response_from_eval_websocket_adapter_result(
            websocket_phase.expect("WebSocket adapter phase checked above"),
            result,
        );
    }
    let result = dispatch_ingress_via_in_process_boundary(
        &interpreter,
        context,
        &mut heap,
        target.boundary().clone(),
        &request_context,
    )
    .await;
    let result = result.map_err(RequestError::from)?;
    interpreter
        .ensure_test_effects_consumed()
        .map_err(RequestError::from)?;
    match result {
        InProcessBoundaryIngressResponse::RuntimePayload(payload) => {
            Ok(BoundaryResponse::payload(payload))
        }
        InProcessBoundaryIngressResponse::BinaryHttp(response) => Ok(BoundaryResponse::http(
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
        )),
    }
}

struct AssemblyRequestLifecycle {
    target: RuntimeAssemblyRequestTarget,
    cancelled: Arc<AtomicBool>,
}

impl AssemblyRequestLifecycle {
    fn new(target: RuntimeAssemblyRequestTarget, cancelled: Arc<AtomicBool>) -> Self {
        Self { target, cancelled }
    }

    fn target(&self) -> &RuntimeAssemblyRequestTarget {
        &self.target
    }
}

impl Drop for AssemblyRequestLifecycle {
    fn drop(&mut self) {
        let request_activation = self.target.eval().request_activation();
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            request_activation.cancel();
        }
        request_activation.end_request();
    }
}

fn validate_assembly_ingress_request(request: &RequestEnvelope) -> RequestResult<()> {
    if request.ingress_selector.is_none() {
        return Err(RequestError::Unsupported(
            "request.start canonical ingress selector is required".to_string(),
        ));
    }
    if request.mode != "unary" {
        return Err(RequestError::Unsupported(format!(
            "canonical assembly ingress only supports unary request.start, got {}",
            request.mode
        )));
    }
    if request.http_adapter.is_some() {
        return Err(RequestError::Unsupported(
            "legacy HTTP callable adapter metadata is not accepted by canonical assembly ingress"
                .to_string(),
        ));
    }
    let selector = request
        .ingress_selector
        .as_ref()
        .expect("canonical selector checked above");
    match selector.protocol {
        skiff_artifact_model::IngressProtocol::Http => {
            if request.websocket_adapter.is_some() {
                return Err(RequestError::Unsupported(
                    "canonical HTTP ingress does not accept WebSocket metadata".to_string(),
                ));
            }
            if request.extra.contains_key("websocketEntryId") {
                return Err(RequestError::Unsupported(
                    "canonical HTTP ingress does not accept websocketEntryId".to_string(),
                ));
            }
        }
        skiff_artifact_model::IngressProtocol::WebSocket => {
            if request.websocket_adapter.is_none() || request.binary_http.is_some() {
                return Err(RequestError::Unsupported(
                    "canonical WebSocket ingress requires only WebSocket metadata".to_string(),
                ));
            }
            websocket_request::validate(request)?;
        }
    }
    Ok(())
}

fn canonical_runtime_operation(
    target: &RuntimeAssemblyRequestTarget,
    request: &RequestEnvelope,
) -> RuntimeOperation {
    let descriptor = target.boundary().descriptor();
    RuntimeOperation {
        operation_abi_id: None,
        operation: descriptor.operation_id.as_str().to_string(),
        target: descriptor.operation_id.as_str().to_string(),
        mode: request.mode.clone(),
        parameters: Vec::new(),
        service_protocol_identity: Some(
            target
                .boundary()
                .contract()
                .service_protocol_identity
                .as_str()
                .to_string(),
        ),
        extra: serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use skiff_artifact_model::{IngressProtocol, IngressSelector};

    use super::validate_assembly_ingress_request;
    use crate::{
        GatewayAdapterArg, GatewayAdapterSource, RequestEnvelope, WebSocketAdapter,
        WebSocketAdapterKind, WebSocketConnectRequest, WebSocketContextCodec, WebSocketMessage,
        WebSocketMessageEncoding, WebSocketMessageTag, WebSocketPayloadSegment,
        WebSocketPayloadSegmentKind, WebSocketReceiveRequest,
    };

    #[test]
    fn websocket_ingress_accepts_only_canonical_websocket_phase_metadata() {
        let mut request = canonical_websocket_connect_request();
        assert!(validate_assembly_ingress_request(&request).is_ok());

        request.ingress_selector.as_mut().unwrap().protocol = IngressProtocol::Http;
        let error = validate_assembly_ingress_request(&request)
            .expect_err("HTTP selector must not accept WebSocket phase metadata");
        assert!(error
            .to_string()
            .contains("does not accept WebSocket metadata"));
    }

    #[test]
    fn websocket_ingress_rejects_phase_payload_and_identity_mutations() {
        let mut payload = canonical_websocket_connect_request();
        payload.payload_bytes.push(1);
        assert!(validate_assembly_ingress_request(&payload)
            .expect_err("connect payload bytes must fail closed")
            .to_string()
            .contains("connect payload must be empty"));

        let mut phase = canonical_websocket_connect_request();
        phase.websocket_adapter.as_mut().unwrap().kind = WebSocketAdapterKind::Receive;
        assert!(validate_assembly_ingress_request(&phase)
            .expect_err("connect metadata under receive phase must fail closed")
            .to_string()
            .contains("phase metadata is inconsistent"));

        let mut identity = canonical_websocket_connect_request();
        identity.extra.remove("gatewayEntryIdentity");
        assert!(validate_assembly_ingress_request(&identity)
            .expect_err("missing admitted identity must fail closed")
            .to_string()
            .contains("gatewayEntryIdentity"));
    }

    #[test]
    fn websocket_ingress_preserves_nominal_zero_byte_context_segment() {
        let mut request = canonical_websocket_connect_request();
        request.payload_bytes = b"message".to_vec();
        request.websocket_adapter = Some(WebSocketAdapter {
            kind: WebSocketAdapterKind::Receive,
            adapter_args: vec![event_arg()],
            context_expectation: None,
            connect_request: None,
            receive_request: Some(WebSocketReceiveRequest {
                connection_id: "connection-1".to_string(),
                business_identity: None,
                message: WebSocketMessage {
                    tag: WebSocketMessageTag::Text,
                    encoding: WebSocketMessageEncoding::Utf8,
                },
                context_codec: Some(WebSocketContextCodec {
                    operation_abi_id: "operation-abi".to_string(),
                    context_type_identity: "context-type".to_string(),
                }),
                payload_segments: vec![
                    WebSocketPayloadSegment {
                        kind: WebSocketPayloadSegmentKind::Context,
                        offset: 0,
                        length: 0,
                    },
                    WebSocketPayloadSegment {
                        kind: WebSocketPayloadSegmentKind::Message,
                        offset: 0,
                        length: 7,
                    },
                ],
            }),
        });
        assert!(validate_assembly_ingress_request(&request).is_ok());

        request
            .websocket_adapter
            .as_mut()
            .unwrap()
            .receive_request
            .as_mut()
            .unwrap()
            .payload_segments
            .remove(0);
        assert!(validate_assembly_ingress_request(&request)
            .expect_err("typed Context codec requires a Context payload segment")
            .to_string()
            .contains("Context presence"));
    }

    #[test]
    fn assembly_ingress_ignores_legacy_target_fields_but_requires_canonical_selector() {
        let mut request = request();
        request.build_id = "mutated-build".to_string();
        request.operation_abi_id = Some("mutated-operation-abi".to_string());
        request.selector = Some("mutated-display-selector".to_string());
        request.target = "mutated-display-target".to_string();
        assert!(validate_assembly_ingress_request(&request).is_ok());

        request.ingress_selector = None;
        let error = validate_assembly_ingress_request(&request)
            .expect_err("missing canonical selector must fail closed");
        assert!(error.to_string().contains("canonical ingress selector"));
    }

    #[test]
    fn assembly_ingress_rejects_legacy_callable_adapter_before_dispatch() {
        let mut request = request();
        request.mode = "serverStream".to_string();
        assert!(validate_assembly_ingress_request(&request).is_err());
    }

    fn request() -> RequestEnvelope {
        RequestEnvelope {
            request_id: "assembly-ingress-request".to_string(),
            mode: "unary".to_string(),
            target: "display-only".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: None,
            build_id: "legacy-build".to_string(),
            service_protocol_identity: "legacy-protocol".to_string(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: Some(IngressSelector {
                protocol: IngressProtocol::Http,
                host: "example.test".to_string(),
                method: Some("POST".to_string()),
                path: "/entry".to_string(),
            }),
            binary_http: None,
            http_adapter: None,
            websocket_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    fn canonical_websocket_connect_request() -> RequestEnvelope {
        let mut request = request();
        request.ingress_selector = Some(IngressSelector {
            protocol: IngressProtocol::WebSocket,
            host: "example.test".to_string(),
            method: None,
            path: "/socket".to_string(),
        });
        request.websocket_adapter = Some(WebSocketAdapter {
            kind: WebSocketAdapterKind::Connect,
            adapter_args: vec![event_arg()],
            context_expectation: None,
            connect_request: Some(WebSocketConnectRequest {
                connection_id: "connection-1".to_string(),
                url: "ws://example.test/socket".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
                cookies: Vec::new(),
                version: None,
            }),
            receive_request: None,
        });
        request.extra.insert(
            "websocketEntryId".to_string(),
            serde_json::Value::String("entry-claim".to_string()),
        );
        request.extra.insert(
            "gatewayEntryIdentity".to_string(),
            serde_json::Value::String("gateway-claim".to_string()),
        );
        request
    }

    fn event_arg() -> GatewayAdapterArg {
        GatewayAdapterArg {
            param: "event".to_string(),
            source: GatewayAdapterSource::WebSocketIngressEvent,
        }
    }
}
