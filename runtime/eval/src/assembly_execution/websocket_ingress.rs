use std::collections::BTreeMap;

use serde_json::{json, Value};
use skiff_artifact_model::WebSocketIngressContext;
use skiff_runtime_boundary::{
    payload::{PayloadBoundary, PayloadBoundaryKind},
    value as boundary_bytes,
};
use skiff_runtime_capability_context::RequestPayloadContext;
use skiff_runtime_linked_program::{CallIr, LinkedCallTarget};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

#[path = "websocket_contract_plan.rs"]
mod websocket_contract_plan;
#[cfg(test)]
pub(super) use websocket_contract_plan::test_support as websocket_contract_plan_test_support;
pub(super) use websocket_contract_plan::PinnedWebSocketContractPlan;

use super::{
    dispatch_in_process_boundary,
    websocket_identity::{validate_admitted_identity, AdmittedWebSocketIngressIdentity},
    InProcessBoundaryDispatchOrigin,
};
use crate::{
    env::Env,
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    invocation::EvalWebSocketAdapterResult,
    invocation_builder::{
        EvalInvocationBuildArgFrom, EvalInvocationBuildWebSocketAdapter,
        EvalInvocationBuildWebSocketKind, EvalInvocationBuildWebSocketMessageEncoding,
        EvalInvocationBuildWebSocketMessageTag, EvalInvocationBuildWebSocketPayloadSegmentKind,
    },
    program_execution::ProgramExecutionContext,
    Interpreter, RuntimeAssemblyServiceCallTarget,
};

/// Materializes the unified canonical WebSocket event, enters the production in-process boundary
/// dispatcher, then projects the phase-specific response.
pub async fn dispatch_websocket_ingress_via_in_process_boundary(
    interpreter: &Interpreter,
    context: ProgramExecutionContext<'_>,
    heap: &mut RequestHeap,
    target: RuntimeAssemblyServiceCallTarget,
    request: &RequestPayloadContext<'_>,
    adapter: &EvalInvocationBuildWebSocketAdapter,
    admitted_identity: &AdmittedWebSocketIngressIdentity,
) -> Result<EvalWebSocketAdapterResult> {
    let contract = std::sync::Arc::clone(target.contract());
    let operation_id = target.descriptor().operation_id.clone();
    let pinned_plan = PinnedWebSocketContractPlan::compile(
        &contract,
        &operation_id,
        target.schema_records().as_ref(),
    )?;
    validate_canonical_adapter(adapter, request.target())?;
    validate_admitted_identity(
        contract.service_id.as_str(),
        contract.service_protocol_identity.as_str(),
        pinned_plan.operation_id().as_str(),
        admitted_identity,
        request.target(),
    )?;

    let addr = target.executable_addr().clone();
    let projection = super::RuntimeExecutionProjection::for_context(interpreter, &context)?;
    let resolved = projection.resolve_executable(&addr)?;
    let canonical_addr = resolved.addr.clone();
    pinned_plan.validate_executable(resolved.executable)?;
    let event = canonical_websocket_event_value(adapter, request, &pinned_plan, heap)?;
    let mut env = Env::new();
    let call = CallIr {
        target: LinkedCallTarget::Executable {
            addr: canonical_addr.clone(),
        },
        args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    };
    let value = {
        let mut eval_context = EvalContext::new(
            interpreter,
            context,
            heap,
            &mut env,
            &canonical_addr,
            resolved.file,
            resolved.executable,
        )?;
        dispatch_in_process_boundary(
            &mut eval_context,
            &call,
            target,
            vec![event],
            InProcessBoundaryDispatchOrigin::Ingress,
        )
        .await?
    };
    match adapter.kind {
        EvalInvocationBuildWebSocketKind::Connect => {
            super::websocket_response::project_connect_response(
                request.target(),
                &value,
                &pinned_plan,
                heap,
            )
        }
        EvalInvocationBuildWebSocketKind::Receive => {
            if !matches!(value, RuntimeValue::Null) {
                return Err(RuntimeError::Protocol {
                    target: request.target().to_string(),
                    message: "websocket receive operation must return null".to_string(),
                });
            }
            Ok(EvalWebSocketAdapterResult::Receive)
        }
    }
}

fn validate_canonical_adapter(
    adapter: &EvalInvocationBuildWebSocketAdapter,
    target: &str,
) -> Result<()> {
    let [arg] = adapter.args.as_slice() else {
        return Err(websocket_protocol_error(
            target,
            "canonical WebSocket ingress requires exactly one event adapter argument",
        ));
    };
    if arg.param != "event" || arg.from != EvalInvocationBuildArgFrom::WebSocketIngressEvent {
        return Err(websocket_protocol_error(
            target,
            "canonical WebSocket ingress adapterArgs must be event:websocket.ingressEvent",
        ));
    }
    if adapter.context_expectation.is_some() {
        return Err(websocket_protocol_error(
            target,
            "canonical WebSocket ingress derives Context from the pinned ServiceContract",
        ));
    }
    match (
        adapter.kind,
        adapter.connect_request.is_some(),
        adapter.receive_request.is_some(),
    ) {
        (EvalInvocationBuildWebSocketKind::Connect, true, false)
        | (EvalInvocationBuildWebSocketKind::Receive, false, true) => Ok(()),
        _ => Err(websocket_protocol_error(
            target,
            "canonical WebSocket ingress phase metadata is inconsistent",
        )),
    }
}

fn canonical_websocket_event_value(
    adapter: &EvalInvocationBuildWebSocketAdapter,
    request: &RequestPayloadContext<'_>,
    pinned_plan: &PinnedWebSocketContractPlan<'_>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let wire = match adapter.kind {
        EvalInvocationBuildWebSocketKind::Connect => {
            if !request.payload_bytes().is_empty() {
                return Err(websocket_protocol_error(
                    request.target(),
                    "canonical WebSocket connect payload must be empty",
                ));
            }
            let connect = adapter
                .connect_request
                .as_ref()
                .expect("canonical connect metadata validated");
            json!({
                "tag": "connect",
                "connectRequest": {
                    "connectionId": connect.connection_id,
                    "url": connect.url,
                    "query": websocket_name_values(&connect.query),
                    "headers": websocket_name_values(&connect.headers),
                    "cookies": websocket_name_values(&connect.cookies),
                    "version": connect.version,
                }
            })
        }
        EvalInvocationBuildWebSocketKind::Receive => {
            let receive = adapter
                .receive_request
                .as_ref()
                .expect("canonical receive metadata validated");
            let context_value =
                canonical_receive_context_wire(receive, request, pinned_plan, heap)?;
            let message = canonical_receive_message_wire(receive, request)?;
            json!({
                "tag": "receive",
                "receiveEvent": {
                    "connection": {
                        "id": receive.connection_id,
                        "businessIdentity": receive.business_identity,
                        "context": context_value,
                    },
                    "message": message,
                }
            })
        }
    };
    pinned_plan.decode_event_json(&wire, heap, request.target())
}

fn websocket_name_values(
    values: &[crate::invocation_builder::EvalInvocationBuildWebSocketNameValue],
) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| json!({ "name": value.name, "value": value.value }))
            .collect(),
    )
}

fn canonical_receive_context_wire(
    receive: &crate::invocation_builder::EvalInvocationBuildWebSocketReceiveRequest,
    request: &RequestPayloadContext<'_>,
    pinned_plan: &PinnedWebSocketContractPlan<'_>,
    heap: &mut RequestHeap,
) -> Result<Value> {
    match pinned_plan.ingress_context() {
        WebSocketIngressContext::Null => {
            if receive.context_codec.is_some()
                || receive.payload_segments.iter().any(|segment| {
                    segment.kind == EvalInvocationBuildWebSocketPayloadSegmentKind::Context
                })
            {
                return Err(websocket_protocol_error(
                    request.target(),
                    "canonical WebSocket receive carried Context for a null contract Context",
                ));
            }
            Ok(Value::Null)
        }
        WebSocketIngressContext::PackageSchema(package_schema_type) => {
            let codec = receive.context_codec.as_ref().ok_or_else(|| {
                websocket_protocol_error(
                    request.target(),
                    "canonical WebSocket receive context codec is missing",
                )
            })?;
            if codec.operation_abi_id != pinned_plan.operation_id().as_str()
                || codec.context_type_identity
                    != package_schema_type.package_schema_type_id.as_str()
            {
                return Err(websocket_protocol_error(
                    request.target(),
                    "canonical WebSocket receive context codec does not match the pinned contract",
                ));
            }
            let bytes = canonical_payload_segment(
                receive,
                request,
                EvalInvocationBuildWebSocketPayloadSegmentKind::Context,
            )?;
            pinned_plan.decode_context_binary_to_json(
                bytes,
                &websocket_payload_boundary(),
                heap,
                request.target(),
            )
        }
    }
}

fn canonical_receive_message_wire(
    receive: &crate::invocation_builder::EvalInvocationBuildWebSocketReceiveRequest,
    request: &RequestPayloadContext<'_>,
) -> Result<Value> {
    let bytes = canonical_payload_segment(
        receive,
        request,
        EvalInvocationBuildWebSocketPayloadSegmentKind::Message,
    )?;
    match (receive.message.tag, receive.message.encoding) {
        (
            EvalInvocationBuildWebSocketMessageTag::Text,
            EvalInvocationBuildWebSocketMessageEncoding::Utf8,
        ) => {
            let text = std::str::from_utf8(bytes).map_err(|error| {
                RuntimeError::Decode(format!("websocket text is not UTF-8: {error}"))
            })?;
            Ok(json!({ "tag": "text", "text": text }))
        }
        (
            EvalInvocationBuildWebSocketMessageTag::Binary,
            EvalInvocationBuildWebSocketMessageEncoding::Raw,
        ) => Ok(json!({
            "tag": "binary",
            "base64": boundary_bytes::encode_base64(bytes),
        })),
        _ => Err(websocket_protocol_error(
            request.target(),
            "canonical WebSocket message tag and encoding do not match",
        )),
    }
}

fn canonical_payload_segment<'a>(
    receive: &crate::invocation_builder::EvalInvocationBuildWebSocketReceiveRequest,
    request: &'a RequestPayloadContext<'_>,
    kind: EvalInvocationBuildWebSocketPayloadSegmentKind,
) -> Result<&'a [u8]> {
    let matching = receive
        .payload_segments
        .iter()
        .filter(|segment| segment.kind == kind)
        .collect::<Vec<_>>();
    let [segment] = matching.as_slice() else {
        return Err(websocket_protocol_error(
            request.target(),
            "canonical WebSocket payload segment is missing or duplicated",
        ));
    };
    let end = segment.offset.checked_add(segment.length).ok_or_else(|| {
        websocket_protocol_error(
            request.target(),
            "canonical WebSocket payload range overflows",
        )
    })?;
    request
        .payload_bytes()
        .get(segment.offset..end)
        .ok_or_else(|| {
            websocket_protocol_error(
                request.target(),
                "canonical WebSocket payload range is out of bounds",
            )
        })
}

fn websocket_payload_boundary() -> PayloadBoundary {
    PayloadBoundary::external_untrusted(PayloadBoundaryKind::WebsocketRequest)
}

fn websocket_protocol_error(target: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::Protocol {
        target: target.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::invocation_builder::{
        EvalInvocationBuildArg, EvalInvocationBuildWebSocketConnectRequest,
        EvalInvocationBuildWebSocketContextCodec, EvalInvocationBuildWebSocketContextExpectation,
        EvalInvocationBuildWebSocketMessage, EvalInvocationBuildWebSocketPayloadSegment,
        EvalInvocationBuildWebSocketReceiveRequest,
    };
    use websocket_contract_plan::test_support::{empty_nominal_contract, null_contract};

    fn event_arg() -> EvalInvocationBuildArg {
        EvalInvocationBuildArg {
            param: "event".to_string(),
            from: EvalInvocationBuildArgFrom::WebSocketIngressEvent,
        }
    }

    fn connect_adapter() -> EvalInvocationBuildWebSocketAdapter {
        EvalInvocationBuildWebSocketAdapter {
            kind: EvalInvocationBuildWebSocketKind::Connect,
            args: vec![event_arg()],
            context_expectation: None,
            connect_request: Some(EvalInvocationBuildWebSocketConnectRequest {
                connection_id: "connection-A".to_string(),
                url: "ws://example.test/socket".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
                cookies: Vec::new(),
                version: None,
            }),
            receive_request: None,
        }
    }

    fn receive_adapter(
        context_codec: Option<EvalInvocationBuildWebSocketContextCodec>,
        context_length: Option<usize>,
        message: EvalInvocationBuildWebSocketMessage,
        message_length: usize,
    ) -> EvalInvocationBuildWebSocketAdapter {
        let mut payload_segments = Vec::new();
        let message_offset = context_length.unwrap_or(0);
        if let Some(context_length) = context_length {
            payload_segments.push(EvalInvocationBuildWebSocketPayloadSegment {
                kind: EvalInvocationBuildWebSocketPayloadSegmentKind::Context,
                offset: 0,
                length: context_length,
            });
        }
        payload_segments.push(EvalInvocationBuildWebSocketPayloadSegment {
            kind: EvalInvocationBuildWebSocketPayloadSegmentKind::Message,
            offset: message_offset,
            length: message_length,
        });
        EvalInvocationBuildWebSocketAdapter {
            kind: EvalInvocationBuildWebSocketKind::Receive,
            args: vec![event_arg()],
            context_expectation: None,
            connect_request: None,
            receive_request: Some(EvalInvocationBuildWebSocketReceiveRequest {
                connection_id: "connection-A".to_string(),
                business_identity: None,
                message,
                context_codec,
                payload_segments,
            }),
        }
    }

    #[test]
    fn websocket_adapter_materializes_null_connect_text_and_binary_from_pinned_contract() {
        let fixture = null_contract();
        let plan = PinnedWebSocketContractPlan::compile(
            &fixture.contract,
            &fixture.operation_id,
            &fixture.package_schema_records,
        )
        .unwrap();
        let mut heap = RequestHeap::default();
        let connect_request = RequestPayloadContext::new("websocket", &[], None);
        let connect =
            canonical_websocket_event_value(&connect_adapter(), &connect_request, &plan, &mut heap)
                .unwrap();
        assert_eq!(
            plan.event_value_plan()
                .encode_json_value(&connect, &mut heap)
                .unwrap()["tag"],
            "connect"
        );

        for (payload, message, expected) in [
            (
                b"marker-A".to_vec(),
                EvalInvocationBuildWebSocketMessage {
                    tag: EvalInvocationBuildWebSocketMessageTag::Text,
                    encoding: EvalInvocationBuildWebSocketMessageEncoding::Utf8,
                },
                json!({"tag": "text", "text": "marker-A"}),
            ),
            (
                vec![0, 1, 2],
                EvalInvocationBuildWebSocketMessage {
                    tag: EvalInvocationBuildWebSocketMessageTag::Binary,
                    encoding: EvalInvocationBuildWebSocketMessageEncoding::Raw,
                },
                json!({"tag": "binary", "base64": "AAEC"}),
            ),
        ] {
            let adapter = receive_adapter(None, None, message, payload.len());
            let request = RequestPayloadContext::new("websocket", &payload, None);
            let receive = canonical_websocket_event_value(&adapter, &request, &plan, &mut heap)
                .expect("null Context receive should decode from the pinned Event plan");
            let receive = plan
                .event_value_plan()
                .encode_json_value(&receive, &mut heap)
                .unwrap();
            assert_eq!(receive["tag"], "receive");
            assert_eq!(
                receive["receiveEvent"]["connection"]["context"],
                json!(null)
            );
            assert_eq!(receive["receiveEvent"]["message"], expected);
        }
    }

    #[test]
    fn websocket_adapter_decodes_nominal_context_for_text_and_binary_receive() {
        let fixture = empty_nominal_contract();
        let context_type_id = fixture
            .context_type
            .as_ref()
            .unwrap()
            .package_schema_type_id
            .clone();
        let plan = PinnedWebSocketContractPlan::compile(
            &fixture.contract,
            &fixture.operation_id,
            &fixture.package_schema_records,
        )
        .unwrap();
        let mut context_heap = RequestHeap::default();
        let context = plan
            .context_value_plan()
            .decode_json_value(&json!({}), &mut context_heap)
            .unwrap();
        let context_bytes = plan
            .context_value_plan()
            .encode_binary(&context, &websocket_payload_boundary(), &context_heap)
            .unwrap();
        let codec = EvalInvocationBuildWebSocketContextCodec {
            operation_abi_id: fixture.operation_id.as_str().to_string(),
            context_type_identity: context_type_id.as_str().to_string(),
        };

        for (message_bytes, message, expected) in [
            (
                b"nominal-text".to_vec(),
                EvalInvocationBuildWebSocketMessage {
                    tag: EvalInvocationBuildWebSocketMessageTag::Text,
                    encoding: EvalInvocationBuildWebSocketMessageEncoding::Utf8,
                },
                json!({"tag": "text", "text": "nominal-text"}),
            ),
            (
                vec![0xff, 0x00],
                EvalInvocationBuildWebSocketMessage {
                    tag: EvalInvocationBuildWebSocketMessageTag::Binary,
                    encoding: EvalInvocationBuildWebSocketMessageEncoding::Raw,
                },
                json!({"tag": "binary", "base64": "/wA="}),
            ),
        ] {
            let mut payload = context_bytes.clone();
            payload.extend_from_slice(&message_bytes);
            let adapter = receive_adapter(
                Some(codec.clone()),
                Some(context_bytes.len()),
                message,
                message_bytes.len(),
            );
            let request = RequestPayloadContext::new("websocket", &payload, None);
            let mut heap = RequestHeap::default();
            let receive = canonical_websocket_event_value(&adapter, &request, &plan, &mut heap)
                .expect("nominal Context must decode before the Event plan materializes");
            let receive = plan
                .event_value_plan()
                .encode_json_value(&receive, &mut heap)
                .unwrap();
            assert_eq!(receive["receiveEvent"]["connection"]["context"], json!({}));
            assert_eq!(receive["receiveEvent"]["message"], expected);
        }
    }

    #[test]
    fn websocket_adapter_rejects_ambient_context_expectation() {
        let mut adapter = connect_adapter();
        adapter.context_expectation = Some(EvalInvocationBuildWebSocketContextExpectation::Null);
        let error = validate_canonical_adapter(&adapter, "websocket")
            .expect_err("canonical ingress must derive Context from the pinned contract");
        assert!(error.to_string().contains("pinned ServiceContract"));
    }

    #[test]
    fn websocket_adapter_rejects_codec_identity_drift_and_preserves_zero_byte_presence() {
        let fixture = empty_nominal_contract();
        let context_type_id = fixture
            .context_type
            .as_ref()
            .unwrap()
            .package_schema_type_id
            .clone();
        let plan = PinnedWebSocketContractPlan::compile(
            &fixture.contract,
            &fixture.operation_id,
            &fixture.package_schema_records,
        )
        .unwrap();
        let message = EvalInvocationBuildWebSocketMessage {
            tag: EvalInvocationBuildWebSocketMessageTag::Text,
            encoding: EvalInvocationBuildWebSocketMessageEncoding::Utf8,
        };
        for codec in [
            EvalInvocationBuildWebSocketContextCodec {
                operation_abi_id: "operation:other".to_string(),
                context_type_identity: context_type_id.as_str().to_string(),
            },
            EvalInvocationBuildWebSocketContextCodec {
                operation_abi_id: fixture.operation_id.as_str().to_string(),
                context_type_identity: "contract-type:other".to_string(),
            },
        ] {
            let adapter = receive_adapter(Some(codec), Some(0), message.clone(), 0);
            let request = RequestPayloadContext::new("websocket", &[], None);
            let error = canonical_websocket_event_value(
                &adapter,
                &request,
                &plan,
                &mut RequestHeap::default(),
            )
            .expect_err("codec must be bound to the pinned contract tuple");
            assert!(error
                .to_string()
                .contains("does not match the pinned contract"));
        }

        let correct = receive_adapter(
            Some(EvalInvocationBuildWebSocketContextCodec {
                operation_abi_id: fixture.operation_id.as_str().to_string(),
                context_type_identity: context_type_id.as_str().to_string(),
            }),
            Some(0),
            message,
            0,
        );
        let receive = correct.receive_request.as_ref().unwrap();
        let request = RequestPayloadContext::new("websocket", &[], None);
        assert!(canonical_payload_segment(
            receive,
            &request,
            EvalInvocationBuildWebSocketPayloadSegmentKind::Context,
        )
        .expect("typed zero-byte Context segment must remain structurally present")
        .is_empty());
    }
}
