use super::error::ActorLifecycleContractError;
use crate::protocol::RUNTIME_FRAME_SCHEMA_VERSION;

pub(super) const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub(super) const SHA256_PREFIX: &str = "sha256";

pub(super) fn validate_non_empty(
    value: &str,
    field: &'static str,
) -> Result<(), ActorLifecycleContractError> {
    if value.trim().is_empty() {
        Err(ActorLifecycleContractError::EmptyField { field })
    } else {
        Ok(())
    }
}

pub(super) fn validate_token(
    value: &str,
    field: &'static str,
) -> Result<(), ActorLifecycleContractError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        Err(ActorLifecycleContractError::InvalidCanonicalToken {
            field,
            value: value.to_string(),
        })
    } else {
        Ok(())
    }
}

pub(super) fn validate_sha256_identity(
    value: &str,
    expected_prefix: &'static str,
    field: &'static str,
) -> Result<(), ActorLifecycleContractError> {
    let valid = value
        .strip_prefix(expected_prefix)
        .and_then(|suffix| suffix.strip_prefix(':'))
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        });
    if valid {
        Ok(())
    } else {
        Err(ActorLifecycleContractError::InvalidSha256Identity {
            field,
            expected_prefix,
            value: value.to_string(),
        })
    }
}

pub(super) fn validate_positive_sequence(
    value: u64,
    field: &'static str,
) -> Result<(), ActorLifecycleContractError> {
    if value == 0 || value > JAVASCRIPT_MAX_SAFE_INTEGER {
        Err(ActorLifecycleContractError::InvalidPositiveSequence { field, value })
    } else {
        Ok(())
    }
}

pub(super) fn validate_frame_identity(
    schema_version: &str,
    envelope_type: &str,
    expected_type: &'static str,
) -> Result<(), ActorLifecycleContractError> {
    if schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(ActorLifecycleContractError::UnexpectedSchemaVersion {
            actual: schema_version.to_string(),
        });
    }
    if envelope_type != expected_type {
        return Err(ActorLifecycleContractError::UnexpectedFrameType {
            expected: expected_type,
            actual: envelope_type.to_string(),
        });
    }
    Ok(())
}
