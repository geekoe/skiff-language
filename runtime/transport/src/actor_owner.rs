use base64::Engine as _;
use serde::{Deserialize, Serialize};
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};

use crate::{
    actor_method::{
        ActorDeclarationOwnerFrameHeader, ActorMethodDeadlineFrameHeader,
        ActorMethodInvokeFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
    },
    protocol::{decode_binary_frame, encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    BinaryFrameError, TransportError,
};

pub const ACTOR_OWNER_INVOKE_FRAME_TYPE: &str = "actor.owner.invoke";
pub const ACTOR_OWNER_CONTROL_FRAME_TYPE: &str = "actor.owner.control";
pub const ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE: &str = "actor.owner.control.ack";
pub const ACTOR_OWNER_FAILURE_FRAME_TYPE: &str = "actor.owner.failure";
pub const ACTOR_BOOTSTRAP_ENCODING_V1: &str = "skiff-canonical-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerFenceFrameHeader {
    pub owner_runtime_id: String,
    pub owner_lease_id: String,
    pub epoch: u64,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorActivationBootstrapFrameHeader {
    pub encoding_version: String,
    pub payload_base64: String,
}

impl ActorActivationBootstrapFrameHeader {
    pub fn decode_payload(&self) -> Result<Vec<u8>, BinaryFrameError> {
        if self.encoding_version != ACTOR_BOOTSTRAP_ENCODING_V1 {
            return Err(TransportError::decode(
                "actor activation bootstrap encoding is unsupported",
            ));
        }
        let payload = base64::engine::general_purpose::STANDARD
            .decode(&self.payload_base64)
            .map_err(|_| TransportError::decode("actor activation bootstrap is not base64"))?;
        if base64::engine::general_purpose::STANDARD.encode(&payload) != self.payload_base64 {
            return Err(TransportError::decode(
                "actor activation bootstrap is not canonical base64",
            ));
        }
        Ok(payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerInvokeFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub target_runtime_id: String,
    pub owner_fence: ActorOwnerFenceFrameHeader,
    pub invoke: ActorMethodInvokeFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_bootstrap: Option<ActorActivationBootstrapFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerLogicalKeyFrameHeader {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerControlFenceFrameHeader {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: String,
    pub epoch: u64,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    pub owner_lease_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eviction_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerActivationTransitionFrameHeader {
    pub old_epoch: u64,
    pub new_epoch: u64,
    pub actor_abi_identity: ActorAbiIdentity,
    pub target_implementation_identity: ActorImplementationIdentity,
    pub bootstrap_encoding_version: String,
    pub bootstrap_payload_base64: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorOwnerControlOperation {
    MarkUpgrading,
    Discard,
    Activate,
    ActivateInitial,
    IdleEvict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerControlFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub target_runtime_id: String,
    pub request_id: String,
    pub operation: ActorOwnerControlOperation,
    pub fence: ActorOwnerControlFenceFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<ActorOwnerActivationTransitionFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<ActorActivationBootstrapFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<ActorMethodDeadlineFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerControlAckFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
    pub request_id: String,
    pub operation: ActorOwnerControlOperation,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ActorOwnerFailureReasonFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerFailureReasonFrameHeader {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorOwnerFailureFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub invocation_id: String,
    pub owner_runtime_id: String,
    pub owner_lease_id: String,
    pub epoch: u64,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub reason: ActorOwnerFailureReasonFrameHeader,
}

pub fn encode_actor_owner_invoke_frame(
    header: &ActorOwnerInvokeFrameHeader,
    arguments_payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_invoke(header)?;
    encode_binary_frame(header, arguments_payload)
}

pub fn decode_actor_owner_invoke_frame(
    bytes: &[u8],
) -> Result<(ActorOwnerInvokeFrameHeader, Vec<u8>), BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    let header: ActorOwnerInvokeFrameHeader = serde_json::from_value(frame.header)
        .map_err(|error| TransportError::decode(format!("invalid actor.owner.invoke: {error}")))?;
    validate_invoke(&header)?;
    Ok((header, frame.payload_bytes))
}

pub fn encode_actor_owner_control_frame(
    header: &ActorOwnerControlFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_control(header)?;
    encode_binary_frame(header, &[])
}

pub fn decode_actor_owner_control_frame(
    bytes: &[u8],
) -> Result<ActorOwnerControlFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    if !frame.payload_bytes.is_empty() {
        return Err(TransportError::decode(
            "actor.owner.control payload must be empty",
        ));
    }
    let header: ActorOwnerControlFrameHeader = serde_json::from_value(frame.header)
        .map_err(|error| TransportError::decode(format!("invalid actor.owner.control: {error}")))?;
    validate_control(&header)?;
    Ok(header)
}

pub fn encode_actor_owner_control_ack_frame(
    header: &ActorOwnerControlAckFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE,
    )?;
    validate_token(&header.runtime_id, "runtimeId")?;
    validate_token(&header.request_id, "requestId")?;
    if header.accepted && header.reason.is_some() {
        return Err(TransportError::decode(
            "actor.owner.control.ack must not carry a failure reason when accepted",
        ));
    }
    if let Some(reason) = &header.reason {
        validate_token(&reason.code, "reason.code")?;
        if reason.message.is_empty() || reason.message.len() > 4096 {
            return Err(TransportError::decode(
                "reason.message must contain 1..4096 bytes",
            ));
        }
    }
    encode_binary_frame(header, &[])
}

pub fn encode_actor_owner_failure_frame(
    header: &ActorOwnerFailureFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        ACTOR_OWNER_FAILURE_FRAME_TYPE,
    )?;
    validate_token(&header.invocation_id, "invocationId")?;
    validate_token(&header.owner_runtime_id, "ownerRuntimeId")?;
    validate_token(&header.owner_lease_id, "ownerLeaseId")?;
    if header.epoch == 0 {
        return Err(TransportError::decode("epoch must be positive"));
    }
    validate_identity(
        header.actor_implementation_identity.as_str(),
        "skiff-actor-implementation-v1:sha256",
        "actorImplementationIdentity",
    )?;
    validate_token(&header.reason.code, "reason.code")?;
    if header.reason.message.is_empty() || header.reason.message.len() > 4096 {
        return Err(TransportError::decode(
            "reason.message must contain 1..4096 bytes",
        ));
    }
    encode_binary_frame(header, &[])
}

pub fn decode_actor_owner_failure_frame(
    bytes: &[u8],
) -> Result<ActorOwnerFailureFrameHeader, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    if !frame.payload_bytes.is_empty() {
        return Err(TransportError::decode(
            "actor.owner.failure payload must be empty",
        ));
    }
    let header: ActorOwnerFailureFrameHeader = serde_json::from_value(frame.header)
        .map_err(|error| TransportError::decode(format!("invalid actor.owner.failure: {error}")))?;
    encode_actor_owner_failure_frame(&header)?;
    Ok(header)
}

fn validate_invoke(header: &ActorOwnerInvokeFrameHeader) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        ACTOR_OWNER_INVOKE_FRAME_TYPE,
    )?;
    validate_token(&header.target_runtime_id, "targetRuntimeId")?;
    validate_token(
        &header.owner_fence.owner_runtime_id,
        "ownerFence.ownerRuntimeId",
    )?;
    validate_token(
        &header.owner_fence.owner_lease_id,
        "ownerFence.ownerLeaseId",
    )?;
    if header.owner_fence.epoch == 0 {
        return Err(TransportError::decode("ownerFence.epoch must be positive"));
    }
    if header.target_runtime_id != header.owner_fence.owner_runtime_id {
        return Err(TransportError::decode(
            "targetRuntimeId must equal ownerFence.ownerRuntimeId",
        ));
    }
    if header.invoke.actor_ref.epoch != header.owner_fence.epoch
        || header.invoke.actor_abi_identity != header.owner_fence.actor_abi_identity
        || header.invoke.actor_implementation_identity
            != header.owner_fence.actor_implementation_identity
        || header.invoke.declaration_owner != header.owner_fence.declaration_owner
    {
        return Err(TransportError::decode(
            "actor.owner.invoke does not match its admitted owner fence",
        ));
    }
    if let Some(bootstrap) = &header.activation_bootstrap {
        bootstrap.decode_payload()?;
    }
    validate_owner(&header.owner_fence.declaration_owner)
}

fn validate_control(header: &ActorOwnerControlFrameHeader) -> Result<(), BinaryFrameError> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        ACTOR_OWNER_CONTROL_FRAME_TYPE,
    )?;
    validate_token(&header.target_runtime_id, "targetRuntimeId")?;
    validate_token(&header.request_id, "requestId")?;
    validate_token(&header.fence.owner_lease_id, "fence.ownerLeaseId")?;
    validate_logical_key(&ActorOwnerLogicalKeyFrameHeader {
        service_id: header.fence.service_id.clone(),
        actor_type_identity: header.fence.actor_type_identity.clone(),
        actor_id_type_identity: header.fence.actor_id_type_identity.clone(),
        actor_id_encoding_version: header.fence.actor_id_encoding_version.clone(),
        canonical_actor_id_key_bytes_base64: header
            .fence
            .canonical_actor_id_key_bytes_base64
            .clone(),
        actor_id_hash: header.fence.actor_id_hash.clone(),
    })?;
    validate_owner(&header.fence.declaration_owner)?;
    validate_identity(
        header.fence.actor_abi_identity.as_str(),
        "skiff-actor-abi-v1:sha256",
        "fence.actorAbiIdentity",
    )?;
    validate_identity(
        header.fence.actor_implementation_identity.as_str(),
        "skiff-actor-implementation-v1:sha256",
        "fence.actorImplementationIdentity",
    )?;
    if header.fence.epoch == 0 {
        return Err(TransportError::decode("fence.epoch must be positive"));
    }
    match header.operation {
        ActorOwnerControlOperation::Activate => {
            let transition = header
                .transition
                .as_ref()
                .ok_or_else(|| TransportError::decode("activate control requires transition"))?;
            if header.fence.eviction_request_id.is_some()
                || header.bootstrap.is_some()
                || header.deadline.is_some()
                || transition.new_epoch != header.fence.epoch
                || transition.old_epoch >= transition.new_epoch
                || transition.actor_abi_identity != header.fence.actor_abi_identity
                || transition.target_implementation_identity
                    != header.fence.actor_implementation_identity
            {
                return Err(TransportError::decode(
                    "activate control transition does not match fence",
                ));
            }
            ActorActivationBootstrapFrameHeader {
                encoding_version: transition.bootstrap_encoding_version.clone(),
                payload_base64: transition.bootstrap_payload_base64.clone(),
            }
            .decode_payload()?;
        }
        ActorOwnerControlOperation::IdleEvict => {
            if header.transition.is_some() {
                return Err(TransportError::decode(
                    "idleEvict control must not contain transition",
                ));
            }
            if header.bootstrap.is_some() || header.deadline.is_some() {
                return Err(TransportError::decode(
                    "idleEvict control must not contain bootstrap or deadline",
                ));
            }
            validate_token(
                header.fence.eviction_request_id.as_deref().ok_or_else(|| {
                    TransportError::decode("idleEvict control requires evictionRequestId")
                })?,
                "fence.evictionRequestId",
            )?;
        }
        ActorOwnerControlOperation::ActivateInitial => {
            if header.transition.is_some() || header.fence.eviction_request_id.is_some() {
                return Err(TransportError::decode(
                    "activateInitial control must not contain transition or evictionRequestId",
                ));
            }
            let bootstrap = header.bootstrap.as_ref().ok_or_else(|| {
                TransportError::decode("activateInitial control requires bootstrap")
            })?;
            bootstrap.decode_payload()?;
            let deadline = header.deadline.as_ref().ok_or_else(|| {
                TransportError::decode("activateInitial control requires deadline")
            })?;
            validate_activation_deadline(deadline)?;
        }
        ActorOwnerControlOperation::MarkUpgrading | ActorOwnerControlOperation::Discard => {
            if header.transition.is_some()
                || header.fence.eviction_request_id.is_some()
                || header.bootstrap.is_some()
                || header.deadline.is_some()
            {
                return Err(TransportError::decode(
                    "control operation contains unsupported optional fields",
                ));
            }
        }
    }
    Ok(())
}

fn validate_common(
    schema_version: &str,
    envelope_type: &str,
    expected_type: &str,
) -> Result<(), BinaryFrameError> {
    if schema_version != RUNTIME_FRAME_SCHEMA_VERSION || envelope_type != expected_type {
        return Err(TransportError::decode(format!(
            "invalid {expected_type} schemaVersion or type"
        )));
    }
    Ok(())
}

fn validate_logical_key(key: &ActorOwnerLogicalKeyFrameHeader) -> Result<(), BinaryFrameError> {
    for (label, value) in [
        ("actorRef.serviceId", key.service_id.as_str()),
        (
            "actorRef.actorTypeIdentity",
            key.actor_type_identity.as_str(),
        ),
        (
            "actorRef.actorIdTypeIdentity",
            key.actor_id_type_identity.as_str(),
        ),
        (
            "actorRef.actorIdEncodingVersion",
            key.actor_id_encoding_version.as_str(),
        ),
        ("actorRef.actorIdHash", key.actor_id_hash.as_str()),
    ] {
        if value.is_empty() {
            return Err(TransportError::decode(format!("{label} must be non-empty")));
        }
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&key.canonical_actor_id_key_bytes_base64)
        .map_err(|_| TransportError::decode("actorRef key bytes are not base64"))?;
    if base64::engine::general_purpose::STANDARD.encode(bytes)
        != key.canonical_actor_id_key_bytes_base64
    {
        return Err(TransportError::decode(
            "actorRef key bytes are not canonical base64",
        ));
    }
    Ok(())
}

fn validate_token(value: &str, label: &str) -> Result<(), BinaryFrameError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(TransportError::decode(format!(
            "{label} must be a canonical token"
        )));
    }
    Ok(())
}

fn validate_identity(value: &str, prefix: &str, label: &str) -> Result<(), BinaryFrameError> {
    let Some(digest) = value.strip_prefix(&format!("{prefix}:")) else {
        return Err(TransportError::decode(format!(
            "{label} has the wrong prefix"
        )));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(TransportError::decode(format!(
            "{label} must contain a lowercase sha256 digest"
        )));
    }
    Ok(())
}

fn validate_activation_deadline(
    deadline: &ActorMethodDeadlineFrameHeader,
) -> Result<(), BinaryFrameError> {
    if deadline.timeout_ms == 0 {
        return Err(TransportError::decode(
            "activation deadline timeoutMs must be positive",
        ));
    }
    if deadline.expires_at.is_empty() {
        return Err(TransportError::decode(
            "activation deadline expiresAt must be non-empty",
        ));
    }
    Ok(())
}

fn validate_owner(owner: &ActorDeclarationOwnerFrameHeader) -> Result<(), BinaryFrameError> {
    validate_token(&owner.actor_symbol, "declarationOwner.actorSymbol")?;
    match &owner.unit {
        ActorOwnerUnitFrameHeader::Service => {}
        ActorOwnerUnitFrameHeader::Package(_) => {}
    }
    match &owner.file {
        ActorOwnerFileFrameHeader::LoadedFileIndex(_) => {}
        ActorOwnerFileFrameHeader::FileIrIdentity(value) => {
            validate_token(value, "declarationOwner.file.value")?
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
