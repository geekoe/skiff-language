use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_model::{
    validate_activation_generation, validate_activation_token, validate_runtime_assembly_identity,
};

use crate::actor_method::{ActorDeclarationOwnerFrameHeader, ActorMethodDeadlineFrameHeader};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorKeyFrameMetadata {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRefFrameMetadata {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationIdentityFrameMetadata {
    pub assembly_identity: String,
    pub generation: u64,
    pub runtime_replica_id: String,
    pub deployment_revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActivationIdentityFrameMetadata {
    assembly_identity: String,
    generation: u64,
    runtime_replica_id: String,
    deployment_revision: String,
}

impl<'de> Deserialize<'de> for ActivationIdentityFrameMetadata {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawActivationIdentityFrameMetadata::deserialize(deserializer)?;
        validate_runtime_assembly_identity(&raw.assembly_identity).map_err(de::Error::custom)?;
        validate_activation_generation(raw.generation, "generation").map_err(de::Error::custom)?;
        validate_activation_token(&raw.runtime_replica_id, "runtimeReplicaId")
            .map_err(de::Error::custom)?;
        validate_activation_token(&raw.deployment_revision, "deploymentRevision")
            .map_err(de::Error::custom)?;
        Ok(Self {
            assembly_identity: raw.assembly_identity,
            generation: raw.generation,
            runtime_replica_id: raw.runtime_replica_id,
            deployment_revision: raw.deployment_revision,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorGetOrCreateRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityFrameMetadata,
    pub actor_key: ActorKeyFrameMetadata,
    pub actor_abi_identity: String,
    pub actor_implementation_identity: String,
    pub bootstrap_encoding_version: String,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<ActorMethodDeadlineFrameHeader>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_case_capability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_case_parent_request_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActorGetOrCreateRequestFrameHeader {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    rpc_id: String,
    runtime_id: String,
    activation_identity: ActivationIdentityFrameMetadata,
    actor_key: ActorKeyFrameMetadata,
    actor_abi_identity: String,
    actor_implementation_identity: String,
    bootstrap_encoding_version: String,
    declaration_owner: ActorDeclarationOwnerFrameHeader,
    #[serde(default)]
    deadline: Option<ActorMethodDeadlineFrameHeader>,
    #[serde(default)]
    test_case_capability: Option<String>,
    #[serde(default)]
    test_case_parent_request_id: Option<String>,
}

impl<'de> Deserialize<'de> for ActorGetOrCreateRequestFrameHeader {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawActorGetOrCreateRequestFrameHeader::deserialize(deserializer)?;
        validate_test_case_authority(
            raw.test_case_capability.as_deref(),
            raw.test_case_parent_request_id.as_deref(),
        )
        .map_err(de::Error::custom)?;
        Ok(Self {
            schema_version: raw.schema_version,
            envelope_type: raw.envelope_type,
            rpc_id: raw.rpc_id,
            runtime_id: raw.runtime_id,
            activation_identity: raw.activation_identity,
            actor_key: raw.actor_key,
            actor_abi_identity: raw.actor_abi_identity,
            actor_implementation_identity: raw.actor_implementation_identity,
            bootstrap_encoding_version: raw.bootstrap_encoding_version,
            declaration_owner: raw.declaration_owner,
            deadline: raw.deadline,
            test_case_capability: raw.test_case_capability,
            test_case_parent_request_id: raw.test_case_parent_request_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorGetOrCreateResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub actor_ref: ActorRefFrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorReplaceRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityFrameMetadata,
    pub actor_key: ActorKeyFrameMetadata,
    pub actor_abi_identity: String,
    pub actor_implementation_identity: String,
    pub bootstrap_encoding_version: String,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<ActorMethodDeadlineFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorReplaceResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub actor_ref: ActorRefFrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorFindRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityFrameMetadata,
    pub actor_key: ActorKeyFrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorFindResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_ref: Option<ActorRefFrameMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRemoveRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub activation_identity: ActivationIdentityFrameMetadata,
    pub actor_key: ActorKeyFrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRemoveResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub removed: bool,
}

pub(crate) fn validate_test_case_authority(
    capability: Option<&str>,
    parent_request_id: Option<&str>,
) -> std::result::Result<(), String> {
    if capability.is_some() != parent_request_id.is_some() {
        return Err(
            "testCaseCapability and testCaseParentRequestId must be present together".to_string(),
        );
    }
    if let Some(capability) = capability {
        validate_canonical_token(capability, "testCaseCapability")?;
    }
    if let Some(parent_request_id) = parent_request_id {
        validate_canonical_token(parent_request_id, "testCaseParentRequestId")?;
    }
    Ok(())
}

fn validate_canonical_token(value: &str, label: &str) -> std::result::Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(format!("{label} must be a non-empty canonical token"));
    }
    Ok(())
}
