use serde_json::Value;
use skiff_artifact_model::schema::{SERVICE_ASSEMBLY_KIND, SERVICE_ASSEMBLY_SCHEMA_VERSION};

use crate::{
    framed_identity,
    framing::{is_lowercase_sha256, sha256_hex},
    ArtifactIdentityError, Result, SERVICE_ASSEMBLY_IDENTITY_PREFIX, SERVICE_BUILD_IDENTITY_PREFIX,
};

pub fn service_assembly_identity_projection(assembly: &Value) -> Result<Value> {
    let object = assembly
        .as_object()
        .ok_or_else(|| invalid_service_assembly("must be an object"))?;
    require_string(object, "schemaVersion", SERVICE_ASSEMBLY_SCHEMA_VERSION)?;
    require_string(object, "kind", SERVICE_ASSEMBLY_KIND)?;
    let service = object
        .get("service")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_service_assembly("service must be an object"))?;
    for field in ["id", "revisionId", "protocolIdentity"] {
        required_value(service, field)?;
    }
    required_value(service, "api")?;
    required_value(object, "serviceUnit")?;

    let mut projection = object.clone();
    let projected_service = projection
        .get_mut("service")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            invalid_service_assembly("service must remain an object during projection")
        })?;
    projected_service.remove("assemblyIdentity");
    Ok(Value::Object(projection))
}

pub fn service_assembly_hash(assembly: &Value) -> Result<String> {
    let projection = service_assembly_identity_projection(assembly)?;
    let bytes = skiff_canonical_json::canonical_json_bytes(&projection).map_err(|error| {
        invalid_service_assembly(format!("identity projection cannot serialize: {error}"))
    })?;
    Ok(sha256_hex(&bytes))
}

pub fn service_assembly_identity(assembly: &Value) -> Result<String> {
    Ok(framed_identity(
        SERVICE_ASSEMBLY_IDENTITY_PREFIX,
        &service_assembly_hash(assembly)?,
    ))
}

pub fn validate_service_assembly_identity(assembly: &Value, declared_identity: &str) -> Result<()> {
    let computed = service_assembly_identity(assembly)?;
    if declared_identity != computed {
        return Err(ArtifactIdentityError::ServiceAssemblyIdentityMismatch {
            declared: declared_identity.to_string(),
            computed,
        });
    }
    let embedded = assembly
        .pointer("/service/assemblyIdentity")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_service_assembly("service.assemblyIdentity is required"))?;
    if embedded != declared_identity {
        return Err(ArtifactIdentityError::ServiceAssemblyIdentityMismatch {
            declared: embedded.to_string(),
            computed: declared_identity.to_string(),
        });
    }
    Ok(())
}

pub fn service_build_identity_hash(build_identity: &str) -> Result<&str> {
    build_identity
        .strip_prefix(&format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:"))
        .filter(|hash| is_lowercase_sha256(hash))
        .ok_or_else(
            || ArtifactIdentityError::InvalidRuntimeProgramBuildIdentity {
                identity: build_identity.to_string(),
            },
        )
}

pub fn service_build_identity_from_assembly_identity(assembly_identity: &str) -> Result<String> {
    let hash = service_assembly_identity_hash(assembly_identity)?;
    Ok(format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{hash}"))
}

pub(crate) fn service_assembly_identity_hash(identity: &str) -> Result<&str> {
    identity
        .strip_prefix(&format!("{SERVICE_ASSEMBLY_IDENTITY_PREFIX}:"))
        .filter(|hash| is_lowercase_sha256(hash))
        .ok_or_else(|| {
            invalid_service_assembly(format!(
                "identity {identity} must use {SERVICE_ASSEMBLY_IDENTITY_PREFIX}:<64 lowercase hex>"
            ))
        })
}

pub(crate) fn invalid_service_assembly(message: impl Into<String>) -> ArtifactIdentityError {
    ArtifactIdentityError::InvalidServiceAssembly {
        message: message.into(),
    }
}

fn required_value<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a Value> {
    object
        .get(field)
        .ok_or_else(|| invalid_service_assembly(format!("{field} is required")))
}

fn require_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
    expected: &str,
) -> Result<()> {
    let actual = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_service_assembly(format!("{field} must be a string")))?;
    if actual != expected {
        return Err(invalid_service_assembly(format!(
            "{field} must be {expected}"
        )));
    }
    Ok(())
}
