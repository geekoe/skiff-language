use std::fmt;

use serde::{de, Deserializer};

pub const MAX_SAFE_ACTIVATION_GENERATION: u64 = 9_007_199_254_740_991;
pub const MAX_EXPECTED_ACTIVATION_GENERATION: u64 = MAX_SAFE_ACTIVATION_GENERATION - 1;
pub const RUNTIME_ASSEMBLY_IDENTITY_PREFIX: &str = "skiff-runtime-assembly-v3:sha256";

pub fn validate_activation_token(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 200
        || !value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
    {
        return Err(format!(
            "{label} must be an ASCII visible token between 1 and 200 bytes"
        ));
    }
    Ok(())
}

pub fn validate_activation_environment(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 200
        || value == "."
        || value == ".."
        || !value.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_'
            )
        })
    {
        return Err(
            "environment must be 1-200 ASCII letters, digits, dot, dash, or underscore and must not be . or .."
                .to_string(),
        );
    }
    Ok(())
}

pub fn validate_activation_generation(generation: u64, label: &str) -> Result<(), String> {
    if generation > MAX_SAFE_ACTIVATION_GENERATION {
        return Err(format!(
            "{label} must be between 0 and {MAX_SAFE_ACTIVATION_GENERATION}"
        ));
    }
    Ok(())
}

pub fn validate_expected_activation_generation(generation: u64, label: &str) -> Result<(), String> {
    if generation > MAX_EXPECTED_ACTIVATION_GENERATION {
        return Err(format!(
            "{label} must be between 0 and {MAX_EXPECTED_ACTIVATION_GENERATION}"
        ));
    }
    Ok(())
}

pub fn validate_transition_generations(
    expected_generation: u64,
    candidate_generation: u64,
) -> Result<(), String> {
    validate_expected_activation_generation(expected_generation, "expectedGeneration")?;
    validate_activation_generation(candidate_generation, "candidateGeneration")?;
    if expected_generation + 1 != candidate_generation {
        return Err("candidateGeneration must equal expectedGeneration + 1".to_string());
    }
    Ok(())
}

pub fn validate_runtime_assembly_identity(value: &str) -> Result<(), String> {
    runtime_assembly_identity_hash(value).map(|_| ())
}

pub fn runtime_assembly_identity_hash(value: &str) -> Result<&str, String> {
    let expected_prefix = format!("{RUNTIME_ASSEMBLY_IDENTITY_PREFIX}:");
    let Some(hash) = value.strip_prefix(&expected_prefix) else {
        return Err(invalid_runtime_assembly_identity_message());
    };
    if hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(hash)
    } else {
        Err(invalid_runtime_assembly_identity_message())
    }
}

pub fn deserialize_activation_generation<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(ActivationGenerationVisitor)
}

struct ActivationGenerationVisitor;

impl de::Visitor<'_> for ActivationGenerationVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical unsigned integer")
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(value)
    }
}

fn invalid_runtime_assembly_identity_message() -> String {
    format!("assemblyIdentity must use {RUNTIME_ASSEMBLY_IDENTITY_PREFIX}:<64 lowercase hex>")
}
