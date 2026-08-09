use serde::{de, Deserialize, Deserializer, Serialize};

use super::{
    error::ActorLifecycleContractError,
    identity::ExactActorOwnerFenceFrameMetadata,
    validation::{validate_frame_identity, validate_token},
};
use crate::{
    protocol::{decode_typed_binary_frame, encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    BinaryFrameError, TransportError,
};

pub const ACTOR_IDLE_DISCARD_REQUEST_FRAME_TYPE: &str = "actor.owner.idleDiscard.request";
pub const ACTOR_IDLE_DISCARD_ACK_FRAME_TYPE: &str = "actor.owner.idleDiscard.ack";

/// Router-to-Runtime request to stop admission, drain continuations, and
/// physically destroy the exact fenced Actor arena.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorIdleDiscardRequestFrameHeader {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    request_id: String,
    target_runtime_id: String,
    fence: ExactActorOwnerFenceFrameMetadata,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActorIdleDiscardRequestFrameHeader {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    request_id: String,
    target_runtime_id: String,
    fence: ExactActorOwnerFenceFrameMetadata,
}

impl ActorIdleDiscardRequestFrameHeader {
    pub fn new(
        request_id: impl Into<String>,
        target_runtime_id: impl Into<String>,
        fence: ExactActorOwnerFenceFrameMetadata,
    ) -> Result<Self, ActorLifecycleContractError> {
        let header = Self {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: ACTOR_IDLE_DISCARD_REQUEST_FRAME_TYPE.to_string(),
            request_id: request_id.into(),
            target_runtime_id: target_runtime_id.into(),
            fence,
        };
        header.validate()?;
        Ok(header)
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn target_runtime_id(&self) -> &str {
        &self.target_runtime_id
    }

    pub fn fence(&self) -> &ExactActorOwnerFenceFrameMetadata {
        &self.fence
    }

    /// Validate exact request/ACK correlation before an owner may be cleared.
    pub fn validate_ack(
        &self,
        ack: &ActorIdleDiscardAckFrameHeader,
    ) -> Result<ActorIdleDiscardAckOutcome, ActorLifecycleContractError> {
        self.validate()?;
        ack.validate()?;
        if self.request_id != ack.request_id {
            return Err(ActorLifecycleContractError::DiscardAckRequestMismatch {
                request_id: self.request_id.clone(),
                ack_request_id: ack.request_id.clone(),
            });
        }
        if self.fence != ack.fence {
            return Err(ActorLifecycleContractError::DiscardAckFenceMismatch);
        }
        Ok(ack.outcome)
    }

    pub fn validate(&self) -> Result<(), ActorLifecycleContractError> {
        validate_frame_identity(
            &self.schema_version,
            &self.envelope_type,
            ACTOR_IDLE_DISCARD_REQUEST_FRAME_TYPE,
        )?;
        validate_token(&self.request_id, "requestId")?;
        validate_token(&self.target_runtime_id, "targetRuntimeId")?;
        self.fence.validate()?;
        if self.target_runtime_id != self.fence.owner_runtime_id() {
            return Err(ActorLifecycleContractError::TargetRuntimeMismatch {
                target_runtime_id: self.target_runtime_id.clone(),
                owner_runtime_id: self.fence.owner_runtime_id().to_string(),
            });
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ActorIdleDiscardRequestFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawActorIdleDiscardRequestFrameHeader::deserialize(deserializer)?;
        let header = Self {
            schema_version: raw.schema_version,
            envelope_type: raw.envelope_type,
            request_id: raw.request_id,
            target_runtime_id: raw.target_runtime_id,
            fence: raw.fence,
        };
        header.validate().map_err(de::Error::custom)?;
        Ok(header)
    }
}

/// Runtime disposition for an idle-discard request. Only `Discarded` and
/// `AlreadyAbsent` prove that the exact fenced arena cannot admit or resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorIdleDiscardAckOutcome {
    Discarded,
    AlreadyAbsent,
    FenceMismatch,
}

impl ActorIdleDiscardAckOutcome {
    pub const fn confirms_absence(self) -> bool {
        matches!(self, Self::Discarded | Self::AlreadyAbsent)
    }
}

/// Runtime-to-Router acknowledgement. The exact fence is echoed so a request
/// id alone can never clear a newer owner or arena epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorIdleDiscardAckFrameHeader {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    request_id: String,
    runtime_id: String,
    fence: ExactActorOwnerFenceFrameMetadata,
    outcome: ActorIdleDiscardAckOutcome,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActorIdleDiscardAckFrameHeader {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    request_id: String,
    runtime_id: String,
    fence: ExactActorOwnerFenceFrameMetadata,
    outcome: ActorIdleDiscardAckOutcome,
}

impl ActorIdleDiscardAckFrameHeader {
    pub fn new(
        request_id: impl Into<String>,
        runtime_id: impl Into<String>,
        fence: ExactActorOwnerFenceFrameMetadata,
        outcome: ActorIdleDiscardAckOutcome,
    ) -> Result<Self, ActorLifecycleContractError> {
        let header = Self {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: ACTOR_IDLE_DISCARD_ACK_FRAME_TYPE.to_string(),
            request_id: request_id.into(),
            runtime_id: runtime_id.into(),
            fence,
            outcome,
        };
        header.validate()?;
        Ok(header)
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub fn fence(&self) -> &ExactActorOwnerFenceFrameMetadata {
        &self.fence
    }

    pub const fn outcome(&self) -> ActorIdleDiscardAckOutcome {
        self.outcome
    }

    pub fn validate(&self) -> Result<(), ActorLifecycleContractError> {
        validate_frame_identity(
            &self.schema_version,
            &self.envelope_type,
            ACTOR_IDLE_DISCARD_ACK_FRAME_TYPE,
        )?;
        validate_token(&self.request_id, "requestId")?;
        validate_token(&self.runtime_id, "runtimeId")?;
        self.fence.validate()?;
        if self.runtime_id != self.fence.owner_runtime_id() {
            return Err(ActorLifecycleContractError::AckRuntimeMismatch {
                runtime_id: self.runtime_id.clone(),
                owner_runtime_id: self.fence.owner_runtime_id().to_string(),
            });
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ActorIdleDiscardAckFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawActorIdleDiscardAckFrameHeader::deserialize(deserializer)?;
        let header = Self {
            schema_version: raw.schema_version,
            envelope_type: raw.envelope_type,
            request_id: raw.request_id,
            runtime_id: raw.runtime_id,
            fence: raw.fence,
            outcome: raw.outcome,
        };
        header.validate().map_err(de::Error::custom)?;
        Ok(header)
    }
}

pub fn encode_actor_idle_discard_request_frame(
    header: &ActorIdleDiscardRequestFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    header.validate().map_err(contract_transport_error)?;
    encode_binary_frame(header, &[])
}

pub fn decode_actor_idle_discard_request_frame(
    bytes: &[u8],
) -> Result<ActorIdleDiscardRequestFrameHeader, BinaryFrameError> {
    let (header, payload) = decode_typed_binary_frame(bytes)?;
    reject_payload(&payload, ACTOR_IDLE_DISCARD_REQUEST_FRAME_TYPE)?;
    Ok(header)
}

pub fn encode_actor_idle_discard_ack_frame(
    header: &ActorIdleDiscardAckFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    header.validate().map_err(contract_transport_error)?;
    encode_binary_frame(header, &[])
}

pub fn decode_actor_idle_discard_ack_frame(
    bytes: &[u8],
) -> Result<ActorIdleDiscardAckFrameHeader, BinaryFrameError> {
    let (header, payload) = decode_typed_binary_frame(bytes)?;
    reject_payload(&payload, ACTOR_IDLE_DISCARD_ACK_FRAME_TYPE)?;
    Ok(header)
}

fn reject_payload(payload: &[u8], frame_type: &str) -> Result<(), BinaryFrameError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(TransportError::decode(format!(
            "{frame_type} payload must be empty"
        )))
    }
}

fn contract_transport_error(error: ActorLifecycleContractError) -> BinaryFrameError {
    TransportError::decode(format!("invalid Actor lifecycle contract: {error}"))
}
