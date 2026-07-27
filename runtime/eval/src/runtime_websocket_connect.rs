use std::{collections::BTreeSet, num::NonZeroU32};

use serde_json::{json, Map, Value};
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayEntryIdentity,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayProtocolSurface, PackageCallableId,
    PackageCallableSignature, WebSocketEntryId,
};
use skiff_runtime_capability_context::{
    WebSocketConnectionPolicyControl, WebSocketConnectionPolicyOverflowControl,
};
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, LinkedExecutable};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

use crate::{
    error::{Result, RuntimeError},
    program_execution::ProgramExecutionContext,
    runtime_ops::{runtime_from_wire_required_plan, runtime_to_wire_required_plan},
    Interpreter, RuntimeAssemblyEvalTarget,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWebSocketNameValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWebSocketConnectRequest {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<RuntimeWebSocketNameValue>,
    pub headers: Vec<RuntimeWebSocketNameValue>,
    pub cookies: Vec<RuntimeWebSocketNameValue>,
    pub version: Option<String>,
    pub websocket_entry_id: WebSocketEntryId,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeWebSocketConnectResult {
    Accept {
        business_identity: Option<String>,
        connection_policy: Option<WebSocketConnectionPolicyControl>,
    },
    Reject {
        code: u16,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeWebSocketConnectCallable<'a> {
    pub callable_id: &'a PackageCallableId,
    pub signature: &'a PackageCallableSignature,
    pub addr: &'a ExecutableAddr,
}

pub trait RuntimeWebSocketConnectExecutionTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget;
    fn gateway_entry_key(&self) -> &GatewayEntryKey;
    fn gateway_entry_identity(&self) -> &GatewayEntryIdentity;
    fn websocket_entry_id(&self) -> &WebSocketEntryId;
    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface;
    fn adapter_plan(&self) -> &GatewayAdapterPlan;
    fn handler(&self) -> RuntimeWebSocketConnectCallable<'_>;
}

impl Interpreter {
    pub async fn execute_runtime_websocket_connect(
        &self,
        context: ProgramExecutionContext<'_>,
        request: &RuntimeWebSocketConnectRequest,
        target: &impl RuntimeWebSocketConnectExecutionTarget,
    ) -> Result<RuntimeWebSocketConnectResult> {
        validate_execution_pin(&context, target)?;
        validate_request(request, target)?;
        let mut heap = context.request_heap();
        let handler = target.handler();
        let args = handler_args(request, target, handler, &mut heap)?;
        let value = self
            .execute_runtime_assembly_addr(context, &mut heap, handler.addr, args)
            .await?;
        let return_plan = callable_return_plan(target, handler)?;
        let wire = runtime_to_wire_required_plan(
            &value,
            Some(&return_plan),
            "websocket connect result",
            &mut heap,
        )?;
        decode_connect_result(target, wire)
    }
}

fn validate_execution_pin(
    context: &ProgramExecutionContext<'_>,
    target: &impl RuntimeWebSocketConnectExecutionTarget,
) -> Result<()> {
    let pinned = context.runtime_assembly_target()?;
    let expected = target.eval_target();
    if !std::sync::Arc::ptr_eq(pinned.execution_image(), expected.execution_image())
        || !std::sync::Arc::ptr_eq(pinned.activation_context(), expected.activation_context())
        || pinned.request_activation().generation() != expected.request_activation().generation()
    {
        return Err(protocol_error(
            target,
            "websocket connect execution context does not match the exact eval target",
        ));
    }
    expected.ensure_execution_ready()?;
    Ok(())
}

fn validate_request(
    request: &RuntimeWebSocketConnectRequest,
    target: &impl RuntimeWebSocketConnectExecutionTarget,
) -> Result<()> {
    if !matches!(
        target.protocol_surface().protocol,
        GatewayProtocolSurface::WebSocketConnect(_)
    ) || target.adapter_plan().kind != GatewayAdapterKind::WebSocketConnect
    {
        return Err(protocol_error(
            target,
            "websocket connect execution requires the fixed connect protocol surface",
        ));
    }
    if request.websocket_entry_id != *target.websocket_entry_id()
        || request.gateway_entry_identity != *target.gateway_entry_identity()
    {
        return Err(protocol_error(
            target,
            "websocket connect request does not match the exact admitted entry identities",
        ));
    }
    Ok(())
}

fn handler_args(
    request: &RuntimeWebSocketConnectRequest,
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    handler: RuntimeWebSocketConnectCallable<'_>,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>> {
    let executable = callable_executable(target, handler)?;
    if executable.params.len() != target.adapter_plan().args.len() {
        return Err(protocol_error(
            target,
            "websocket connect adapter plan does not cover the exact handler signature",
        ));
    }
    let mut values = Vec::with_capacity(executable.params.len());
    for parameter in &executable.params {
        let arg = target
            .adapter_plan()
            .args
            .iter()
            .find(|arg| arg.param == parameter.name)
            .ok_or_else(|| {
                protocol_error(
                    target,
                    format!(
                        "websocket connect adapter plan is missing parameter {}",
                        parameter.name
                    ),
                )
            })?;
        let wire = websocket_connect_source_wire(request, target, arg.source)?;
        let plan = RuntimeTypePlan::from_linked_nested_ref(
            &parameter.ty,
            &PlanContext::from_type_view(
                target.eval_target().execution_projection().type_view(),
                handler.addr,
            ),
        )?;
        values.push(runtime_from_wire_required_plan(
            &wire,
            Some(&plan),
            "websocket connect adapter argument",
            heap,
        )?);
    }
    Ok(values)
}

fn websocket_connect_source_wire(
    request: &RuntimeWebSocketConnectRequest,
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    source: GatewayAdapterSource,
) -> Result<Value> {
    match source {
        GatewayAdapterSource::WebSocketConnectRequest => Ok(json!({
            "connectionId": request.connection_id,
            "url": request.url,
            "query": name_values_wire(&request.query),
            "headers": name_values_wire(&request.headers),
            "cookies": name_values_wire(&request.cookies),
            "version": request.version,
            "websocketEntryId": request.websocket_entry_id.as_str(),
            "gatewayEntryIdentity": request.gateway_entry_identity.as_str(),
        })),
        GatewayAdapterSource::WebSocketConnectionId => {
            Ok(Value::String(request.connection_id.clone()))
        }
        GatewayAdapterSource::HttpRequest
        | GatewayAdapterSource::HttpBody
        | GatewayAdapterSource::HttpContext => Err(protocol_error(
            target,
            "websocket connect execution refuses HTTP adapter sources",
        )),
        GatewayAdapterSource::WebSocketJsonRpcParams
        | GatewayAdapterSource::WebSocketBusinessIdentity => Err(protocol_error(
            target,
            "websocket connect execution refuses WebSocket JSON-RPC-only adapter sources",
        )),
    }
}

fn name_values_wire(items: &[RuntimeWebSocketNameValue]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| json!({ "name": item.name, "value": item.value }))
            .collect(),
    )
}

fn callable_executable<'a>(
    target: &'a impl RuntimeWebSocketConnectExecutionTarget,
    callable: RuntimeWebSocketConnectCallable<'a>,
) -> Result<&'a LinkedExecutable> {
    let resolved = target
        .eval_target()
        .execution_projection()
        .resolve_executable(callable.addr)?;
    if resolved.addr != *callable.addr
        || resolved.executable.kind != ExecutableKind::Function
        || resolved.executable.self_type.is_some()
        || resolved.executable.return_type.is_none()
        || resolved.executable.type_params != callable.signature.type_params
        || resolved.executable.may_suspend != callable.signature.may_suspend
        || resolved.executable.params.len() != callable.signature.parameters.len()
        || resolved
            .executable
            .params
            .iter()
            .zip(&callable.signature.parameters)
            .any(|(linked, declared)| linked.name != declared.name)
    {
        return Err(protocol_error(
            target,
            format!(
                "websocket connect callable {} does not match its exact linked signature",
                callable.callable_id
            ),
        ));
    }
    Ok(resolved.executable)
}

fn callable_return_plan(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    callable: RuntimeWebSocketConnectCallable<'_>,
) -> Result<RuntimeTypePlan> {
    let executable = callable_executable(target, callable)?;
    let return_type = executable.return_type.as_ref().ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "websocket connect callable {} is missing its linked return type",
            callable.callable_id
        ))
    })?;
    RuntimeTypePlan::from_linked(
        return_type,
        &PlanContext::from_type_view(
            target.eval_target().execution_projection().type_view(),
            callable.addr,
        ),
    )
    .map_err(Into::into)
}

fn decode_connect_result(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    wire: Value,
) -> Result<RuntimeWebSocketConnectResult> {
    let fields = wire
        .as_object()
        .ok_or_else(|| protocol_error(target, "websocket connect result must be an object"))?;
    let tag = required_string(target, fields, "tag")?;
    match tag {
        "accept" => {
            require_exact_fields(
                target,
                fields,
                &["tag", "businessIdentity", "connectionPolicy"],
            )?;
            let business_identity = optional_string(target, fields, "businessIdentity")?;
            let connection_policy = match fields.get("connectionPolicy") {
                Some(Value::Null) => None,
                Some(value) => Some(decode_connection_policy(target, value)?),
                None => {
                    return Err(protocol_error(
                        target,
                        "websocket connect accept omitted connectionPolicy",
                    ))
                }
            };
            Ok(RuntimeWebSocketConnectResult::Accept {
                business_identity,
                connection_policy,
            })
        }
        "reject" => {
            require_exact_fields(target, fields, &["tag", "code", "reason"])?;
            let code = fields
                .get("code")
                .and_then(Value::as_u64)
                .and_then(|code| u16::try_from(code).ok())
                .ok_or_else(|| {
                    protocol_error(
                        target,
                        "websocket connect reject code must be an unsigned 16-bit integer",
                    )
                })?;
            let reason = required_string(target, fields, "reason")?.to_string();
            Ok(RuntimeWebSocketConnectResult::Reject { code, reason })
        }
        other => Err(protocol_error(
            target,
            format!("websocket connect returned unsupported tag {other}"),
        )),
    }
}

fn decode_connection_policy(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    value: &Value,
) -> Result<WebSocketConnectionPolicyControl> {
    let fields = value
        .as_object()
        .ok_or_else(|| protocol_error(target, "websocket connectionPolicy must be an object"))?;
    require_exact_fields(
        target,
        fields,
        &["maxConnections", "overflow", "closeCode", "closeReason"],
    )?;
    let max_connections = fields
        .get("maxConnections")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .and_then(NonZeroU32::new)
        .ok_or_else(|| {
            protocol_error(
                target,
                "websocket connectionPolicy.maxConnections must be a non-zero u32",
            )
        })?;
    let overflow = match required_string(target, fields, "overflow")? {
        "close-oldest" => WebSocketConnectionPolicyOverflowControl::CloseOldest,
        "reject-new" => WebSocketConnectionPolicyOverflowControl::RejectNew,
        _ => {
            return Err(protocol_error(
                target,
                "websocket connectionPolicy.overflow is invalid",
            ))
        }
    };
    let close_code = optional_u16(target, fields, "closeCode")?;
    let close_reason = optional_string(target, fields, "closeReason")?;
    Ok(WebSocketConnectionPolicyControl {
        max_connections,
        overflow,
        close_code,
        close_reason,
    })
}

fn require_exact_fields(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    fields: &Map<String, Value>,
    expected: &[&str],
) -> Result<()> {
    let actual = fields.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(protocol_error(
            target,
            "websocket connect result fields are not the exact canonical shape",
        ));
    }
    Ok(())
}

fn required_string<'a>(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    fields: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str> {
    fields.get(name).and_then(Value::as_str).ok_or_else(|| {
        protocol_error(
            target,
            format!("websocket connect field {name} must be a string"),
        )
    })
}

fn optional_string(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    fields: &Map<String, Value>,
    name: &str,
) -> Result<Option<String>> {
    match fields.get(name) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(protocol_error(
            target,
            format!("websocket connect field {name} must be a string or null"),
        )),
        None => Err(protocol_error(
            target,
            format!("websocket connect result omitted {name}"),
        )),
    }
}

fn optional_u16(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    fields: &Map<String, Value>,
    name: &str,
) -> Result<Option<u16>> {
    match fields.get(name) {
        Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| {
                protocol_error(
                    target,
                    format!("websocket connect field {name} must be an unsigned u16 or null"),
                )
            }),
        Some(_) => Err(protocol_error(
            target,
            format!("websocket connect field {name} must be an unsigned u16 or null"),
        )),
        None => Err(protocol_error(
            target,
            format!("websocket connect result omitted {name}"),
        )),
    }
}

fn protocol_error(
    target: &impl RuntimeWebSocketConnectExecutionTarget,
    message: impl Into<String>,
) -> RuntimeError {
    RuntimeError::Protocol {
        target: target.gateway_entry_key().as_str().to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::WEBSOCKET_GATEWAY_ENTRY_KEY;

    struct DecodeTarget {
        key: GatewayEntryKey,
    }

    impl DecodeTarget {
        fn new() -> Self {
            Self {
                key: GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap(),
            }
        }
    }

    impl RuntimeWebSocketConnectExecutionTarget for DecodeTarget {
        fn eval_target(&self) -> &RuntimeAssemblyEvalTarget {
            panic!("result decoding does not consult an eval target")
        }

        fn gateway_entry_key(&self) -> &GatewayEntryKey {
            &self.key
        }

        fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
            panic!("result decoding does not consult the gateway identity")
        }

        fn websocket_entry_id(&self) -> &WebSocketEntryId {
            panic!("result decoding does not consult the internal entry id")
        }

        fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
            panic!("result decoding does not consult the protocol surface")
        }

        fn adapter_plan(&self) -> &GatewayAdapterPlan {
            panic!("result decoding does not consult the adapter plan")
        }

        fn handler(&self) -> RuntimeWebSocketConnectCallable<'_> {
            panic!("result decoding does not consult the handler")
        }
    }

    #[test]
    fn websocket_connect_result_decodes_accept_with_optional_identity_and_policy() {
        let target = DecodeTarget::new();
        assert_eq!(
            decode_connect_result(
                &target,
                json!({
                    "tag": "accept",
                    "businessIdentity": "tenant-1",
                    "connectionPolicy": {
                        "maxConnections": 3,
                        "overflow": "close-oldest",
                        "closeCode": 4001,
                        "closeReason": "replaced"
                    }
                })
            )
            .unwrap(),
            RuntimeWebSocketConnectResult::Accept {
                business_identity: Some("tenant-1".to_string()),
                connection_policy: Some(WebSocketConnectionPolicyControl {
                    max_connections: NonZeroU32::new(3).unwrap(),
                    overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
                    close_code: Some(4001),
                    close_reason: Some("replaced".to_string()),
                }),
            }
        );
        assert_eq!(
            decode_connect_result(
                &target,
                json!({
                    "tag": "accept",
                    "businessIdentity": null,
                    "connectionPolicy": null
                })
            )
            .unwrap(),
            RuntimeWebSocketConnectResult::Accept {
                business_identity: None,
                connection_policy: None,
            }
        );
    }

    #[test]
    fn websocket_connect_result_decodes_reject_and_refuses_noncanonical_shapes() {
        let target = DecodeTarget::new();
        assert_eq!(
            decode_connect_result(
                &target,
                json!({"tag": "reject", "code": 1008, "reason": "policy"})
            )
            .unwrap(),
            RuntimeWebSocketConnectResult::Reject {
                code: 1008,
                reason: "policy".to_string(),
            }
        );

        for invalid in [
            json!({"tag": "accept", "businessIdentity": null}),
            json!({
                "tag": "accept",
                "businessIdentity": null,
                "connectionPolicy": {
                    "maxConnections": 0,
                    "overflow": "close-oldest",
                    "closeCode": null,
                    "closeReason": null
                }
            }),
            json!({"tag": "reject", "code": 65536, "reason": "policy"}),
            json!({"tag": "reject", "code": 1008, "reason": "policy", "legacy": true}),
        ] {
            assert!(decode_connect_result(&target, invalid).is_err());
        }
    }

    #[test]
    fn native_websocket_connect_refuses_jsonrpc_only_sources_before_value_projection() {
        let target = DecodeTarget::new();
        let request = RuntimeWebSocketConnectRequest {
            connection_id: "connection-1".to_string(),
            url: "ws://websocket.test/ws".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            version: None,
            websocket_entry_id: WebSocketEntryId::parse(format!(
                "skiff-websocket-entry-v1:sha256:{}",
                "a".repeat(64)
            ))
            .unwrap(),
            gateway_entry_identity: GatewayEntryIdentity::parse(format!(
                "skiff-gateway-entry-v2:sha256:{}",
                "b".repeat(64)
            ))
            .unwrap(),
        };

        for source in [
            GatewayAdapterSource::WebSocketJsonRpcParams,
            GatewayAdapterSource::WebSocketBusinessIdentity,
        ] {
            let error = websocket_connect_source_wire(&request, &target, source)
                .expect_err("connect evaluator must reject JSON-RPC-only sources");
            assert!(
                error
                    .to_string()
                    .contains("WebSocket JSON-RPC-only adapter sources"),
                "{error}"
            );
        }
    }
}
