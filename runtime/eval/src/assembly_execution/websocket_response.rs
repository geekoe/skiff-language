use serde_json::{Map, Value};
use skiff_artifact_model::WebSocketIngressContext;
use skiff_runtime_boundary::payload::{PayloadBoundary, PayloadBoundaryKind};
use skiff_runtime_capability_context::WebSocketConnectionPolicyControl;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeObjectFields, RuntimeValue},
};

use super::websocket_ingress::PinnedWebSocketContractPlan;
use crate::{
    error::{Result, RuntimeError},
    invocation::{
        EvalWebSocketAdapterResult, EvalWebSocketConnectAccept, EvalWebSocketConnectContext,
        EvalWebSocketConnectReject, EvalWebSocketContextCodec,
    },
};

/// The sole canonical projector from a typed WebSocket operation result to the runtime response
/// boundary. Legacy adapter projection intentionally remains in `websocket_adapter`.
pub(super) fn project_connect_response(
    request_target: &str,
    value: &RuntimeValue,
    pinned_plan: &PinnedWebSocketContractPlan<'_>,
    heap: &mut RequestHeap,
) -> Result<EvalWebSocketAdapterResult> {
    if matches!(value, RuntimeValue::Null) {
        return Err(protocol_error(
            request_target,
            "websocket connect operation must return accept or reject, not null",
        ));
    }
    let wire = pinned_plan.encode_result_json(value, heap, request_target)?;
    let fields = wire_object(request_target, &wire)?;
    let tag = wire_string(request_target, fields, "tag")?;
    if tag == "reject" {
        let code = wire_number(request_target, fields, "code")?;
        if !code.is_finite() || code.fract() != 0.0 || !(0.0..=u16::MAX as f64).contains(&code) {
            return Err(protocol_error(
                request_target,
                "websocket reject code must be an unsigned 16-bit integer",
            ));
        }
        return Ok(EvalWebSocketAdapterResult::ConnectReject(
            EvalWebSocketConnectReject {
                code: code as u16,
                reason: wire_string(request_target, fields, "reason")?.to_string(),
            },
        ));
    }
    if tag != "accept" {
        return Err(protocol_error(
            request_target,
            format!("websocket connect returned unsupported tag {tag}"),
        ));
    }

    let context_value = runtime_object_fields(value, heap)?
        .get("context")
        .cloned()
        .ok_or_else(|| protocol_error(request_target, "websocket accept omitted Context"))?;
    let context = match pinned_plan.ingress_context() {
        WebSocketIngressContext::Null => {
            if !matches!(context_value, RuntimeValue::Null) {
                return Err(protocol_error(
                    request_target,
                    "websocket accept Context must be null for this ServiceContract",
                ));
            }
            EvalWebSocketConnectContext::Null
        }
        WebSocketIngressContext::Contract(contract_type_id) => EvalWebSocketConnectContext::Typed {
            payload: pinned_plan.encode_context_binary(
                &context_value,
                &websocket_payload_boundary(),
                heap,
                request_target,
            )?,
            codec: EvalWebSocketContextCodec {
                operation_abi_id: pinned_plan.operation_id().as_str().to_string(),
                context_type_identity: contract_type_id.as_str().to_string(),
            },
        },
    };
    let business_identity = wire_optional_string(request_target, fields, "businessIdentity")?;
    let connection_policy = match fields.get("connectionPolicy") {
        Some(Value::Null) => None,
        Some(value) => Some(
            serde_json::from_value::<WebSocketConnectionPolicyControl>(value.clone()).map_err(
                |error| {
                    protocol_error(
                        request_target,
                        format!(
                            "websocket connection policy does not match runtime wire schema: {error}"
                        ),
                    )
                },
            )?,
        ),
        None => {
            return Err(protocol_error(
                request_target,
                "websocket accept omitted connectionPolicy",
            ))
        }
    };
    Ok(EvalWebSocketAdapterResult::ConnectAccept(
        EvalWebSocketConnectAccept {
            business_identity,
            connection_policy,
            context,
        },
    ))
}

fn websocket_payload_boundary() -> PayloadBoundary {
    PayloadBoundary::external_untrusted(PayloadBoundaryKind::WebsocketRequest)
}

fn wire_object<'a>(request_target: &str, value: &'a Value) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| protocol_error(request_target, "websocket connect result must be an object"))
}

fn wire_string<'a>(
    request_target: &str,
    fields: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str> {
    fields.get(name).and_then(Value::as_str).ok_or_else(|| {
        protocol_error(
            request_target,
            format!("websocket field {name} must be string"),
        )
    })
}

fn wire_optional_string(
    request_target: &str,
    fields: &Map<String, Value>,
    name: &str,
) -> Result<Option<String>> {
    match fields.get(name) {
        Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| {
                protocol_error(
                    request_target,
                    format!("websocket field {name} must be string or null"),
                )
            }),
        None => Err(protocol_error(
            request_target,
            format!("websocket result omitted {name}"),
        )),
    }
}

fn wire_number(request_target: &str, fields: &Map<String, Value>, name: &str) -> Result<f64> {
    fields.get(name).and_then(Value::as_f64).ok_or_else(|| {
        protocol_error(
            request_target,
            format!("websocket field {name} must be number"),
        )
    })
}

fn runtime_object_fields<'a>(
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
) -> Result<&'a RuntimeObjectFields> {
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode(
            "websocket connect result must be an object".to_string(),
        ));
    };
    match heap.get(*handle)? {
        HeapNode::Object(object) => Ok(object.fields()),
        _ => Err(RuntimeError::Decode(
            "websocket connect result must be an object".to_string(),
        )),
    }
}

fn protocol_error(request_target: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::Protocol {
        target: request_target.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_runtime_model::runtime_value::RuntimeValue;

    use super::*;
    use crate::assembly_execution::websocket_ingress::{
        websocket_contract_plan_test_support::{empty_nominal_contract, null_contract},
        PinnedWebSocketContractPlan,
    };

    #[test]
    fn canonical_websocket_response_requires_non_null_connect_result() {
        let fixture = null_contract();
        let plan =
            PinnedWebSocketContractPlan::compile(&fixture.contract, &fixture.operation_id).unwrap();
        let error = project_connect_response(
            "test-runtime",
            &RuntimeValue::Null,
            &plan,
            &mut RequestHeap::default(),
        )
        .expect_err("connect null must fail closed");
        assert!(error
            .to_string()
            .contains("must return accept or reject, not null"));
    }

    #[test]
    fn canonical_websocket_response_discriminates_null_accept_reject_and_policy() {
        let fixture = null_contract();
        let plan =
            PinnedWebSocketContractPlan::compile(&fixture.contract, &fixture.operation_id).unwrap();
        let mut heap = RequestHeap::default();
        let accept = plan
            .result_value_plan()
            .decode_json_value(
                &json!({
                    "tag": "accept",
                    "context": null,
                    "businessIdentity": "tenant-1",
                    "connectionPolicy": {
                        "maxConnections": 2,
                        "overflow": "close-oldest",
                        "closeCode": null,
                        "closeReason": null
                    },
                }),
                &mut heap,
            )
            .unwrap();
        let result = project_connect_response("test-runtime", &accept, &plan, &mut heap).unwrap();
        let EvalWebSocketAdapterResult::ConnectAccept(accept) = result else {
            panic!("expected connect accept")
        };
        assert_eq!(accept.business_identity.as_deref(), Some("tenant-1"));
        assert_eq!(accept.context, EvalWebSocketConnectContext::Null);
        assert_eq!(accept.connection_policy.unwrap().max_connections.get(), 2);

        let reject = plan
            .result_value_plan()
            .decode_json_value(
                &json!({ "tag": "reject", "code": 1008, "reason": "policy" }),
                &mut heap,
            )
            .unwrap();
        assert_eq!(
            project_connect_response("test-runtime", &reject, &plan, &mut heap).unwrap(),
            EvalWebSocketAdapterResult::ConnectReject(EvalWebSocketConnectReject {
                code: 1008,
                reason: "policy".to_string(),
            })
        );
    }

    #[test]
    fn canonical_websocket_response_encodes_nominal_context_from_the_pinned_plan() {
        let fixture = empty_nominal_contract();
        let plan =
            PinnedWebSocketContractPlan::compile(&fixture.contract, &fixture.operation_id).unwrap();
        let mut heap = RequestHeap::default();
        let value = plan
            .result_value_plan()
            .decode_json_value(
                &json!({
                    "tag": "accept",
                    "context": {},
                    "businessIdentity": null,
                    "connectionPolicy": null,
                }),
                &mut heap,
            )
            .unwrap();
        let result = project_connect_response("test-runtime", &value, &plan, &mut heap).unwrap();
        let EvalWebSocketAdapterResult::ConnectAccept(accept) = result else {
            panic!("expected typed accept")
        };
        let EvalWebSocketConnectContext::Typed { payload, codec } = accept.context else {
            panic!("expected typed context")
        };
        assert_eq!(codec.operation_abi_id, fixture.operation_id.as_str());
        assert_eq!(
            codec.context_type_identity,
            fixture.context_type_id.unwrap().as_str()
        );
        let decoded = plan
            .context_value_plan()
            .decode_binary(
                &payload,
                &websocket_payload_boundary(),
                &mut RequestHeap::default(),
            )
            .expect("typed Context payload should decode with the same pinned plan");
        assert!(matches!(decoded, RuntimeValue::Heap(_)));
    }
}
