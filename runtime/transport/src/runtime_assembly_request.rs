use std::num::NonZeroU32;

use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_model::{
    AssemblyIdentity, GatewayEntryIdentity, ServiceDeploymentRef, WebSocketEntryId,
};

use crate::connection_protocol::CONNECTION_REQUEST_MAX_PAYLOAD_BYTES;
use crate::{BinaryFrameError, TransportError};

mod lexical;
mod metadata;
mod strict_json;

use lexical::{
    deserialize_assembly_identity, deserialize_dispatch_mode, deserialize_gateway_caller_kind,
    deserialize_gateway_entry_identity, deserialize_optional_test_case_capability,
    deserialize_optional_test_case_parent_request_id,
    deserialize_optional_websocket_jsonrpc_business_identity, deserialize_request_start_type,
    deserialize_response_end_type, deserialize_runtime_assembly_routing_kind,
    deserialize_runtime_assembly_websocket_jsonrpc_connection_id,
    deserialize_runtime_assembly_websocket_jsonrpc_method,
    deserialize_runtime_assembly_websocket_jsonrpc_request_id,
    deserialize_runtime_frame_schema_version, deserialize_safe_activation_generation,
    deserialize_service_caller_kind, deserialize_service_deployment_ref,
    deserialize_task_invocation_kind, deserialize_task_target, deserialize_task_target_kind,
    deserialize_task_unary_dispatch_mode, deserialize_unary_dispatch_mode,
    deserialize_websocket_jsonrpc_unary_dispatch_mode,
};
use metadata::deserialize_present_option;
pub use metadata::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestStartFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    pub schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_request_start_type")]
    pub frame_type: String,
    pub request_id: String,
    #[serde(deserialize_with = "deserialize_dispatch_mode")]
    pub mode: String,
    pub caller: RuntimeAssemblyRequestCallerFrameHeader,
    pub routing: RuntimeAssemblyRequestRoutingFrameHeader,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_session: Option<RuntimeAssemblyRequestClientSessionFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    pub trace: RuntimeAssemblyRequestTraceFrameHeader,
    pub http_request: RuntimeAssemblyHttpRequestFrameHeader,
    #[serde(default)]
    pub test_effects_enabled: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_test_case_capability",
        skip_serializing_if = "Option::is_none"
    )]
    pub test_case_capability: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_test_case_parent_request_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub test_case_parent_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RuntimeAssemblyRequestStartFrameWireHeader {
    Http(RuntimeAssemblyRequestStartFrameHeader),
    WebSocketConnect(RuntimeAssemblyWebSocketConnectRequestStartFrameHeader),
    WebSocketJsonRpc(RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader),
    Task(RuntimeAssemblyTaskRequestStartFrameHeader),
}

impl<'de> Deserialize<'de> for RuntimeAssemblyRequestStartFrameWireHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value
            .get("invocation")
            .and_then(|invocation| invocation.get("kind"))
            .and_then(serde_json::Value::as_str)
            == Some("task")
        {
            return serde_json::from_value(value)
                .map(Self::Task)
                .map_err(de::Error::custom);
        }
        let protocol = value
            .get("routing")
            .and_then(|routing| routing.get("ingress"))
            .and_then(|ingress| ingress.get("protocol"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                de::Error::custom("request.start routing.ingress.protocol is required")
            })?;
        match protocol {
            "http" => serde_json::from_value(value)
                .map(Self::Http)
                .map_err(de::Error::custom),
            "webSocket" => match value
                .get("routing")
                .and_then(|routing| routing.get("ingress"))
                .and_then(|ingress| ingress.get("method"))
            {
                Some(serde_json::Value::Null) => serde_json::from_value(value)
                    .map(Self::WebSocketConnect)
                    .map_err(de::Error::custom),
                Some(serde_json::Value::String(_)) => serde_json::from_value(value)
                    .map(Self::WebSocketJsonRpc)
                    .map_err(de::Error::custom),
                _ => Err(de::Error::custom(
                    "request.start WebSocket routing.ingress.method must be null or a string",
                )),
            },
            _ => Err(de::Error::custom(
                "request.start routing.ingress.protocol must be http or webSocket",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyTaskRequestStartFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    pub schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_request_start_type")]
    pub frame_type: String,
    pub request_id: String,
    #[serde(deserialize_with = "deserialize_task_unary_dispatch_mode")]
    pub mode: String,
    pub caller: RuntimeAssemblyTaskRequestCallerFrameHeader,
    pub routing: RuntimeAssemblyTaskRequestRoutingFrameHeader,
    pub invocation: RuntimeAssemblyTaskInvocationFrameHeader,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    pub trace: RuntimeAssemblyRequestTraceFrameHeader,
    pub test_effects_enabled: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_test_case_capability",
        skip_serializing_if = "Option::is_none"
    )]
    pub test_case_capability: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_attempt: Option<RuntimeAssemblyTaskAttemptFrameHeader>,
}

/// Optional task-attempt association on a task `request.start` (D1 wire
/// contract §5). When present it carries the durable task identity, the
/// current claim's `AttemptId` and the lease id so the Router can map a
/// terminal outcome back to task settlement. Settlement itself is D2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyTaskAttemptFrameHeader {
    pub task_id: String,
    pub attempt_id: String,
    pub lease_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyTaskRequestCallerFrameHeader {
    #[serde(deserialize_with = "deserialize_service_caller_kind")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyTaskRequestRoutingFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_assembly_routing_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_assembly_identity")]
    pub assembly_identity: AssemblyIdentity,
    #[serde(deserialize_with = "deserialize_safe_activation_generation")]
    pub assembly_generation: u64,
    #[serde(deserialize_with = "deserialize_service_deployment_ref")]
    pub deployment: ServiceDeploymentRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyTaskInvocationFrameHeader {
    #[serde(deserialize_with = "deserialize_task_invocation_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_task_target_kind")]
    pub target_kind: String,
    #[serde(deserialize_with = "deserialize_task_target")]
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    pub schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_request_start_type")]
    pub frame_type: String,
    pub request_id: String,
    #[serde(deserialize_with = "deserialize_unary_dispatch_mode")]
    pub mode: String,
    pub caller: RuntimeAssemblyRequestCallerFrameHeader,
    pub routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_session: Option<RuntimeAssemblyRequestClientSessionFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    pub trace: RuntimeAssemblyRequestTraceFrameHeader,
    pub websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader,
    #[serde(default)]
    pub test_effects_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    pub schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_request_start_type")]
    pub frame_type: String,
    #[serde(deserialize_with = "deserialize_runtime_assembly_websocket_jsonrpc_request_id")]
    pub request_id: String,
    #[serde(deserialize_with = "deserialize_websocket_jsonrpc_unary_dispatch_mode")]
    pub mode: String,
    pub caller: RuntimeAssemblyRequestCallerFrameHeader,
    pub routing: RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_session: Option<RuntimeAssemblyRequestClientSessionFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    pub trace: RuntimeAssemblyRequestTraceFrameHeader,
    pub websocket_json_rpc: RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader,
    #[serde(default)]
    pub test_effects_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestCallerFrameHeader {
    #[serde(deserialize_with = "deserialize_gateway_caller_kind")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestRoutingFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_assembly_routing_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_assembly_identity")]
    pub assembly_identity: AssemblyIdentity,
    #[serde(deserialize_with = "deserialize_safe_activation_generation")]
    pub assembly_generation: u64,
    #[serde(deserialize_with = "deserialize_service_deployment_ref")]
    pub deployment: ServiceDeploymentRef,
    #[serde(deserialize_with = "deserialize_gateway_entry_identity")]
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub ingress: RuntimeAssemblyRequestIngressFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_assembly_routing_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_assembly_identity")]
    pub assembly_identity: AssemblyIdentity,
    #[serde(deserialize_with = "deserialize_safe_activation_generation")]
    pub assembly_generation: u64,
    #[serde(deserialize_with = "deserialize_service_deployment_ref")]
    pub deployment: ServiceDeploymentRef,
    #[serde(deserialize_with = "deserialize_gateway_entry_identity")]
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_assembly_routing_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_assembly_identity")]
    pub assembly_identity: AssemblyIdentity,
    #[serde(deserialize_with = "deserialize_safe_activation_generation")]
    pub assembly_generation: u64,
    #[serde(deserialize_with = "deserialize_service_deployment_ref")]
    pub deployment: ServiceDeploymentRef,
    #[serde(deserialize_with = "deserialize_gateway_entry_identity")]
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub ingress: RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestIngressFrameHeader {
    pub protocol: RuntimeAssemblyRequestIngressProtocol,
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyRequestIngressProtocol {
    #[serde(rename = "http")]
    Http,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyWebSocketConnectIngressProtocol {
    #[serde(rename = "webSocket")]
    WebSocket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectIngressFrameHeader {
    pub protocol: RuntimeAssemblyWebSocketConnectIngressProtocol,
    pub method: (),
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader {
    pub protocol: RuntimeAssemblyWebSocketConnectIngressProtocol,
    pub method: String,
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketConnectIngressFrameHeader {
    protocol: RuntimeAssemblyWebSocketConnectIngressProtocol,
    method: (),
    path: String,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyWebSocketConnectIngressFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyWebSocketConnectIngressFrameHeader::deserialize(deserializer)?;
        if !raw.path.starts_with('/') {
            return Err(de::Error::custom(
                "routing.ingress.path must be an absolute path",
            ));
        }
        Ok(Self {
            protocol: raw.protocol,
            method: raw.method,
            path: raw.path,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketJsonRpcIngressFrameHeader {
    protocol: RuntimeAssemblyWebSocketConnectIngressProtocol,
    #[serde(deserialize_with = "deserialize_runtime_assembly_websocket_jsonrpc_method")]
    method: String,
    path: String,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyWebSocketJsonRpcIngressFrameHeader::deserialize(deserializer)?;
        if !raw.path.starts_with('/') {
            return Err(de::Error::custom(
                "routing.ingress.path must be an absolute path",
            ));
        }
        Ok(Self {
            protocol: raw.protocol,
            method: raw.method,
            path: raw.path,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyRequestIngressFrameHeader {
    protocol: RuntimeAssemblyRequestIngressProtocol,
    method: String,
    path: String,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyRequestIngressFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyRequestIngressFrameHeader::deserialize(deserializer)?;
        if raw.method.is_empty() {
            return Err(de::Error::custom(
                "routing.ingress.method must be a non-empty string",
            ));
        }
        if !raw.path.starts_with('/') {
            return Err(de::Error::custom(
                "routing.ingress.path must be an absolute path",
            ));
        }
        Ok(Self {
            protocol: raw.protocol,
            method: raw.method,
            path: raw.path,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectRequestFrameHeader {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub headers: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub cookies: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub websocket_entry_id: WebSocketEntryId,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketConnectRequestFrameHeader {
    connection_id: String,
    url: String,
    query: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    headers: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    cookies: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    version: Option<String>,
    websocket_entry_id: WebSocketEntryId,
    gateway_entry_identity: GatewayEntryIdentity,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyWebSocketConnectRequestFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyWebSocketConnectRequestFrameHeader::deserialize(deserializer)?;
        if raw.connection_id.is_empty()
            || raw.connection_id.len() > 255
            || !raw.connection_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'~' | b'-')
            })
        {
            return Err(de::Error::custom(
                "websocketConnect.connectionId is not canonical",
            ));
        }
        Ok(Self {
            connection_id: raw.connection_id,
            url: raw.url,
            query: raw.query,
            headers: raw.headers,
            cookies: raw.cookies,
            version: raw.version,
            websocket_entry_id: raw.websocket_entry_id,
            gateway_entry_identity: raw.gateway_entry_identity,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyWebSocketJsonRpcProfile {
    #[serde(rename = "jsonrpc-2.0-text")]
    JsonRpc2_0Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader {
    pub profile: RuntimeAssemblyWebSocketJsonRpcProfile,
    #[serde(deserialize_with = "deserialize_runtime_assembly_websocket_jsonrpc_connection_id")]
    pub connection_id: String,
    pub websocket_entry_id: WebSocketEntryId,
    pub gateway_entry_identity: GatewayEntryIdentity,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_websocket_jsonrpc_business_identity",
        skip_serializing_if = "Option::is_none"
    )]
    pub business_identity: Option<String>,
}

pub fn decode_runtime_assembly_request_start_frame(
    frame: &[u8],
) -> Result<(RuntimeAssemblyRequestStartFrameWireHeader, Vec<u8>), BinaryFrameError> {
    let (header, payload) = strict_json::decode_runtime_assembly_request_json_frame(frame)?;
    let header: RuntimeAssemblyRequestStartFrameWireHeader = serde_json::from_value(header)
        .map_err(|error| {
            TransportError::decode(format!(
                "invalid skiff binary frame: header failed typed decode: {error}"
            ))
        })?;
    match &header {
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(websocket) => {
            if websocket.websocket_connect.gateway_entry_identity
                != websocket.routing.gateway_entry_identity
            {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly websocketConnect request.start frame: websocketConnect.gatewayEntryIdentity must match routing.gatewayEntryIdentity",
                ));
            }
            if !payload.is_empty() {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly websocketConnect request.start frame: payload must be empty",
                ));
            }
        }
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(websocket) => {
            if websocket.websocket_json_rpc.gateway_entry_identity
                != websocket.routing.gateway_entry_identity
            {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly websocketJsonRpc request.start frame: websocketJsonRpc.gatewayEntryIdentity must match routing.gatewayEntryIdentity",
                ));
            }
            if payload.is_empty() || payload.len() > CONNECTION_REQUEST_MAX_PAYLOAD_BYTES {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly websocketJsonRpc request.start frame: payload must be present and within the payload limit",
                ));
            }
        }
        RuntimeAssemblyRequestStartFrameWireHeader::Task(task) => {
            if payload.is_empty() {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly task request.start frame: recoverable args payload must be present",
                ));
            }
            if task.request_id.is_empty() {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly task request.start frame: requestId must be non-empty",
                ));
            }
            if let Some(attempt) = &task.task_attempt {
                if attempt.task_id.trim().is_empty()
                    || attempt.attempt_id.trim().is_empty()
                    || attempt.lease_id.trim().is_empty()
                {
                    return Err(TransportError::decode(
                        "invalid runtimeAssembly task request.start frame: taskAttempt taskId/attemptId/leaseId must be non-empty",
                    ));
                }
            }
            if task.test_effects_enabled != task.test_case_capability.is_some() {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly task request.start frame: testEffectsEnabled must match testCaseCapability presence",
                ));
            }
        }
        RuntimeAssemblyRequestStartFrameWireHeader::Http(http) => {
            if http.test_effects_enabled != http.test_case_capability.is_some() {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly HTTP request.start frame: testEffectsEnabled must match testCaseCapability presence",
                ));
            }
            if http.test_case_parent_request_id.is_some() && http.test_case_capability.is_none() {
                return Err(TransportError::decode(
                    "invalid runtimeAssembly HTTP request.start frame: testCaseParentRequestId requires testCaseCapability",
                ));
            }
        }
    }
    Ok((header, payload))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum RuntimeAssemblyWebSocketConnectResponseFrameHeader {
    #[serde(rename = "accept")]
    Accept {
        #[serde(
            default,
            deserialize_with = "deserialize_present_option",
            skip_serializing_if = "Option::is_none"
        )]
        business_identity: Option<String>,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_positive_safe_admission_rank",
            skip_serializing_if = "Option::is_none"
        )]
        admission_rank: Option<u64>,
        #[serde(
            default,
            deserialize_with = "deserialize_present_option",
            skip_serializing_if = "Option::is_none"
        )]
        connection_policy: Option<RuntimeAssemblyWebSocketConnectionPolicyFrameHeader>,
    },
    #[serde(rename = "reject")]
    Reject { code: u16, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectionPolicyFrameHeader {
    pub max_connections: NonZeroU32,
    pub overflow: RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub close_code: Option<u16>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader {
    CloseOldest,
    RejectNew,
}

fn deserialize_optional_positive_safe_admission_rank<'de, D>(
    deserializer: D,
) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    let value = u64::deserialize(deserializer)?;
    if (1..=MAX_SAFE_INTEGER).contains(&value) {
        Ok(Some(value))
    } else {
        Err(de::Error::custom(
            "websocketConnect.admissionRank must be a positive safe integer",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader",
    into = "RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader"
)]
pub struct RuntimeAssemblyWebSocketConnectResponseEndFrameHeader {
    pub schema_version: String,
    pub frame_type: String,
    pub request_id: String,
    pub websocket_connect: RuntimeAssemblyWebSocketConnectResponseFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_response_end_type")]
    frame_type: String,
    request_id: String,
    payload_present: bool,
    websocket_connect: RuntimeAssemblyWebSocketConnectResponseFrameHeader,
}

impl TryFrom<RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader>
    for RuntimeAssemblyWebSocketConnectResponseEndFrameHeader
{
    type Error = String;

    fn try_from(
        raw: RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader,
    ) -> Result<Self, Self::Error> {
        if raw.payload_present {
            return Err(
                "runtimeAssembly websocketConnect response.end payloadPresent must be false"
                    .to_string(),
            );
        }
        Ok(Self {
            schema_version: raw.schema_version,
            frame_type: raw.frame_type,
            request_id: raw.request_id,
            websocket_connect: raw.websocket_connect,
        })
    }
}

impl From<RuntimeAssemblyWebSocketConnectResponseEndFrameHeader>
    for RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader
{
    fn from(header: RuntimeAssemblyWebSocketConnectResponseEndFrameHeader) -> Self {
        Self {
            schema_version: header.schema_version,
            frame_type: header.frame_type,
            request_id: header.request_id,
            payload_present: false,
            websocket_connect: header.websocket_connect,
        }
    }
}

pub fn decode_runtime_assembly_websocket_connect_response_end_frame(
    frame: &[u8],
) -> Result<RuntimeAssemblyWebSocketConnectResponseEndFrameHeader, BinaryFrameError> {
    let (header, payload) = strict_json::decode_runtime_assembly_json_frame(
        frame,
        "runtimeAssembly websocketConnect response.end",
    )?;
    if !payload.is_empty() {
        return Err(TransportError::decode(
            "invalid runtimeAssembly websocketConnect response.end frame: payload must be empty",
        ));
    }
    serde_json::from_value(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid runtimeAssembly websocketConnect response.end frame: header failed typed decode: {error}"
        ))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeAssemblyWebSocketJsonRpcResponseOutcome {
    Success,
    InvalidParams,
    InternalError,
    DeadlineExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketJsonRpcResponseFrameHeader {
    pub outcome: RuntimeAssemblyWebSocketJsonRpcResponseOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "RawRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader",
    into = "RawRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader"
)]
pub struct RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader {
    pub schema_version: String,
    pub frame_type: String,
    pub request_id: String,
    pub websocket_json_rpc: RuntimeAssemblyWebSocketJsonRpcResponseFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_response_end_type")]
    frame_type: String,
    #[serde(deserialize_with = "deserialize_runtime_assembly_websocket_jsonrpc_request_id")]
    request_id: String,
    payload_present: bool,
    websocket_json_rpc: RuntimeAssemblyWebSocketJsonRpcResponseFrameHeader,
}

impl TryFrom<RawRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader>
    for RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader
{
    type Error = String;

    fn try_from(
        raw: RawRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
    ) -> Result<Self, Self::Error> {
        let expected_payload_present = matches!(
            raw.websocket_json_rpc.outcome,
            RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
        );
        if raw.payload_present != expected_payload_present {
            return Err(
                "runtimeAssembly websocketJsonRpc response.end payloadPresent must match outcome"
                    .to_string(),
            );
        }
        Ok(Self {
            schema_version: raw.schema_version,
            frame_type: raw.frame_type,
            request_id: raw.request_id,
            websocket_json_rpc: raw.websocket_json_rpc,
        })
    }
}

impl From<RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader>
    for RawRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader
{
    fn from(header: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader) -> Self {
        let payload_present = matches!(
            header.websocket_json_rpc.outcome,
            RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
        );
        Self {
            schema_version: header.schema_version,
            frame_type: header.frame_type,
            request_id: header.request_id,
            payload_present,
            websocket_json_rpc: header.websocket_json_rpc,
        }
    }
}

pub fn encode_runtime_assembly_websocket_jsonrpc_response_end_frame(
    header: &RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    let value = serde_json::to_value(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid runtimeAssembly websocketJsonRpc response.end frame: header failed typed encode: {error}"
        ))
    })?;
    let validated: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader =
        serde_json::from_value(value).map_err(|error| {
            TransportError::decode(format!(
                "invalid runtimeAssembly websocketJsonRpc response.end frame: header failed typed validation: {error}"
            ))
        })?;
    validate_runtime_assembly_websocket_jsonrpc_response_payload(&validated, payload)?;
    crate::protocol::encode_binary_frame(&validated, payload)
}

pub fn decode_runtime_assembly_websocket_jsonrpc_response_end_frame(
    frame: &[u8],
) -> Result<
    (
        RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
        Vec<u8>,
    ),
    BinaryFrameError,
> {
    let (header, payload) = strict_json::decode_runtime_assembly_json_frame(
        frame,
        "runtimeAssembly websocketJsonRpc response.end",
    )?;
    let header: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader =
        serde_json::from_value(header).map_err(|error| {
            TransportError::decode(format!(
                "invalid runtimeAssembly websocketJsonRpc response.end frame: header failed typed decode: {error}"
            ))
        })?;
    validate_runtime_assembly_websocket_jsonrpc_response_payload(&header, &payload)?;
    Ok((header, payload))
}

fn validate_runtime_assembly_websocket_jsonrpc_response_payload(
    header: &RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    if payload.len() > CONNECTION_REQUEST_MAX_PAYLOAD_BYTES {
        return Err(TransportError::decode(
            "invalid runtimeAssembly websocketJsonRpc response.end frame: payload exceeds the payload limit",
        ));
    }
    let payload_present = !payload.is_empty();
    let expected_payload_present = matches!(
        header.websocket_json_rpc.outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
    );
    if payload_present != expected_payload_present {
        return Err(TransportError::decode(
            "invalid runtimeAssembly websocketJsonRpc response.end frame: payload presence must match outcome",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_assembly_request/tests.rs"]
mod tests;
