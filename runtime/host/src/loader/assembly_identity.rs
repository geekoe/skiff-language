use std::path::Path;

use serde_json::{json, Value};
use skiff_runtime_loader::{service_id_artifact_path, ArtifactIndexPointer};

use super::{
    identity::{identity_hash_with_label, validate_identity_alias, validate_identity_prefix},
    projection::reject_removed_service_assembly_fields,
    service_http::service_http_hash_input,
    value_sha256, SERVICE_ASSEMBLY_IDENTITY_PREFIX,
};

pub(super) fn validate_service_assembly_identity(
    entry: &ArtifactIndexPointer,
    assembly: &Value,
) -> anyhow::Result<String> {
    let index_identity = entry.service_assembly.assembly_identity.as_deref();
    let assembly_identity = assembly
        .pointer("/service/assemblyIdentity")
        .and_then(Value::as_str);

    if let (Some(index_identity), Some(assembly_identity)) = (index_identity, assembly_identity) {
        validate_identity_alias(
            assembly_identity,
            index_identity,
            "service.assemblyIdentity",
        )?;
    }
    let identity = index_identity.or(assembly_identity).ok_or_else(|| {
            anyhow::anyhow!(
                "serviceAssembly {} requires assemblyIdentity in index serviceAssembly or assembly service.assemblyIdentity",
                entry.service_assembly.path.display()
            )
        })?;
    validate_identity_prefix(
        identity,
        SERVICE_ASSEMBLY_IDENTITY_PREFIX,
        "serviceAssembly",
    )?;
    validate_service_assembly_content_identity(assembly, identity)?;
    Ok(identity.to_string())
}

fn validate_service_assembly_content_identity(
    assembly: &Value,
    assembly_identity: &str,
) -> anyhow::Result<()> {
    let expected_hash = identity_hash_with_label(assembly_identity, "serviceAssembly")?;
    let actual_hash = value_sha256(&service_assembly_hash_input(assembly)?)?;
    if actual_hash != expected_hash {
        anyhow::bail!(
            "serviceAssembly content sha256 {} does not match assemblyIdentity hash {}",
            actual_hash,
            expected_hash
        );
    }
    Ok(())
}

pub(super) fn service_assembly_hash_input(assembly: &Value) -> anyhow::Result<Value> {
    reject_removed_service_assembly_fields(assembly, Path::new("<service-assembly>"))?;
    let service = assembly_service_hash_input(assembly)?;
    Ok(json!({
        "schemaVersion": assembly.get("schemaVersion").cloned().unwrap_or(Value::Null),
        "kind": assembly.get("kind").cloned().unwrap_or(Value::Null),
        "service": service,
        "files": assembly.get("files").cloned().unwrap_or(Value::Null),
        "preludeIdentity": assembly.get("preludeIdentity").cloned().unwrap_or(Value::Null),
        "prelude": assembly.get("prelude").cloned().unwrap_or(Value::Null),
        "packageConfigs": assembly.get("packageConfigs").cloned().unwrap_or(Value::Null),
        "configShape": assembly.get("configShape").cloned().unwrap_or(Value::Null),
        "configUses": assembly.get("configUses").cloned().unwrap_or(Value::Null),
        "configActivation": assembly.get("configActivation").cloned().unwrap_or(Value::Null),
        "configRequirements": assembly.get("configRequirements").cloned().unwrap_or(Value::Null),
        "db": assembly.get("db").cloned().unwrap_or(Value::Null),
        "operations": assembly.get("operations").cloned().unwrap_or(Value::Null),
        "gateway": assembly.get("gateway").cloned().unwrap_or(Value::Null),
        "timeout": assembly.get("timeout").cloned().unwrap_or(Value::Null),
        "dependencyLock": assembly.get("dependencyLock").cloned().unwrap_or(Value::Null),
        "serviceUnit": assembly.get("serviceUnit").cloned().unwrap_or(Value::Null),
        "sourceMap": assembly.get("sourceMap").cloned().unwrap_or(Value::Null),
    }))
}

fn assembly_service_hash_input(assembly: &Value) -> anyhow::Result<Value> {
    let mut service = serde_json::Map::new();
    service.insert(
        "id".to_string(),
        assembly
            .pointer("/service/id")
            .cloned()
            .unwrap_or(Value::Null),
    );
    service.insert(
        "revisionId".to_string(),
        assembly
            .pointer("/service/revisionId")
            .cloned()
            .unwrap_or(Value::Null),
    );
    service.insert(
        "protocolIdentity".to_string(),
        assembly
            .pointer("/service/protocolIdentity")
            .cloned()
            .unwrap_or(Value::Null),
    );
    if let Some(access) = assembly.pointer("/service/access") {
        service.insert("access".to_string(), access.clone());
    }
    if let Some(http) = service_http_hash_input(assembly.get("service").unwrap_or(&Value::Null))? {
        service.insert("http".to_string(), http);
    }
    service.insert(
        "api".to_string(),
        assembly
            .pointer("/service/api")
            .cloned()
            .unwrap_or(Value::Null),
    );
    Ok(Value::Object(service))
}

pub(super) fn validate_service_assembly_path_identity(
    assembly_path: &Path,
    service_id: &str,
    assembly_identity: &str,
) -> anyhow::Result<()> {
    let canonical_prefix = Path::new("assemblies").join("services");
    if !assembly_path.starts_with(&canonical_prefix) {
        return Ok(());
    }

    let Some(file_name) = assembly_path.file_name().and_then(|value| value.to_str()) else {
        anyhow::bail!(
            "serviceAssembly path {} has invalid file name",
            assembly_path.display()
        );
    };
    let Some(stem) = file_name.strip_suffix(".json") else {
        anyhow::bail!(
            "serviceAssembly path {} must end with .json",
            assembly_path.display()
        );
    };
    let identity_hash = identity_hash_with_label(assembly_identity, "serviceAssembly")?;
    if stem != identity_hash {
        anyhow::bail!(
            "serviceAssembly path {} identity hash {} does not match assemblyIdentity hash {}",
            assembly_path.display(),
            stem,
            identity_hash
        );
    }
    let expected_path = canonical_prefix
        .join(service_id_artifact_path(service_id)?)
        .join(file_name);
    if assembly_path != expected_path {
        anyhow::bail!(
            "serviceAssembly path {} service path does not match index serviceId {}",
            assembly_path.display(),
            service_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
