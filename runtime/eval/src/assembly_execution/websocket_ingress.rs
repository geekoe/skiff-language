use std::collections::BTreeMap;

use serde_json::{json, Value};
use skiff_artifact_model::{websocket_ingress_context, WebSocketIngressContext};
use skiff_runtime_boundary::{
    binary::decode_payload_plan,
    payload::{PayloadBoundary, PayloadBoundaryKind},
    value as boundary_bytes,
};
use skiff_runtime_capability_context::RequestPayloadContext;
use skiff_runtime_linked_program::{CallIr, LinkedCallTarget};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
    type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan},
};

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
    program_ir::executable_has_explicit_self_binding,
    runtime_ops::{runtime_from_wire_required_plan, runtime_to_wire_required_plan},
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
    let descriptor = target.descriptor();
    let operation_id = descriptor.operation_id.clone();
    let ingress_context = websocket_ingress_context(target.contract(), &operation_id)
        .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
    validate_canonical_adapter(adapter, request.target())?;
    validate_admitted_identity(
        target.contract().service_id.as_str(),
        target.contract().service_protocol_identity.as_str(),
        target.descriptor().operation_id.as_str(),
        admitted_identity,
        request.target(),
    )?;

    let addr = target.executable_addr().clone();
    let projection = super::RuntimeExecutionProjection::for_context(interpreter, &context)?;
    let resolved = projection.resolve_executable(&addr)?;
    let canonical_addr = resolved.addr.clone();
    let parameters = resolved
        .executable
        .params
        .iter()
        .skip(usize::from(executable_has_explicit_self_binding(
            resolved.executable,
        )))
        .collect::<Vec<_>>();
    let [parameter] = parameters.as_slice() else {
        return Err(RuntimeError::InvalidArtifact(
            "canonical WebSocket ingress executable must have exactly one event parameter"
                .to_string(),
        ));
    };
    if parameter.name != "event" {
        return Err(RuntimeError::InvalidArtifact(
            "canonical WebSocket ingress executable parameter must be named event".to_string(),
        ));
    }
    let event_plan = RuntimeTypePlan::from_linked(
        &parameter.ty,
        &PlanContext::from_type_view(projection.type_view(), &canonical_addr),
    )?;
    let event = canonical_websocket_event_value(
        adapter,
        request,
        &event_plan,
        &ingress_context,
        &operation_id,
        heap,
    )?;
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
            let return_type = resolved.executable.return_type.as_ref().ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "canonical WebSocket ingress executable has no return type".to_string(),
                )
            })?;
            let return_plan = RuntimeTypePlan::from_linked(
                return_type,
                &PlanContext::from_type_view(projection.type_view(), &canonical_addr),
            )?;
            super::websocket_response::project_connect_response(
                request.target(),
                &value,
                &return_plan,
                &ingress_context,
                &operation_id,
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
    event_plan: &RuntimeTypePlan,
    context: &WebSocketIngressContext,
    operation_id: &skiff_artifact_model::ContractOperationId,
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
            let context_value = canonical_receive_context_wire(
                receive,
                request,
                event_plan,
                context,
                operation_id,
                heap,
            )?;
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
    runtime_from_wire_required_plan(
        &wire,
        Some(event_plan),
        "canonical websocket ingress event",
        heap,
    )
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
    event_plan: &RuntimeTypePlan,
    context: &WebSocketIngressContext,
    operation_id: &skiff_artifact_model::ContractOperationId,
    heap: &mut RequestHeap,
) -> Result<Value> {
    match context {
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
        WebSocketIngressContext::Contract(contract_type_id) => {
            let codec = receive.context_codec.as_ref().ok_or_else(|| {
                websocket_protocol_error(
                    request.target(),
                    "canonical WebSocket receive context codec is missing",
                )
            })?;
            if codec.operation_abi_id != operation_id.as_str()
                || codec.context_type_identity != contract_type_id.as_str()
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
            let plan = ingress_receive_context_plan(event_plan)?;
            let value = decode_payload_plan(bytes, plan, &websocket_payload_boundary(), heap)?;
            runtime_to_wire_required_plan(&value, Some(plan), "websocket context", heap)
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

fn ingress_receive_context_plan(event_plan: &RuntimeTypePlan) -> Result<&RuntimeTypePlan> {
    let receive_branch = union_items(event_plan)
        .iter()
        .find(|plan| record_has_tag(plan, "receive"))
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "WebSocketIngressEvent plan has no receive branch".to_string(),
            )
        })?;
    let receive_event = record_field_plan(receive_branch, "receiveEvent")?;
    let connection = record_field_plan(receive_event, "connection")?;
    record_field_plan(connection, "context")
}

fn record_has_tag(plan: &RuntimeTypePlan, tag: &str) -> bool {
    let RuntimeTypeNode::Record { fields, .. } = peel_plan(plan).node() else {
        return false;
    };
    fields.iter().any(|field| {
        field.name == "tag"
            && matches!(peel_plan(&field.ty).node(), RuntimeTypeNode::LiteralString(value) if value == tag)
    })
}

fn record_field_plan<'a>(plan: &'a RuntimeTypePlan, name: &str) -> Result<&'a RuntimeTypePlan> {
    let RuntimeTypeNode::Record { fields, .. } = peel_plan(plan).node() else {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical WebSocket type containing {name} must be a record"
        )));
    };
    field_plan(fields, name)
}

fn field_plan<'a>(fields: &'a [RuntimeRecordFieldPlan], name: &str) -> Result<&'a RuntimeTypePlan> {
    fields
        .iter()
        .find(|field| field.name == name)
        .map(|field| &field.ty)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "canonical WebSocket type is missing field {name}"
            ))
        })
}

fn union_items(plan: &RuntimeTypePlan) -> &[RuntimeTypePlan] {
    match peel_plan(plan).node() {
        RuntimeTypeNode::Union(items) => items,
        _ => std::slice::from_ref(plan),
    }
}

fn peel_plan(plan: &RuntimeTypePlan) -> &RuntimeTypePlan {
    match plan.node() {
        RuntimeTypeNode::Alias(inner)
        | RuntimeTypeNode::Nullable(inner)
        | RuntimeTypeNode::Representation { payload: inner, .. } => peel_plan(inner),
        _ => plan,
    }
}

fn websocket_protocol_error(target: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::Protocol {
        target: target.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use skiff_artifact_model::{ContractOperationId, ContractTypeId};
    use skiff_runtime_boundary::type_descriptor::RuntimeTypePlanDescriptorExt;

    use super::*;
    use crate::invocation_builder::{
        EvalInvocationBuildArg, EvalInvocationBuildWebSocketConnectRequest,
        EvalInvocationBuildWebSocketContextCodec, EvalInvocationBuildWebSocketContextExpectation,
        EvalInvocationBuildWebSocketMessage, EvalInvocationBuildWebSocketPayloadSegment,
        EvalInvocationBuildWebSocketReceiveRequest,
    };

    fn ingress_plan(context: Value) -> RuntimeTypePlan {
        RuntimeTypePlan::from_descriptor(&json!({
            "kind": "builtin",
            "name": "std.websocket.WebSocketIngressEvent",
            "args": [context],
        }))
        .unwrap()
    }

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
        context_segment: bool,
        message_length: usize,
    ) -> EvalInvocationBuildWebSocketAdapter {
        let mut payload_segments = Vec::new();
        if context_segment {
            payload_segments.push(EvalInvocationBuildWebSocketPayloadSegment {
                kind: EvalInvocationBuildWebSocketPayloadSegmentKind::Context,
                offset: 0,
                length: 0,
            });
        }
        payload_segments.push(EvalInvocationBuildWebSocketPayloadSegment {
            kind: EvalInvocationBuildWebSocketPayloadSegmentKind::Message,
            offset: 0,
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
                message: EvalInvocationBuildWebSocketMessage {
                    tag: EvalInvocationBuildWebSocketMessageTag::Text,
                    encoding: EvalInvocationBuildWebSocketMessageEncoding::Utf8,
                },
                context_codec,
                payload_segments,
            }),
        }
    }

    #[test]
    fn websocket_adapter_materializes_connect_and_receive_union_branches() {
        let plan = ingress_plan(json!({ "kind": "builtin", "name": "null", "args": [] }));
        let operation = ContractOperationId::new("operation:websocket");
        let mut heap = RequestHeap::default();
        let connect_request = RequestPayloadContext::new("websocket", &[], None);
        let connect = canonical_websocket_event_value(
            &connect_adapter(),
            &connect_request,
            &plan,
            &WebSocketIngressContext::Null,
            &operation,
            &mut heap,
        )
        .unwrap();
        assert_eq!(
            runtime_to_wire_required_plan(&connect, Some(&plan), "connect", &mut heap).unwrap()
                ["tag"],
            "connect"
        );

        let payload = b"marker-A";
        let receive_request = RequestPayloadContext::new("websocket", payload, None);
        let receive = canonical_websocket_event_value(
            &receive_adapter(None, false, payload.len()),
            &receive_request,
            &plan,
            &WebSocketIngressContext::Null,
            &operation,
            &mut heap,
        )
        .unwrap();
        let receive =
            runtime_to_wire_required_plan(&receive, Some(&plan), "receive", &mut heap).unwrap();
        assert_eq!(receive["tag"], "receive");
        assert_eq!(receive["receiveEvent"]["connection"]["id"], "connection-A");
        assert_eq!(receive["receiveEvent"]["message"]["text"], "marker-A");
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
    fn websocket_adapter_rejects_context_codec_outside_pinned_contract() {
        let operation = ContractOperationId::new("operation:websocket");
        let context_type = ContractTypeId::new("contract-type:context");
        let adapter = receive_adapter(
            Some(EvalInvocationBuildWebSocketContextCodec {
                operation_abi_id: "operation:other".to_string(),
                context_type_identity: context_type.as_str().to_string(),
            }),
            true,
            0,
        );
        let request = RequestPayloadContext::new("websocket", &[], None);
        let plan = ingress_plan(json!({ "kind": "record", "fields": {} }));
        let mut heap = RequestHeap::default();
        let error = canonical_websocket_event_value(
            &adapter,
            &request,
            &plan,
            &WebSocketIngressContext::Contract(context_type),
            &operation,
            &mut heap,
        )
        .expect_err("codec must be bound to the pinned contract operation");
        assert!(error
            .to_string()
            .contains("does not match the pinned contract"));
    }
}
