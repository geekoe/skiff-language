use std::fmt;

use crate::protocol::RUNTIME_FRAME_SCHEMA_VERSION;

/// Structured failure returned when an exact Actor lifecycle contract is
/// constructed or validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorLifecycleContractError {
    EmptyField {
        field: &'static str,
    },
    InvalidCanonicalToken {
        field: &'static str,
        value: String,
    },
    InvalidSha256Identity {
        field: &'static str,
        expected_prefix: &'static str,
        value: String,
    },
    InvalidPositiveSequence {
        field: &'static str,
        value: u64,
    },
    InvalidActorLogicalKey {
        message: String,
    },
    ActorDeploymentServiceMismatch {
        actor_service_id: String,
        deployment_service_id: String,
    },
    TargetRuntimeMismatch {
        target_runtime_id: String,
        owner_runtime_id: String,
    },
    AckRuntimeMismatch {
        runtime_id: String,
        owner_runtime_id: String,
    },
    DiscardAckRequestMismatch {
        request_id: String,
        ack_request_id: String,
    },
    DiscardAckFenceMismatch,
    DiscardAckDidNotConfirmAbsence,
    UnexpectedSchemaVersion {
        actual: String,
    },
    UnexpectedFrameType {
        expected: &'static str,
        actual: String,
    },
    InvalidActivationSnapshot {
        message: String,
    },
}

impl fmt::Display for ActorLifecycleContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { field } => write!(formatter, "{field} must be non-empty"),
            Self::InvalidCanonicalToken { field, value } => write!(
                formatter,
                "{field} value {value:?} must be a non-empty canonical token"
            ),
            Self::InvalidSha256Identity {
                field,
                expected_prefix,
                value,
            } => write!(
                formatter,
                "{field} value {value:?} must use {expected_prefix}:<64 lowercase hex>"
            ),
            Self::InvalidPositiveSequence { field, value } => write!(
                formatter,
                "{field} value {value} must be a positive JavaScript safe integer"
            ),
            Self::InvalidActorLogicalKey { message } => {
                write!(formatter, "invalid Actor logical key: {message}")
            }
            Self::ActorDeploymentServiceMismatch {
                actor_service_id,
                deployment_service_id,
            } => write!(
                formatter,
                "Actor service {actor_service_id:?} does not match deployment service {deployment_service_id:?}"
            ),
            Self::TargetRuntimeMismatch {
                target_runtime_id,
                owner_runtime_id,
            } => write!(
                formatter,
                "target runtime {target_runtime_id:?} does not match fence owner runtime {owner_runtime_id:?}"
            ),
            Self::AckRuntimeMismatch {
                runtime_id,
                owner_runtime_id,
            } => write!(
                formatter,
                "ACK runtime {runtime_id:?} does not match fence owner runtime {owner_runtime_id:?}"
            ),
            Self::DiscardAckRequestMismatch {
                request_id,
                ack_request_id,
            } => write!(
                formatter,
                "discard ACK request {ack_request_id:?} does not match request {request_id:?}"
            ),
            Self::DiscardAckFenceMismatch => {
                formatter.write_str("discard ACK does not echo the requested exact owner fence")
            }
            Self::DiscardAckDidNotConfirmAbsence => formatter.write_str(
                "discard ACK reported fenceMismatch and did not confirm owner absence",
            ),
            Self::UnexpectedSchemaVersion { actual } => write!(
                formatter,
                "schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}, got {actual:?}"
            ),
            Self::UnexpectedFrameType { expected, actual } => {
                write!(formatter, "frame type must be {expected}, got {actual:?}")
            }
            Self::InvalidActivationSnapshot { message } => {
                write!(formatter, "invalid durable Actor activation snapshot: {message}")
            }
        }
    }
}

impl std::error::Error for ActorLifecycleContractError {}
