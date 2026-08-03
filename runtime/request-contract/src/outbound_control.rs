use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, AssemblyIdentity,
    DeploymentRevision,
};

use crate::{
    actor_invocation::ActorInvocationDeclarationOwner, actor_ref::ActorRef, response::ResponseError,
};

#[derive(Debug, Clone, PartialEq)]
pub enum OutboundControlMessage {
    ActorGetOrCreate {
        request: ActorGetOrCreateControlRequest,
        payload: Vec<u8>,
    },
    ActorReplace {
        request: ActorReplaceControlRequest,
        payload: Vec<u8>,
    },
    ActorFind {
        request: ActorFindControlRequest,
    },
    ActorRemove {
        request: ActorRemoveControlRequest,
    },
    TaskSubmit {
        request: TaskSubmitControlRequest,
        payload: Vec<u8>,
    },
    RequestCancel {
        request: RequestCancelControl,
    },
    ConnectionSend {
        request: ConnectionSendControl,
        payload: Vec<u8>,
    },
    ConnectionRequest {
        request: ConnectionRequestControl,
        payload: Vec<u8>,
    },
    ConnectionRequestCancel {
        request: ConnectionRequestCancelControl,
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
pub struct ActivationIdentityControl {
    pub assembly_identity: AssemblyIdentity,
    pub generation: u64,
    pub runtime_replica_id: String,
    pub deployment_revision: DeploymentRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorGetOrCreateControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityControl,
    pub actor_key: ActorKeyControlMetadata,
    pub actor_abi_identity: String,
    pub actor_implementation_identity: String,
    pub bootstrap_encoding_version: String,
    pub declaration_owner: ActorInvocationDeclarationOwner,
    pub deadline: Option<ActorControlDeadline>,
    pub test_case_capability: Option<String>,
    pub test_case_parent_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorReplaceControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityControl,
    pub actor_key: ActorKeyControlMetadata,
    pub actor_abi_identity: String,
    pub actor_implementation_identity: String,
    pub bootstrap_encoding_version: String,
    pub declaration_owner: ActorInvocationDeclarationOwner,
    pub deadline: Option<ActorControlDeadline>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorControlDeadline {
    pub timeout_ms: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorFindControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityControl,
    pub actor_key: ActorKeyControlMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRemoveControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityControl,
    pub actor_key: ActorKeyControlMetadata,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskSubmitControlRequest {
    pub rpc_id: String,
    pub runtime_id: String,
    pub target_kind: String,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    pub target: String,
    pub task_id: Option<String>,
    pub build_id: Option<String>,
    pub activation_identity: ActivationIdentityControl,
    pub caller_request_id: Option<String>,
    pub trace_id: Option<String>,
    pub caller_target: Option<String>,
    pub max_queue_wait_ms: Option<f64>,
    pub actor_method: Option<ActorMethodTaskTargetControl>,
}

/// Closed parent-kind namespace for the canonical task wire generation
/// (C-model-task §2). `callerKind` selects the unique parent resolver:
/// `request` -> FunctionTaskParentResolver, `actorInvocation` ->
/// ActorTaskParentResolver. The old shape (missing `callerKind`) is rejected
/// with no compatible reader; `H-task-parent-cut` is the production hard cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCallerKind {
    Request,
    ActorInvocation,
}

impl TaskCallerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::ActorInvocation => "actorInvocation",
        }
    }
}

/// Actor-method target facts for a `task.submit` whose targetKind is
/// `actorMethod`. The receiver travels as identity metadata (never inside the
/// recoverable args payload); the owner runtime routes by it and re-activates
/// the instance from the registry entry when it is not live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorMethodTaskTargetControl {
    pub actor_ref: ActorRef,
    pub declaration_owner: ActorInvocationDeclarationOwner,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub method_identity: ActorMethodIdentity,
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
pub struct RuntimeDeadlineControl {
    pub timeout_ms: u64,
    pub expires_at: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequestControl {
    pub request_id: String,
    pub service_id: String,
    pub websocket_entry_id: String,
    pub connection_id: String,
    pub method: String,
    pub deadline: Option<RuntimeDeadlineControl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRequestCancelControl {
    pub request_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutboundResponse {
    End { payload: Vec<u8> },
    Error(ResponseError),
}

impl OutboundResponse {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::End { .. } => "response.end",
            Self::Error(_) => "response.error",
        }
    }
}
