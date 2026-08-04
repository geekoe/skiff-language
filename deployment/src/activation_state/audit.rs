//! Durable activation audit event (frozen C-router-activation-state §6 shape).
//!
//! The event carries no Mongo URL, secret, or business payload. `event_id` is
//! a deterministic digest of the dedup key
//! `(profile, activation_id, operation, expected_generation,
//! candidate_generation)` so retried mutations never append a duplicate event
//! and the Mongo adapter can use `_id: event_id` as an idempotency anchor.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const EVENT_ID_DOMAIN: &[u8] = b"skiff-router-activation-audit-event-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivationAuditOperation {
    Prepare,
    Commit,
    Abort,
}

impl ActivationAuditOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Commit => "commit",
            Self::Abort => "abort",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationAuditOutcome {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "cas_mismatch")]
    CasMismatch,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "error")]
    Error,
}

impl ActivationAuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::CasMismatch => "cas_mismatch",
            Self::Invalid => "invalid",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationAuditEvent {
    pub event_id: String,
    pub profile: String,
    pub activation_id: String,
    pub operation: ActivationAuditOperation,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub outcome: ActivationAuditOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_replica_ids: Option<Vec<String>>,
    /// Unix epoch milliseconds (UTC).
    pub timestamp: i64,
}

impl ActivationAuditEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        profile: impl Into<String>,
        activation_id: impl Into<String>,
        operation: ActivationAuditOperation,
        expected_generation: u64,
        candidate_generation: u64,
        outcome: ActivationAuditOutcome,
        participant_replica_ids: Option<Vec<String>>,
        timestamp_millis: i64,
    ) -> Self {
        let profile = profile.into();
        let activation_id = activation_id.into();
        let event_id = activation_audit_event_id(
            &profile,
            &activation_id,
            operation,
            expected_generation,
            candidate_generation,
        );
        Self {
            event_id,
            profile,
            activation_id,
            operation,
            expected_generation,
            candidate_generation,
            outcome,
            participant_replica_ids,
            timestamp: timestamp_millis,
        }
    }
}

pub fn activation_audit_event_id(
    profile: &str,
    activation_id: &str,
    operation: ActivationAuditOperation,
    expected_generation: u64,
    candidate_generation: u64,
) -> String {
    let mut hasher = Sha256::new();
    for part in [
        EVENT_ID_DOMAIN,
        profile.as_bytes(),
        activation_id.as_bytes(),
        operation.as_str().as_bytes(),
        expected_generation.to_string().as_bytes(),
        candidate_generation.to_string().as_bytes(),
    ] {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    format!(
        "skiff-router-activation-audit-event-v1:{}",
        hex::encode(hasher.finalize())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_id_is_stable_and_tuple_sensitive() {
        let first = activation_audit_event_id(
            "test",
            "activation-a",
            ActivationAuditOperation::Prepare,
            7,
            8,
        );
        let replay = activation_audit_event_id(
            "test",
            "activation-a",
            ActivationAuditOperation::Prepare,
            7,
            8,
        );
        let other_profile = activation_audit_event_id(
            "prod",
            "activation-a",
            ActivationAuditOperation::Prepare,
            7,
            8,
        );
        let other_generation = activation_audit_event_id(
            "test",
            "activation-a",
            ActivationAuditOperation::Prepare,
            8,
            9,
        );
        let other_operation = activation_audit_event_id(
            "test",
            "activation-a",
            ActivationAuditOperation::Abort,
            7,
            8,
        );
        assert_eq!(first, replay);
        assert_ne!(first, other_profile);
        assert_ne!(first, other_generation);
        assert_ne!(first, other_operation);
        assert!(first.starts_with("skiff-router-activation-audit-event-v1:"));
    }

    #[test]
    fn event_round_trips_through_serde_with_exact_field_names() {
        let event = ActivationAuditEvent::new(
            "test",
            "activation-a",
            ActivationAuditOperation::Prepare,
            7,
            8,
            ActivationAuditOutcome::Ok,
            Some(vec!["runtime-a".to_string(), "runtime-b".to_string()]),
            1_752_531_600_000,
        );
        let value = serde_json::to_value(&event).expect("serialize audit event");
        assert_eq!(value["operation"], "prepare");
        assert_eq!(value["outcome"], "ok");
        assert_eq!(value["expectedGeneration"], 7);
        assert_eq!(value["candidateGeneration"], 8);
        assert_eq!(
            value["participantReplicaIds"],
            serde_json::json!(["runtime-a", "runtime-b"])
        );
        let decoded: ActivationAuditEvent =
            serde_json::from_value(value).expect("strict decode audit event");
        assert_eq!(decoded, event);
    }

    #[test]
    fn outcome_enum_uses_frozen_serialized_names() {
        assert_eq!(ActivationAuditOutcome::Ok.as_str(), "ok");
        assert_eq!(ActivationAuditOutcome::CasMismatch.as_str(), "cas_mismatch");
        assert_eq!(ActivationAuditOutcome::Invalid.as_str(), "invalid");
        assert_eq!(ActivationAuditOutcome::Error.as_str(), "error");
    }
}
