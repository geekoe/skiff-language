use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{
    identity::{identity_hash, identity_hash_with_label, validate_identity_prefix},
    types::{ArtifactIndexPointer, ServiceAssemblyPointer},
    utils::{is_sha256_hash, map_string, object_string},
    SERVICE_BUILD_IDENTITY_PREFIX,
};

pub(super) fn parse_dev_reload_pointer(
    value: &Value,
    pointer_path: &Path,
) -> anyhow::Result<ArtifactIndexPointer> {
    let object = value.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "{} dev reload pointer must be an object",
            pointer_path.display()
        )
    })?;
    reject_unsupported_pointer_aliases(object, pointer_path)?;

    let mode = object_string(object, "mode")
        .ok_or_else(|| anyhow::anyhow!("{} mode is required", pointer_path.display()))?;
    if mode != "dev" {
        anyhow::bail!("{} mode must be dev, got {}", pointer_path.display(), mode);
    }

    let service_id = object_string(object, "serviceId")
        .ok_or_else(|| anyhow::anyhow!("{} serviceId is required", pointer_path.display()))?;
    let profile = object_string(object, "profile")
        .ok_or_else(|| anyhow::anyhow!("{} profile is required", pointer_path.display()))?;
    if profile.is_empty() {
        anyhow::bail!("{} profile must not be empty", pointer_path.display());
    }
    let protocol_identity = object_string(object, "protocolIdentity").ok_or_else(|| {
        anyhow::anyhow!("{} protocolIdentity is required", pointer_path.display())
    })?;
    let contract_hash = object_string(object, "contractHash")
        .ok_or_else(|| anyhow::anyhow!("{} contractHash is required", pointer_path.display()))?;
    validate_contract_hash_field(&contract_hash, &protocol_identity, pointer_path)?;
    let build_id = object_string(object, "buildId")
        .ok_or_else(|| anyhow::anyhow!("{} buildId is required", pointer_path.display()))?;
    validate_identity_prefix(
        &build_id,
        SERVICE_BUILD_IDENTITY_PREFIX,
        "dev reload pointer buildId",
    )?;
    let service_assembly = parse_service_assembly_pointer(object, pointer_path)?;
    let service_unit_path = parse_service_unit_path(object)?;
    validate_dev_build_id_matches_service_assembly(&build_id, &service_assembly, pointer_path)?;

    Ok(ArtifactIndexPointer {
        service_id,
        service_version: object_string(object, "serviceVersion")
            .or_else(|| object_string(object, "version")),
        build_id,
        contract_identity: Some(protocol_identity),
        implementation_identity: object_string(object, "implementationIdentity"),
        service_unit_path,
        service_assembly,
    })
}

fn validate_dev_build_id_matches_service_assembly(
    build_id: &str,
    service_assembly: &ServiceAssemblyPointer,
    pointer_path: &Path,
) -> anyhow::Result<()> {
    let assembly_identity = service_assembly
        .assembly_identity
        .as_deref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} serviceAssembly.assemblyIdentity is required",
                pointer_path.display()
            )
        })?;
    let assembly_hash = identity_hash_with_label(assembly_identity, "serviceAssembly")?;
    let expected_build_id = format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{assembly_hash}");
    if build_id != expected_build_id {
        anyhow::bail!(
            "{} buildId must match serviceAssembly assemblyIdentity hash; expected {}",
            pointer_path.display(),
            expected_build_id
        );
    }
    Ok(())
}

pub(super) fn reject_unsupported_pointer_aliases(
    object: &Map<String, Value>,
    index_path: &Path,
) -> anyhow::Result<()> {
    if object.contains_key("serviceIr")
        || object.contains_key("serviceIrPath")
        || object_string(object, "artifactIdentity")
            .as_deref()
            .is_some_and(|identity| identity.starts_with("skiff-service-ir-v1"))
    {
        anyhow::bail!(
            "{} legacy serviceIr pointers are not supported",
            index_path.display()
        );
    }
    if object.contains_key("artifactIdentity") {
        anyhow::bail!(
            "{} artifactIdentity is not supported in artifact pointers",
            index_path.display()
        );
    }
    if object.contains_key("serviceAssemblyRef") {
        anyhow::bail!(
            "{} serviceAssemblyRef is not supported in artifact pointers",
            index_path.display()
        );
    }
    Ok(())
}

fn validate_contract_hash_field(
    contract_hash: &str,
    protocol_identity: &str,
    pointer_path: &Path,
) -> anyhow::Result<()> {
    let hash = contract_hash
        .strip_prefix("sha256:")
        .unwrap_or(contract_hash);
    if !is_sha256_hash(hash) {
        anyhow::bail!(
            "{} contractHash must be sha256:<64 lowercase hex> or <64 lowercase hex>",
            pointer_path.display()
        );
    }

    let expected_hash = identity_hash(protocol_identity)?;
    if hash != expected_hash {
        anyhow::bail!(
            "{} contractHash {} does not match protocolIdentity hash {}",
            pointer_path.display(),
            contract_hash,
            expected_hash
        );
    }

    Ok(())
}

pub(super) fn parse_service_assembly_pointer(
    object: &Map<String, Value>,
    index_path: &Path,
) -> anyhow::Result<ServiceAssemblyPointer> {
    let Some(value) = object.get("serviceAssembly") else {
        if object.contains_key("serviceIr") {
            anyhow::bail!(
                "{} legacy serviceIr pointers are not supported; serviceAssembly is required",
                index_path.display()
            );
        }
        anyhow::bail!("{} serviceAssembly is required", index_path.display());
    };

    let assembly = value.as_object().ok_or_else(|| {
        anyhow::anyhow!("{} serviceAssembly must be an object", index_path.display())
    })?;
    for key in ["path", "artifactPath", "identity", "artifactIdentity"] {
        if assembly.contains_key(key) {
            anyhow::bail!(
                "{} serviceAssembly.{} is not supported",
                index_path.display(),
                key
            );
        }
    }
    let path = map_string(Some(assembly), "assemblyPath").ok_or_else(|| {
        anyhow::anyhow!(
            "{} serviceAssembly.assemblyPath is required",
            index_path.display()
        )
    })?;
    let assembly_identity = map_string(Some(assembly), "assemblyIdentity").ok_or_else(|| {
        anyhow::anyhow!(
            "{} serviceAssembly.assemblyIdentity is required",
            index_path.display()
        )
    })?;
    if assembly_identity.starts_with("skiff-service-ir-v1") {
        anyhow::bail!(
            "{} legacy serviceIr pointers are not supported",
            index_path.display()
        );
    }
    Ok(ServiceAssemblyPointer {
        path: PathBuf::from(path),
        assembly_identity: Some(assembly_identity),
    })
}

fn parse_service_unit_path(object: &Map<String, Value>) -> anyhow::Result<Option<PathBuf>> {
    if object.contains_key("serviceUnitPath") {
        anyhow::bail!("serviceUnitPath is not supported; use serviceUnit.unitPath");
    }
    let Some(value) = object.get("serviceUnit") else {
        return Ok(None);
    };
    let service_unit = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("serviceUnit must be an object with unitPath"))?;
    let path = service_unit
        .get("unitPath")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("serviceUnit requires unitPath"))?;
    Ok(Some(PathBuf::from(path)))
}
