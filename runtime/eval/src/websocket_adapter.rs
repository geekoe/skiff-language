use serde_json::{json, Value};
use skiff_runtime_boundary::{
    binary::{decode_payload_plan, encode_payload_plan},
    payload::{PayloadBoundary, PayloadBoundaryKind},
    plan::BoundaryUse,
    value as boundary_bytes,
};
use skiff_runtime_capability_context::RequestPayloadContext;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeObjectFields, RuntimeValue},
    type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan},
};

use super::{
    program_invocation::ProgramInvocationContext,
    runtime_ops::{
        runtime_decode_json_text_required_plan, runtime_from_wire_required_plan,
        runtime_to_wire_required_plan,
    },
    Interpreter,
};
use crate::{
    error::{Result, RuntimeError},
    invocation::{
        AdapterArgPlan, AdapterArgSource, EvalBoundaryProjection, EvalInvocation,
        EvalWebSocketAdapterResult, EvalWebSocketConnectRequest, EvalWebSocketConnectResponse,
        EvalWebSocketConnectResult, EvalWebSocketContextCodec, EvalWebSocketContextExpectation,
        EvalWebSocketMessageEncoding, EvalWebSocketMessageTag, EvalWebSocketNameValue,
        EvalWebSocketPayloadSegment, EvalWebSocketPayloadSegmentKind, EvalWebSocketReceiveRequest,
        WebSocketAdapterProjection, WebSocketAdapterProjectionKind,
    },
};
use skiff_runtime_capability_context::WebSocketConnectionPolicyControl;

struct WebSocketAdapterContext<'a> {
    request: RequestPayloadContext<'a>,
    context_expectation: Option<EvalWebSocketContextExpectation>,
    connect_request: Option<EvalWebSocketConnectRequest>,
    receive_request: Option<EvalWebSocketReceiveRequest>,
}

impl<'a> WebSocketAdapterContext<'a> {
    fn from_projection(
        invocation_context: &ProgramInvocationContext<'a>,
        adapter: &WebSocketAdapterProjection<'_>,
    ) -> Self {
        Self {
            request: invocation_context.request().clone(),
            context_expectation: adapter.context_expectation.clone(),
            connect_request: adapter.connect_request.clone(),
            receive_request: adapter.receive_request.clone(),
        }
    }

    fn protocol_error(&self, message: impl Into<String>) -> RuntimeError {
        RuntimeError::Protocol {
            target: self.request.target().to_string(),
            message: message.into(),
        }
    }

    fn payload_bytes(&self) -> &'a [u8] {
        self.request.payload_bytes()
    }
}

fn websocket_payload_boundary() -> PayloadBoundary {
    PayloadBoundary::external_untrusted(PayloadBoundaryKind::WebsocketRequest)
}

struct WebSocketAdapterPreparedCall<'a> {
    eval_invocation: EvalInvocation<'a>,
    args: Vec<RuntimeValue>,
}

impl Interpreter {
    pub async fn execute_program_websocket_adapter<'a>(
        &'a self,
        invocation_context: &ProgramInvocationContext<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<EvalWebSocketAdapterResult> {
        let adapter = websocket_adapter_projection(eval_invocation)?;
        let context = WebSocketAdapterContext::from_projection(invocation_context, &adapter);
        match adapter.kind {
            WebSocketAdapterProjectionKind::Connect => {
                self.execute_program_websocket_connect_adapter(invocation_context, adapter, context)
                    .await
            }
            WebSocketAdapterProjectionKind::Receive => {
                self.execute_program_websocket_receive_adapter(invocation_context, adapter, context)
                    .await
            }
        }
    }

    async fn execute_program_websocket_connect_adapter<'a>(
        &'a self,
        invocation_context: &ProgramInvocationContext<'a>,
        adapter: WebSocketAdapterProjection<'a>,
        context: WebSocketAdapterContext<'_>,
    ) -> Result<EvalWebSocketAdapterResult> {
        if context.connect_request.is_none() {
            return Err(context.protocol_error("websocket connect adapter metadata is missing"));
        }
        if !context.payload_bytes().is_empty() {
            return Err(context.protocol_error("websocket connect request payload must be empty"));
        }
        let mut heap = invocation_context.request_heap();
        let call = prepare_websocket_adapter_handler_call(&context, adapter, &mut heap)?;
        let (value, return_plan, mut heap) = self
            .execute_eval_invocation_runtime_args_with_heap(
                invocation_context,
                call.eval_invocation,
                call.args,
                heap,
            )
            .await?;
        websocket_connect_response(&context, &value, &return_plan, &mut heap)
    }

    async fn execute_program_websocket_receive_adapter<'a>(
        &'a self,
        invocation_context: &ProgramInvocationContext<'a>,
        adapter: WebSocketAdapterProjection<'a>,
        context: WebSocketAdapterContext<'_>,
    ) -> Result<EvalWebSocketAdapterResult> {
        if context.receive_request.is_none() {
            return Err(context.protocol_error("websocket receive adapter metadata is missing"));
        }
        let mut heap = invocation_context.request_heap();
        let call = prepare_websocket_adapter_handler_call(&context, adapter, &mut heap)?;
        let (_value, _return_plan, _heap) = self
            .execute_eval_invocation_runtime_args_with_heap(
                invocation_context,
                call.eval_invocation,
                call.args,
                heap,
            )
            .await?;
        Ok(EvalWebSocketAdapterResult {
            payload: Vec::new(),
            response: None,
        })
    }
}

fn prepare_websocket_adapter_handler_call<'a>(
    context: &WebSocketAdapterContext<'_>,
    adapter: WebSocketAdapterProjection<'a>,
    heap: &mut RequestHeap,
) -> Result<WebSocketAdapterPreparedCall<'a>> {
    let WebSocketAdapterProjection {
        handler,
        handler_args,
        ..
    } = adapter;
    let mut args = Vec::with_capacity(handler_args.len());
    for arg in &handler_args {
        args.push(websocket_adapter_arg_value(
            context,
            arg,
            &arg.parameter_plan,
            heap,
        )?);
    }

    Ok(WebSocketAdapterPreparedCall {
        eval_invocation: *handler,
        args,
    })
}

fn websocket_adapter_projection<'a>(
    eval_invocation: EvalInvocation<'a>,
) -> Result<WebSocketAdapterProjection<'a>> {
    let target = eval_invocation.request().target().to_string();
    match eval_invocation.boundary_projection().clone() {
        EvalBoundaryProjection::WebSocketAdapter { adapter } => Ok(adapter),
        _ => Err(RuntimeError::Protocol {
            target,
            message: "WebSocket adapter dispatch requires WebSocket adapter projection".to_string(),
        }),
    }
}

fn websocket_adapter_arg_value(
    context: &WebSocketAdapterContext<'_>,
    arg: &AdapterArgPlan,
    param_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    match arg.source {
        AdapterArgSource::WebSocketConnectRequest => {
            let request = context.connect_request.as_ref().ok_or_else(|| {
                context.protocol_error("websocket.connectRequest source is missing")
            })?;
            runtime_from_wire_required_plan(
                &json!({
                    "connectionId": request.connection_id,
                    "url": request.url,
                    "query": websocket_name_values_wire(&request.query),
                    "headers": websocket_name_values_wire(&request.headers),
                    "cookies": websocket_name_values_wire(&request.cookies),
                    "version": request.version,
                }),
                Some(param_plan),
                "websocket connect request",
                heap,
            )
        }
        AdapterArgSource::WebSocketReceiveEvent => {
            let receive = receive_request(context)?;
            let connection_plan = websocket_receive_event_connection_plan(param_plan)?;
            let connection = websocket_connection_wire(context, receive, connection_plan, heap)?;
            let message = websocket_message_wire(context, receive)?;
            runtime_from_wire_required_plan(
                &json!({
                    "connection": connection,
                    "message": message,
                }),
                Some(param_plan),
                "websocket receive event",
                heap,
            )
        }
        AdapterArgSource::WebSocketConnection => {
            let receive = receive_request(context)?;
            let value = websocket_connection_wire(context, receive, param_plan, heap)?;
            runtime_from_wire_required_plan(&value, Some(param_plan), "websocket connection", heap)
        }
        AdapterArgSource::WebSocketConnectionContext => {
            validate_receive_context_codec(context, receive_request(context)?)?;
            let bytes = payload_segment(context, EvalWebSocketPayloadSegmentKind::Context)?;
            Ok(decode_payload_plan(
                bytes,
                param_plan,
                &websocket_payload_boundary(),
                heap,
            )?)
        }
        AdapterArgSource::WebSocketMessage => {
            let receive = receive_request(context)?;
            let value = websocket_message_wire(context, receive)?;
            runtime_from_wire_required_plan(&value, Some(param_plan), "websocket message", heap)
        }
        AdapterArgSource::WebSocketMessageBody => {
            let receive = receive_request(context)?;
            let bytes = payload_segment(context, EvalWebSocketPayloadSegmentKind::Message)?;
            match receive.message.tag {
                EvalWebSocketMessageTag::Text => {
                    let text = std::str::from_utf8(bytes).map_err(|error| {
                        RuntimeError::Decode(format!(
                            "websocket text message is not UTF-8: {error}"
                        ))
                    })?;
                    runtime_decode_json_text_required_plan(
                        text,
                        param_plan,
                        "websocket message body",
                        BoundaryUse::HttpRequest,
                        heap,
                    )
                }
                EvalWebSocketMessageTag::Binary => {
                    let value = RuntimeValue::Heap(heap.alloc_bytes(bytes)?);
                    super::runtime_ops::runtime_coerce_required_plan(
                        &value,
                        param_plan,
                        "websocket binary message body",
                        heap,
                    )
                }
            }
        }
        AdapterArgSource::WebSocketConnectionId => {
            let receive = receive_request(context)?;
            runtime_from_wire_required_plan(
                &Value::String(receive.connection_id.clone()),
                Some(param_plan),
                "websocket connection id",
                heap,
            )
        }
        AdapterArgSource::WebSocketBusinessIdentity => {
            let receive = receive_request(context)?;
            runtime_from_wire_required_plan(
                &receive
                    .business_identity
                    .as_ref()
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null),
                Some(param_plan),
                "websocket business identity",
                heap,
            )
        }
        AdapterArgSource::HttpRequest
        | AdapterArgSource::HttpBody
        | AdapterArgSource::HttpContext => {
            Err(context.protocol_error("HTTP adapter source is not valid for websocket adapter"))
        }
    }
}

fn websocket_name_values_wire(items: &[EvalWebSocketNameValue]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| json!({ "name": item.name.as_str(), "value": item.value.as_str() }))
            .collect(),
    )
}

fn receive_request<'a>(
    context: &'a WebSocketAdapterContext<'_>,
) -> Result<&'a EvalWebSocketReceiveRequest> {
    context
        .receive_request
        .as_ref()
        .ok_or_else(|| context.protocol_error("websocket receive metadata is missing"))
}

fn validate_receive_context_codec(
    context: &WebSocketAdapterContext<'_>,
    receive: &EvalWebSocketReceiveRequest,
) -> Result<()> {
    match (
        context.context_expectation.as_ref(),
        receive.context_codec.as_ref(),
    ) {
        (Some(EvalWebSocketContextExpectation::Null), None) => Ok(()),
        (Some(EvalWebSocketContextExpectation::Null), Some(_)) => {
            Err(context
                .protocol_error("websocket receive context codec is not valid for null context"))
        }
        (
            Some(EvalWebSocketContextExpectation::Typed {
                connect_operation_abi_id,
                context_type_identity,
            }),
            Some(codec),
        ) if codec.operation_abi_id == *connect_operation_abi_id
            && codec.context_type_identity == *context_type_identity =>
        {
            Ok(())
        }
        (Some(EvalWebSocketContextExpectation::Typed { .. }), Some(_)) => {
            Err(context
                .protocol_error("websocket receive context codec does not match entry context"))
        }
        (Some(EvalWebSocketContextExpectation::Typed { .. }), None) => {
            Err(context.protocol_error("websocket receive context codec is missing"))
        }
        (None, None) => Ok(()),
        (None, Some(_)) => Err(context.protocol_error(
            "websocket receive context codec is not valid without context expectation",
        )),
    }
}

fn websocket_connection_wire(
    context: &WebSocketAdapterContext<'_>,
    receive: &EvalWebSocketReceiveRequest,
    _param_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<Value> {
    let context_value =
        match payload_segment_optional(context, EvalWebSocketPayloadSegmentKind::Context)? {
            Some(bytes) => {
                validate_receive_context_codec(context, receive)?;
                let plan = websocket_connection_context_plan(context, _param_plan)?;
                let value = decode_payload_plan(bytes, plan, &websocket_payload_boundary(), heap)?;
                runtime_to_wire_required_plan(&value, Some(plan), "websocket context", heap)?
            }
            None => Value::Null,
        };
    Ok(json!({
        "id": receive.connection_id,
        "businessIdentity": receive.business_identity,
        "context": context_value,
    }))
}

fn websocket_message_wire(
    context: &WebSocketAdapterContext<'_>,
    receive: &EvalWebSocketReceiveRequest,
) -> Result<Value> {
    let bytes = payload_segment(context, EvalWebSocketPayloadSegmentKind::Message)?;
    match receive.message.tag {
        EvalWebSocketMessageTag::Text => {
            if receive.message.encoding != EvalWebSocketMessageEncoding::Utf8 {
                return Err(context.protocol_error("websocket text message must use utf8 encoding"));
            }
            let text = std::str::from_utf8(bytes).map_err(|error| {
                RuntimeError::Decode(format!("websocket text message is not UTF-8: {error}"))
            })?;
            Ok(json!({ "tag": "text", "text": text }))
        }
        EvalWebSocketMessageTag::Binary => {
            if receive.message.encoding != EvalWebSocketMessageEncoding::Raw {
                return Err(
                    context.protocol_error("websocket binary message must use raw encoding")
                );
            }
            Ok(json!({ "tag": "binary", "base64": boundary_bytes::encode_base64(bytes) }))
        }
    }
}

fn websocket_connect_response(
    context: &WebSocketAdapterContext<'_>,
    value: &RuntimeValue,
    return_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<EvalWebSocketAdapterResult> {
    let tag = {
        let fields = runtime_object_fields(value, heap)?;
        runtime_string_field(fields, "tag", heap)?
    };
    if tag == "reject" {
        let fields = runtime_object_fields(value, heap)?;
        let code = runtime_number_field(fields, "code")?;
        let reason = runtime_string_field(fields, "reason", heap)?;
        return Ok(EvalWebSocketAdapterResult {
            payload: Vec::new(),
            response: Some(EvalWebSocketConnectResponse {
                result: EvalWebSocketConnectResult::Reject,
                business_identity: None,
                connection_policy: None,
                context_codec: None,
                context_payload_present: false,
                code: Some(code as u16),
                reason: Some(reason),
            }),
        });
    }
    if tag != "accept" {
        return Err(
            context.protocol_error(format!("websocket connect returned unsupported tag {tag}"))
        );
    }
    let (context_value, business_identity, connection_policy_value) = {
        let fields = runtime_object_fields(value, heap)?;
        (
            fields.get("context").cloned().unwrap_or(RuntimeValue::Null),
            optional_runtime_string_field(fields, "businessIdentity", heap)?,
            fields.get("connectionPolicy").cloned(),
        )
    };
    let context_plan = websocket_connect_context_plan(return_plan)?;
    let context_payload = encode_payload_plan(
        &context_value,
        context_plan,
        &websocket_payload_boundary(),
        heap,
    )?;
    let connection_policy = match connection_policy_value {
        Some(RuntimeValue::Null) | None => None,
        Some(value) => {
            let connection_policy_plan = websocket_connect_connection_policy_plan(return_plan)?;
            let wire_value = runtime_to_wire_required_plan(
                &value,
                Some(connection_policy_plan),
                "websocket connection policy",
                heap,
            )?;
            Some(
                serde_json::from_value::<WebSocketConnectionPolicyControl>(wire_value)
                    .map_err(|error| {
                        context.protocol_error(format!(
                            "websocket connection policy does not match runtime wire schema: {error}"
                        ))
                    })?,
            )
        }
    };
    Ok(EvalWebSocketAdapterResult {
        payload: context_payload,
        response: Some(EvalWebSocketConnectResponse {
            result: EvalWebSocketConnectResult::Accept,
            business_identity,
            connection_policy,
            context_codec: context.context_codec_for_connect(),
            context_payload_present: true,
            code: None,
            reason: None,
        }),
    })
}

impl WebSocketAdapterContext<'_> {
    fn context_codec_for_connect(&self) -> Option<EvalWebSocketContextCodec> {
        match self.context_expectation.as_ref()? {
            EvalWebSocketContextExpectation::Null => None,
            EvalWebSocketContextExpectation::Typed {
                connect_operation_abi_id,
                context_type_identity,
            } => Some(EvalWebSocketContextCodec {
                operation_abi_id: connect_operation_abi_id.clone(),
                context_type_identity: context_type_identity.clone(),
            }),
        }
    }
}

fn websocket_connect_context_plan(return_plan: &RuntimeTypePlan) -> Result<&RuntimeTypePlan> {
    websocket_connect_accept_field_plan(return_plan, "context")
}

fn websocket_connect_connection_policy_plan(
    return_plan: &RuntimeTypePlan,
) -> Result<&RuntimeTypePlan> {
    websocket_connect_accept_field_plan(return_plan, "connectionPolicy")
}

fn websocket_connect_accept_field_plan<'a>(
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

fn websocket_connection_context_plan<'a>(
    _context: &WebSocketAdapterContext<'_>,
    connection_plan: &'a RuntimeTypePlan,
) -> Result<&'a RuntimeTypePlan> {
    let RuntimeTypeNode::Record { fields, .. } = peel_plan(connection_plan).node() else {
        return Err(RuntimeError::Decode(
            "websocket connection type must be a record".to_string(),
        ));
    };
    field_plan(fields, "context")
}

fn websocket_receive_event_connection_plan(
    event_plan: &RuntimeTypePlan,
) -> Result<&RuntimeTypePlan> {
    let RuntimeTypeNode::Record { fields, .. } = peel_plan(event_plan).node() else {
        return Err(RuntimeError::Decode(
            "websocket receive event type must be a record".to_string(),
        ));
    };
    field_plan(fields, "connection")
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

fn payload_segment<'payload>(
    context: &WebSocketAdapterContext<'payload>,
    kind: EvalWebSocketPayloadSegmentKind,
) -> Result<&'payload [u8]> {
    payload_segment_optional(context, kind)?.ok_or_else(|| {
        context.protocol_error(format!("websocket payload segment {kind:?} is missing"))
    })
}

fn payload_segment_optional<'payload>(
    context: &WebSocketAdapterContext<'payload>,
    kind: EvalWebSocketPayloadSegmentKind,
) -> Result<Option<&'payload [u8]>> {
    let receive = receive_request(context)?;
    let matching = receive
        .payload_segments
        .iter()
        .filter(|segment| segment.kind == kind)
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [] => Ok(None),
        [segment] => segment_bytes(context, segment).map(Some),
        _ => {
            Err(context.protocol_error(format!("websocket payload segment {kind:?} is duplicated")))
        }
    }
}

fn segment_bytes<'payload>(
    context: &WebSocketAdapterContext<'payload>,
    segment: &EvalWebSocketPayloadSegment,
) -> Result<&'payload [u8]> {
    let end = segment
        .offset
        .checked_add(segment.length)
        .ok_or_else(|| context.protocol_error("websocket payload segment range overflows"))?;
    context
        .payload_bytes()
        .get(segment.offset..end)
        .ok_or_else(|| context.protocol_error("websocket payload segment range is out of bounds"))
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
            HeapNode::Bytes(_) | HeapNode::Array(_) | HeapNode::Object(_) | HeapNode::Map(_) => {
                Err(RuntimeError::Decode(format!(
                    "websocket field {name} must be string"
                )))
            }
            HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                "websocket field {name} cannot be {}",
                value.diagnostic_label()
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

#[cfg(all(test, any()))]
mod tests {
    use std::num::NonZeroU32;

    use serde_json::{json, Value};
    use skiff_runtime_boundary::type_descriptor::RuntimeTypePlanDescriptorExt;
    use skiff_runtime_capability_context::WebSocketConnectionPolicyOverflowControl;
    use skiff_runtime_model::runtime_value::RuntimeObject;

    use super::*;

    fn named(name: &str) -> Value {
        json!({ "kind": "builtin", "name": name, "args": [] })
    }

    fn generic(name: &str, args: Vec<Value>) -> Value {
        json!({ "kind": "builtin", "name": name, "args": args })
    }

    fn connect_return_plan() -> RuntimeTypePlan {
        RuntimeTypePlan::from_descriptor(&generic(
            "std.websocket.WebSocketConnectResult",
            vec![named("null")],
        ))
        .expect("connect result plan should build")
    }

    fn connect_context() -> WebSocketAdapterContext<'static> {
        WebSocketAdapterContext {
            request: RequestPayloadContext::new("test-runtime", &[], None),
            context_expectation: Some(EvalWebSocketContextExpectation::Null),
            connect_request: None,
            receive_request: None,
        }
    }

    fn object_value(fields: RuntimeObjectFields, heap: &mut RequestHeap) -> RuntimeValue {
        RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(fields))
                .expect("object should allocate"),
        )
    }

    #[test]
    fn accept_connection_policy_encodes_with_accept_field_plan() {
        let return_plan = connect_return_plan();
        let mut heap = RequestHeap::default();
        let input = json!({
            "tag": "accept",
            "context": null,
            "businessIdentity": "host-1",
            "connectionPolicy": {
                "maxConnections": 1,
                "overflow": "close-oldest",
                "closeCode": 4009,
                "closeReason": "host connection replaced",
            },
        });
        let value = runtime_from_wire_required_plan(
            &input,
            Some(&return_plan),
            "websocket connect result",
            &mut heap,
        )
        .expect("connect result should decode");

        let result =
            websocket_connect_response(&connect_context(), &value, &return_plan, &mut heap)
                .expect("accept response should encode");
        let response = result.response.expect("response should be present");

        assert_eq!(response.result, EvalWebSocketConnectResult::Accept);
        assert_eq!(response.business_identity, Some("host-1".to_string()));
        assert_eq!(
            response.connection_policy,
            Some(WebSocketConnectionPolicyControl {
                max_connections: NonZeroU32::new(1).expect("non-zero fixture"),
                overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
                close_code: Some(4009),
                close_reason: Some("host connection replaced".to_string()),
            })
        );
        assert!(response.context_payload_present);
        assert_eq!(response.context_codec, None);
    }

    #[test]
    fn accept_null_or_absent_connection_policy_stays_none() {
        let return_plan = connect_return_plan();

        for connection_policy in [None, Some(RuntimeValue::Null)] {
            let mut heap = RequestHeap::default();
            let mut fields = RuntimeObjectFields::new();
            fields.insert(
                "tag".to_string(),
                RuntimeValue::String("accept".to_string()),
            );
            fields.insert("context".to_string(), RuntimeValue::Null);
            if let Some(value) = connection_policy {
                fields.insert("connectionPolicy".to_string(), value);
            }
            let value = object_value(fields, &mut heap);

            let result =
                websocket_connect_response(&connect_context(), &value, &return_plan, &mut heap)
                    .expect("accept response should encode");
            let response = result.response.expect("response should be present");

            assert_eq!(response.result, EvalWebSocketConnectResult::Accept);
            assert_eq!(response.connection_policy, None);
            assert!(response.context_payload_present);
        }
    }

    #[test]
    fn reject_response_behavior_is_unchanged() {
        let return_plan = connect_return_plan();
        let mut heap = RequestHeap::default();
        let mut fields = RuntimeObjectFields::new();
        fields.insert(
            "tag".to_string(),
            RuntimeValue::String("reject".to_string()),
        );
        fields.insert("code".to_string(), RuntimeValue::Number(1008.0));
        fields.insert(
            "reason".to_string(),
            RuntimeValue::String("policy".to_string()),
        );
        let value = object_value(fields, &mut heap);

        let result =
            websocket_connect_response(&connect_context(), &value, &return_plan, &mut heap)
                .expect("reject response should encode");
        let response = result.response.expect("response should be present");

        assert!(result.payload.is_empty());
        assert_eq!(response.result, EvalWebSocketConnectResult::Reject);
        assert_eq!(response.connection_policy, None);
        assert_eq!(response.context_codec, None);
        assert!(!response.context_payload_present);
        assert_eq!(response.code, Some(1008));
        assert_eq!(response.reason, Some("policy".to_string()));
    }
}
