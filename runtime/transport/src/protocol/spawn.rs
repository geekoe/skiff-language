use serde::{Deserialize, Serialize};

use crate::{
    actor_method::{ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader},
    protocol::{actor::ActivationIdentityFrameMetadata, request::RuntimeErrorFramePayload},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnSubmitRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub target_kind: String,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    pub activation_identity: ActivationIdentityFrameMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queue_wait_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_method: Option<SpawnActorMethodTargetFrameMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnActorMethodTargetFrameMetadata {
    pub actor_ref: ActorLogicalRefFrameHeader,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    pub actor_abi_identity: skiff_artifact_model::ActorAbiIdentity,
    pub actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity,
    pub method_identity: skiff_artifact_model::ActorMethodIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnSubmitResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub spawn_id: String,
    pub request_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorSpawnRuntimeErrorFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub error: RuntimeErrorFramePayload,
}
