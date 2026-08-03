use serde::{de, Deserialize, Deserializer};
use skiff_artifact_model::{
    deserialize_activation_generation, validate_activation_generation,
    validate_runtime_assembly_identity, AssemblyIdentity, GatewayEntryIdentity,
    ServiceDeploymentRef,
};

use crate::protocol::RUNTIME_FRAME_SCHEMA_VERSION;

const MAX_RUNTIME_ASSEMBLY_REQUEST_ID_BYTES: usize = 1024;
const MAX_WEBSOCKET_JSONRPC_METHOD_BYTES: usize = 256;
const MAX_WEBSOCKET_JSONRPC_BUSINESS_IDENTITY_BYTES: usize = 1024;
const MAX_TEST_CASE_CORRELATION_TOKEN_BYTES: usize = 256;

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

pub(super) fn deserialize_unary_dispatch_mode<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(
        deserializer,
        "unary",
        "runtimeAssembly websocketConnect request.start mode",
    )
}

pub(super) fn deserialize_task_unary_dispatch_mode<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(
        deserializer,
        "unary",
        "runtimeAssembly task request.start mode",
    )
}

pub(super) fn deserialize_service_caller_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "service", "runtimeAssembly dispatch caller.kind")
}

pub(super) fn deserialize_task_invocation_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(
        deserializer,
        "task",
        "runtimeAssembly request.start invocation.kind",
    )
}

pub(super) fn deserialize_task_target_kind<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(
        deserializer,
        "function",
        "runtimeAssembly task invocation.targetKind",
    )
}

pub(super) fn deserialize_task_target<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_canonical_string(
        deserializer,
        MAX_RUNTIME_ASSEMBLY_REQUEST_ID_BYTES,
        "runtimeAssembly task invocation.target",
    )
}

pub(super) fn deserialize_optional_test_case_capability<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_test_case_correlation_token(deserializer, "runtimeAssembly testCaseCapability")
        .map(Some)
}

pub(super) fn deserialize_optional_test_case_parent_request_id<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_test_case_correlation_token(deserializer, "runtimeAssembly testCaseParentRequestId")
        .map(Some)
}

fn deserialize_test_case_correlation_token<'de, D>(
    deserializer: D,
    label: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value =
        deserialize_bounded_string(deserializer, MAX_TEST_CASE_CORRELATION_TOKEN_BYTES, label)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(de::Error::custom(format!(
            "{label} must be a test correlation token"
        )));
    }
    Ok(value)
}

pub(super) fn deserialize_websocket_jsonrpc_unary_dispatch_mode<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(
        deserializer,
        "unary",
        "runtimeAssembly websocketJsonRpc request.start mode",
    )
}

pub(super) fn deserialize_runtime_assembly_websocket_jsonrpc_request_id<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_canonical_string(
        deserializer,
        MAX_RUNTIME_ASSEMBLY_REQUEST_ID_BYTES,
        "runtimeAssembly websocketJsonRpc requestId",
    )
}

pub(super) fn deserialize_runtime_assembly_websocket_jsonrpc_method<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_string(
        deserializer,
        MAX_WEBSOCKET_JSONRPC_METHOD_BYTES,
        "runtimeAssembly websocketJsonRpc routing.ingress.method",
    )
}

pub(super) fn deserialize_runtime_assembly_websocket_jsonrpc_connection_id<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty()
        || value.len() > 255
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'~' | b'-')
        })
    {
        return Err(de::Error::custom(
            "runtimeAssembly websocketJsonRpc connectionId is not canonical",
        ));
    }
    Ok(value)
}

pub(super) fn deserialize_optional_websocket_jsonrpc_business_identity<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_canonical_string(
        deserializer,
        MAX_WEBSOCKET_JSONRPC_BUSINESS_IDENTITY_BYTES,
        "runtimeAssembly websocketJsonRpc businessIdentity",
    )
    .map(Some)
}

pub(super) fn deserialize_response_end_type<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_exact_string(deserializer, "response.end", "binary frame type")
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

pub(super) fn deserialize_gateway_entry_identity<'de, D>(
    deserializer: D,
) -> Result<GatewayEntryIdentity, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    GatewayEntryIdentity::parse(value).map_err(de::Error::custom)
}

pub(super) fn deserialize_service_deployment_ref<'de, D>(
    deserializer: D,
) -> Result<ServiceDeploymentRef, D::Error>
where
    D: Deserializer<'de>,
{
    let value = ServiceDeploymentRef::deserialize(deserializer)?;
    if value.service_id.is_empty()
        || value.contract_version.is_empty()
        || value.deployment_revision.as_str().is_empty()
    {
        return Err(de::Error::custom(
            "request.start routing.deployment coordinate must contain non-empty strings",
        ));
    }
    let identity = value.deployment_artifact_identity.as_str();
    let hash = identity
        .strip_prefix("skiff-deployment-artifact-v4:sha256:")
        .filter(|hash| hash.len() == 64)
        .filter(|hash| {
            hash.bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        });
    if hash.is_none() {
        return Err(de::Error::custom(
            "request.start routing.deployment.deploymentArtifactIdentity must be skiff-deployment-artifact-v4:sha256:<64 lowercase hex>",
        ));
    }
    Ok(value)
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

fn deserialize_bounded_string<'de, D>(
    deserializer: D,
    max_bytes: usize,
    label: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() || value.as_bytes().len() > max_bytes {
        return Err(de::Error::custom(format!(
            "{label} must be a bounded non-empty string"
        )));
    }
    Ok(value)
}

fn deserialize_bounded_canonical_string<'de, D>(
    deserializer: D,
    max_bytes: usize,
    label: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = deserialize_bounded_string(deserializer, max_bytes, label)?;
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(de::Error::custom(format!(
            "{label} must be a canonical string"
        )));
    }
    Ok(value)
}
