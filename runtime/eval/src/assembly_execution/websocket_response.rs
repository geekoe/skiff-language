use skiff_artifact_model::{ContractOperationId, WebSocketIngressContext};
use skiff_runtime_boundary::{
    binary::encode_payload_plan,
    payload::{PayloadBoundary, PayloadBoundaryKind},
};
use skiff_runtime_capability_context::WebSocketConnectionPolicyControl;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeObjectFields, RuntimeValue},
    type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan},
};

use crate::{
    error::{Result, RuntimeError},
    invocation::{
        EvalWebSocketAdapterResult, EvalWebSocketConnectAccept, EvalWebSocketConnectContext,
        EvalWebSocketConnectReject, EvalWebSocketContextCodec,
    },
    runtime_ops::runtime_to_wire_required_plan,
};

/// The sole canonical projector from a typed WebSocket operation result to the runtime response
/// boundary. Legacy adapter projection intentionally remains in `websocket_adapter`.
pub(super) fn project_connect_response(
    request_target: &str,
    value: &RuntimeValue,
    return_plan: &RuntimeTypePlan,
    context: &WebSocketIngressContext,
    operation_id: &ContractOperationId,
    heap: &mut RequestHeap,
) -> Result<EvalWebSocketAdapterResult> {
    let protocol_error = |message: String| RuntimeError::Protocol {
        target: request_target.to_string(),
        message,
    };
    if matches!(value, RuntimeValue::Null) {
        return Err(protocol_error(
            "websocket connect operation must return accept or reject, not null".to_string(),
        ));
    }
    let tag = {
        let fields = runtime_object_fields(value, heap)?;
        runtime_string_field(fields, "tag", heap)?
    };
    if tag == "reject" {
        let fields = runtime_object_fields(value, heap)?;
        let code = runtime_number_field(fields, "code")?;
        if !code.is_finite() || code.fract() != 0.0 || !(0.0..=u16::MAX as f64).contains(&code) {
            return Err(protocol_error(
                "websocket reject code must be an unsigned 16-bit integer".to_string(),
            ));
        }
        return Ok(EvalWebSocketAdapterResult::ConnectReject(
            EvalWebSocketConnectReject {
                code: code as u16,
                reason: runtime_string_field(fields, "reason", heap)?,
            },
        ));
    }
    if tag != "accept" {
        return Err(protocol_error(format!(
            "websocket connect returned unsupported tag {tag}"
        )));
    }
    let (context_value, business_identity, connection_policy_value) = {
        let fields = runtime_object_fields(value, heap)?;
        (
            fields.get("context").cloned().unwrap_or(RuntimeValue::Null),
            optional_runtime_string_field(fields, "businessIdentity", heap)?,
            fields.get("connectionPolicy").cloned(),
        )
    };
    let context_plan = connect_accept_field_plan(return_plan, "context")?;
    let context = match context {
        WebSocketIngressContext::Null => {
            if !matches!(context_value, RuntimeValue::Null) {
                return Err(protocol_error(
                    "websocket accept Context must be null for this ServiceContract".to_string(),
                ));
            }
            EvalWebSocketConnectContext::Null
        }
        WebSocketIngressContext::Contract(contract_type_id) => EvalWebSocketConnectContext::Typed {
            payload: encode_payload_plan(
                &context_value,
                context_plan,
                &websocket_payload_boundary(),
                heap,
            )?,
            codec: EvalWebSocketContextCodec {
                operation_abi_id: operation_id.as_str().to_string(),
                context_type_identity: contract_type_id.as_str().to_string(),
            },
        },
    };
    let connection_policy = match connection_policy_value {
        Some(RuntimeValue::Null) | None => None,
        Some(value) => {
            let plan = connect_accept_field_plan(return_plan, "connectionPolicy")?;
            let wire = runtime_to_wire_required_plan(
                &value,
                Some(plan),
                "websocket connection policy",
                heap,
            )?;
            Some(
                serde_json::from_value::<WebSocketConnectionPolicyControl>(wire).map_err(
                    |error| {
                        protocol_error(format!(
                            "websocket connection policy does not match runtime wire schema: {error}"
                        ))
                    },
                )?,
            )
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

fn connect_accept_field_plan<'a>(
    return_plan: &'a RuntimeTypePlan,
    field_name: &str,
) -> Result<&'a RuntimeTypePlan> {
    for item in union_items(return_plan) {
        if let RuntimeTypeNode::Record { fields, .. } = item.node() {
            if fields.iter().any(|field| {
                field.name == "tag" && literal_string_plan(&field.ty) == Some("accept")
            }) {
                return field_plan(fields, field_name);
            }
        }
    }
    Err(RuntimeError::Decode(format!(
        "websocket connect return type missing accept {field_name}"
    )))
}

fn union_items(plan: &RuntimeTypePlan) -> &[RuntimeTypePlan] {
    match peel_plan(plan).node() {
        RuntimeTypeNode::Union(items) => items,
        _ => std::slice::from_ref(plan),
    }
}

fn peel_plan(plan: &RuntimeTypePlan) -> &RuntimeTypePlan {
    match plan.node() {
        RuntimeTypeNode::Alias(inner) | RuntimeTypeNode::Nullable(inner) => peel_plan(inner),
        _ => plan,
    }
}

fn field_plan<'a>(fields: &'a [RuntimeRecordFieldPlan], name: &str) -> Result<&'a RuntimeTypePlan> {
    fields
        .iter()
        .find(|field| field.name == name)
        .map(|field| &field.ty)
        .ok_or_else(|| RuntimeError::Decode(format!("websocket type missing field {name}")))
}

fn literal_string_plan(plan: &RuntimeTypePlan) -> Option<&str> {
    match peel_plan(plan).node() {
        RuntimeTypeNode::LiteralString(value) => Some(value.as_str()),
        _ => None,
    }
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

fn runtime_string_field(
    fields: &RuntimeObjectFields,
    name: &str,
    heap: &RequestHeap,
) -> Result<String> {
    match fields.get(name) {
        Some(RuntimeValue::String(value)) => Ok(value.clone()),
        Some(RuntimeValue::Heap(handle)) => match heap.get(*handle)? {
            HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                "websocket field {name} cannot be {}",
                value.diagnostic_label()
            ))),
            _ => Err(RuntimeError::Decode(format!(
                "websocket field {name} must be string"
            ))),
        },
        _ => Err(RuntimeError::Decode(format!(
            "websocket field {name} must be string"
        ))),
    }
}

fn optional_runtime_string_field(
    fields: &RuntimeObjectFields,
    name: &str,
    heap: &RequestHeap,
) -> Result<Option<String>> {
    match fields.get(name) {
        Some(RuntimeValue::Null) | None => Ok(None),
        Some(_) => runtime_string_field(fields, name, heap).map(Some),
    }
}

fn runtime_number_field(fields: &RuntimeObjectFields, name: &str) -> Result<f64> {
    match fields.get(name) {
        Some(RuntimeValue::Number(value)) => Ok(*value),
        _ => Err(RuntimeError::Decode(format!(
            "websocket field {name} must be number"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use skiff_artifact_model::{ContractTypeId, WebSocketIngressContext};
    use skiff_runtime_boundary::type_descriptor::RuntimeTypePlanDescriptorExt;
    use skiff_runtime_model::runtime_value::RuntimeValue;

    use super::*;
    use crate::runtime_ops::runtime_from_wire_required_plan;

    fn named(name: &str) -> Value {
        json!({ "kind": "builtin", "name": name, "args": [] })
    }

    fn connect_return_plan(context: Value) -> RuntimeTypePlan {
        RuntimeTypePlan::from_descriptor(&json!({
            "kind": "builtin",
            "name": "std.websocket.WebSocketConnectResult",
            "args": [context],
        }))
        .expect("connect result plan should build")
    }

    #[test]
    fn canonical_websocket_response_requires_non_null_connect_result() {
        let mut heap = RequestHeap::default();
        let error = project_connect_response(
            "test-runtime",
            &RuntimeValue::Null,
            &connect_return_plan(named("null")),
            &WebSocketIngressContext::Null,
            &ContractOperationId::new("operation:websocket"),
            &mut heap,
        )
        .expect_err("connect null must fail closed");
        assert!(error
            .to_string()
            .contains("must return accept or reject, not null"));
    }

    #[test]
    fn canonical_websocket_response_discriminates_null_accept_and_reject() {
        let return_plan = connect_return_plan(named("null"));
        let mut heap = RequestHeap::default();
        let accept = runtime_from_wire_required_plan(
            &json!({
                "tag": "accept",
                "context": null,
                "businessIdentity": null,
                "connectionPolicy": null,
            }),
            Some(&return_plan),
            "canonical websocket connect result",
            &mut heap,
        )
        .unwrap();
        assert!(matches!(
            project_connect_response(
                "test-runtime",
                &accept,
                &return_plan,
                &WebSocketIngressContext::Null,
                &ContractOperationId::new("operation:websocket"),
                &mut heap,
            )
            .unwrap(),
            EvalWebSocketAdapterResult::ConnectAccept(EvalWebSocketConnectAccept {
                context: EvalWebSocketConnectContext::Null,
                ..
            })
        ));

        let reject = runtime_from_wire_required_plan(
            &json!({ "tag": "reject", "code": 1008, "reason": "policy" }),
            Some(&return_plan),
            "canonical websocket reject",
            &mut heap,
        )
        .unwrap();
        assert_eq!(
            project_connect_response(
                "test-runtime",
                &reject,
                &return_plan,
                &WebSocketIngressContext::Null,
                &ContractOperationId::new("operation:websocket"),
                &mut heap,
            )
            .unwrap(),
            EvalWebSocketAdapterResult::ConnectReject(EvalWebSocketConnectReject {
                code: 1008,
                reason: "policy".to_string(),
            })
        );
    }

    #[test]
    fn canonical_websocket_response_binds_typed_context_codec_to_contract() {
        let return_plan = connect_return_plan(json!({ "kind": "record", "fields": {} }));
        let mut heap = RequestHeap::default();
        let value = runtime_from_wire_required_plan(
            &json!({
                "tag": "accept",
                "context": {},
                "businessIdentity": null,
                "connectionPolicy": null,
            }),
            Some(&return_plan),
            "canonical websocket typed connect result",
            &mut heap,
        )
        .unwrap();
        let operation = ContractOperationId::new("operation:websocket");
        let context_type = ContractTypeId::new("contract-type:context");
        let result = project_connect_response(
            "test-runtime",
            &value,
            &return_plan,
            &WebSocketIngressContext::Contract(context_type.clone()),
            &operation,
            &mut heap,
        )
        .unwrap();
        let EvalWebSocketAdapterResult::ConnectAccept(accept) = result else {
            panic!("expected typed accept");
        };
        let EvalWebSocketConnectContext::Typed { payload, codec } = accept.context else {
            panic!("expected typed context");
        };
        assert!(!payload.is_empty());
        assert_eq!(codec.operation_abi_id, operation.as_str());
        assert_eq!(codec.context_type_identity, context_type.as_str());
    }
}
