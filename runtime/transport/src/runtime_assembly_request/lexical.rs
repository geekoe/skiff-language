use serde::{de, Deserialize, Deserializer};
use skiff_artifact_model::{
    deserialize_activation_generation, validate_activation_generation,
    validate_runtime_assembly_identity, AssemblyIdentity, ContractOperationId,
};

use crate::protocol::RUNTIME_FRAME_SCHEMA_VERSION;

const CONTRACT_OPERATION_ID_PREFIX: &str = "skiff-contract-operation-v1:sha256:";

pub(super) fn deserialize_runtime_frame_schema_version<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(
        deserializer,
        RUNTIME_FRAME_SCHEMA_VERSION,
        "request.start schemaVersion",
    )
}

pub(super) fn deserialize_request_start_type<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "request.start", "binary frame type")
}

pub(super) fn deserialize_dispatch_mode<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value != "unary" && value != "serverStream" {
        return Err(de::Error::custom(
            "request.start mode must be unary or serverStream",
        ));
    }
    Ok(value)
}

pub(super) fn deserialize_gateway_caller_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "gateway", "runtimeAssembly caller.kind")
}

pub(super) fn deserialize_runtime_assembly_routing_kind<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(
        deserializer,
        "runtimeAssembly",
        "request.start routing.kind",
    )
}

pub(super) fn deserialize_assembly_identity<'de, D>(
    deserializer: D,
) -> Result<AssemblyIdentity, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_runtime_assembly_identity(&value).map_err(de::Error::custom)?;
    Ok(AssemblyIdentity::new(value))
}

pub(super) fn deserialize_safe_activation_generation<'de, D>(
    deserializer: D,
) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = deserialize_activation_generation(deserializer)?;
    validate_activation_generation(value, "routing.assemblyGeneration")
        .map_err(de::Error::custom)?;
    Ok(value)
}

pub(super) fn deserialize_contract_operation_id<'de, D>(
    deserializer: D,
) -> Result<ContractOperationId, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let valid = value
        .strip_prefix(CONTRACT_OPERATION_ID_PREFIX)
        .is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        });
    if !valid {
        return Err(de::Error::custom(
            "routing.contractOperationId must be skiff-contract-operation-v1:sha256:<64 lowercase hex>",
        ));
    }
    Ok(ContractOperationId::new(value))
}

pub(super) fn deserialize_required_nullable_string<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

fn deserialize_exact_string<'de, D>(
    deserializer: D,
    expected: &'static str,
    label: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value != expected {
        return Err(de::Error::custom(format!("{label} must be {expected}")));
    }
    Ok(value)
}
