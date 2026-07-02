use std::{collections::HashMap, num::NonZeroU32};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum RouterWriterMessage {
    Binary(Vec<u8>),
    Control(OutboundControlMessage),
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutboundControlMessage {
    ActorPut {
        request: ActorPutControlRequest,
        payload: Vec<u8>,
    },
    ActorFind {
        request: ActorFindControlRequest,
    },
    ActorRemove {
        request: ActorRemoveControlRequest,
    },
    SpawnSubmit {
        request: SpawnSubmitControlRequest,
        payload: Vec<u8>,
    },
    RequestStart {
        request: RequestStartControl,
        payload: Vec<u8>,
    },
    RequestCancel {
        request: RequestCancelControl,
    },
    ConnectionSend {
        request: ConnectionSendControl,
        payload: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorKeyControlMetadata {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorPutControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub actor_key: ActorKeyControlMetadata,
    pub object_schema_identity: String,
    pub object_encoding_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorFindControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub actor_key: ActorKeyControlMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRemoveControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub actor_key: ActorKeyControlMetadata,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpawnSubmitControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub target_kind: String,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    pub target: String,
    pub spawn_id: Option<String>,
    pub build_id: Option<String>,
    pub activation_identity: Option<String>,
    pub caller_request_id: Option<String>,
    pub trace_id: Option<String>,
    pub caller_target: Option<String>,
    pub max_queue_wait_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestStartControl {
    pub request_id: String,
    pub mode: String,
    pub caller: RuntimeCallerControl,
    pub target: String,
    pub operation_abi_id: Option<String>,
    pub selector: Option<String>,
    pub service_id: Option<String>,
    pub version: Option<String>,
    pub build_id: String,
    pub service_protocol_identity: String,
    pub activation_identity: Option<String>,
    pub gateway_entry_identity: Option<String>,
    pub business_identity: Option<String>,
    pub websocket_entry_id: Option<String>,
    pub client_session: Option<RuntimeClientSessionControl>,
    pub deadline: Option<RuntimeDeadlineControl>,
    pub trace: RuntimeTraceContextControl,
    pub test_effects_enabled: bool,
    pub test_effect_doubles: HashMap<String, Vec<RequestEffectDoubleControl>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeClientSessionControl {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSocketConnectionPolicyControl {
    pub max_connections: NonZeroU32,
    pub overflow: WebSocketConnectionPolicyOverflowControl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketConnectionPolicyOverflowControl {
    #[serde(rename = "close-oldest")]
    CloseOldest,
    #[serde(rename = "reject-new")]
    RejectNew,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCallerControl {
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDeadlineControl {
    pub timeout_ms: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTraceContextControl {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub sampled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestEffectDoubleControl {
    pub expect_request: Option<Value>,
    pub response: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestCancelControl {
    pub request_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSendControl {
    pub service_id: String,
    pub websocket_entry_id: Option<String>,
    pub business_identity: Option<String>,
    pub connection_id: Option<String>,
    pub payload_kind: Option<String>,
}
