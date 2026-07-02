use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::{
    identity::{identity_hash, identity_hash_with_label, validate_identity_prefix},
    paths::{service_id_artifact_json_path, service_id_artifact_path},
    pointer_parse::{
        parse_dev_reload_pointer, parse_service_assembly_pointer,
        reject_unsupported_pointer_aliases,
    },
    types::{ArtifactIndexPointer, ArtifactPointerFile, RootedArtifactPointerFile},
    utils::{object_string, read_json_file},
    SERVICE_BUILD_IDENTITY_PREFIX, SERVICE_BUILD_SCHEMA_VERSION,
    SERVICE_VERSION_POINTER_SCHEMA_VERSION,
};

pub fn load_service_version_build_pointers_from_roots(
    artifact_roots: &[PathBuf],
) -> anyhow::Result<Vec<RootedArtifactPointerFile>> {
    let mut pointers = Vec::new();
    for artifact_root in artifact_roots {
        pointers.extend(
            load_service_version_build_pointers_optional(artifact_root)?
                .into_iter()
                .map(|pointer| RootedArtifactPointerFile::new(artifact_root.clone(), pointer)),
        );
    }
    if pointers.is_empty() {
        anyhow::bail!(
            "artifact roots {} have no service version pointer JSON files",
            display_artifact_roots(artifact_roots)
        );
    }
    Ok(pointers)
}

pub fn load_dev_reload_pointers_from_roots(
    artifact_roots: &[PathBuf],
) -> anyhow::Result<Vec<RootedArtifactPointerFile>> {
    let mut pointers = Vec::new();
    let mut service_slots = HashMap::<String, usize>::new();
    for artifact_root in artifact_roots {
        for pointer in load_dev_reload_pointers_optional(artifact_root)? {
            let service_id = pointer.entry.service_id.clone();
            let rooted = RootedArtifactPointerFile::new(artifact_root.clone(), pointer);
            if let Some(slot) = service_slots.get(&service_id).copied() {
                pointers[slot] = rooted;
            } else {
                service_slots.insert(service_id, pointers.len());
                pointers.push(rooted);
            }
        }
    }
    if pointers.is_empty() {
        anyhow::bail!(
            "artifact roots {} have no dev reload service JSON files",
            display_artifact_roots(artifact_roots)
        );
    }
    Ok(pointers)
}

fn load_service_version_build_pointers_optional(
    artifact_root: &Path,
) -> anyhow::Result<Vec<ArtifactPointerFile>> {
    let version_root = artifact_root.join("versions").join("services");
    if !version_root.exists() {
        return Ok(Vec::new());
    }
    if !version_root.is_dir() {
        anyhow::bail!(
            "artifact versions dir {} is not a directory",
            version_root.display()
        );
    }
    let mut version_paths = collect_json_files_recursive(&version_root)?;
    version_paths.sort();
    if version_paths.is_empty() {
        return Ok(Vec::new());
    }
    load_service_version_build_pointers_from_paths(artifact_root, version_paths)
}

fn load_dev_reload_pointers_optional(
    artifact_root: &Path,
) -> anyhow::Result<Vec<ArtifactPointerFile>> {
    let pointer_dir = artifact_root.join("dev").join("services");
    if !pointer_dir.exists() {
        return Ok(Vec::new());
    }
    if !pointer_dir.is_dir() {
        anyhow::bail!(
            "artifact dev reload dir {} is not a directory",
            pointer_dir.display()
        );
    }
    let mut pointer_paths = collect_json_files_recursive(&pointer_dir)?;
    pointer_paths.sort();
    if pointer_paths.is_empty() {
        return Ok(Vec::new());
    }
    load_dev_reload_pointers_from_paths(pointer_paths)
}

fn load_service_version_build_pointers_from_paths(
    artifact_root: &Path,
    version_paths: Vec<PathBuf>,
) -> anyhow::Result<Vec<ArtifactPointerFile>> {
    let mut pointers = Vec::new();
    let mut seen_builds = std::collections::HashSet::new();
    for version_path in version_paths {
        let version_value = read_json_file(&version_path, "service version pointer")?;
        let version_pointer = parse_service_version_pointer(&version_value, &version_path)?;
        validate_service_version_file_name(&version_path, &version_pointer)?;
        if !seen_builds.insert((
            version_pointer.service_id.clone(),
            version_pointer.build_id.clone(),
        )) {
            continue;
        }
        let build_path = artifact_root
            .join("builds")
            .join("services")
            .join(service_id_artifact_path(&version_pointer.service_id)?)
            .join(format!(
                "{}.json",
                identity_hash_with_label(&version_pointer.build_id, "buildId")?
            ));
        let build_value = read_json_file(&build_path, "service build record")?;
        let entry = parse_service_build_record(&build_value, &build_path, &version_pointer)?;
        validate_service_build_file_name(&build_path, &entry)?;
        pointers.push(ArtifactPointerFile {
            path: build_path,
            entry,
        });
    }
    Ok(pointers)
}

fn load_dev_reload_pointers_from_paths(
    pointer_paths: Vec<PathBuf>,
) -> anyhow::Result<Vec<ArtifactPointerFile>> {
    let mut pointers = Vec::new();
    for pointer_path in pointer_paths {
        let pointer_value = read_json_file(&pointer_path, "dev reload pointer")?;
        let entry = parse_dev_reload_pointer(&pointer_value, &pointer_path)?;
        validate_dev_reload_file_name(&pointer_path, &entry)?;
        pointers.push(ArtifactPointerFile {
            path: pointer_path,
            entry,
        });
    }
    Ok(pointers)
}

fn display_artifact_roots(artifact_roots: &[PathBuf]) -> String {
    artifact_roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn validate_dev_reload_file_name(
    path: &Path,
    entry: &ArtifactIndexPointer,
) -> anyhow::Result<()> {
    let expected_path = Path::new("dev")
        .join("services")
        .join(service_id_artifact_json_path(&entry.service_id)?);
    if !path.ends_with(&expected_path) {
        anyhow::bail!(
            "dev reload pointer {} path does not match serviceId {}",
            path.display(),
            entry.service_id
        );
    }
    Ok(())
}

#[derive(Debug)]
struct ServiceVersionPointer {
    service_id: String,
    version: String,
    build_id: String,
}

fn collect_json_files_recursive(dir: &Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", dir.display()))?
    {
        let path = entry
            .map_err(|error| anyhow::anyhow!("failed to read {} entry: {error}", dir.display()))?
            .path();
        if path.is_dir() {
            paths.extend(collect_json_files_recursive(&path)?);
        } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn parse_service_version_pointer(
    value: &Value,
    version_path: &Path,
) -> anyhow::Result<ServiceVersionPointer> {
    let object = value.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "{} service version pointer must be an object",
            version_path.display()
        )
    })?;
    let schema_version = object_string(object, "schemaVersion")
        .ok_or_else(|| anyhow::anyhow!("{} schemaVersion is required", version_path.display()))?;
    if schema_version != SERVICE_VERSION_POINTER_SCHEMA_VERSION {
        anyhow::bail!(
            "{} schemaVersion must be {SERVICE_VERSION_POINTER_SCHEMA_VERSION}",
            version_path.display()
        );
    }
    let service_id = object_string(object, "serviceId")
        .ok_or_else(|| anyhow::anyhow!("{} serviceId is required", version_path.display()))?;
    let version = object_string(object, "version")
        .ok_or_else(|| anyhow::anyhow!("{} version is required", version_path.display()))?;
    let build_id = object_string(object, "buildId")
        .ok_or_else(|| anyhow::anyhow!("{} buildId is required", version_path.display()))?;
    validate_identity_prefix(
        &build_id,
        SERVICE_BUILD_IDENTITY_PREFIX,
        "service version pointer buildId",
    )?;
    Ok(ServiceVersionPointer {
        service_id,
        version,
        build_id,
    })
}

fn validate_service_version_file_name(
    path: &Path,
    version_pointer: &ServiceVersionPointer,
) -> anyhow::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "service version pointer {} has invalid file name",
                path.display()
            )
        })?;
    if file_name != format!("{}.json", version_pointer.version) {
        anyhow::bail!(
            "service version pointer {} must be named {}.json for version",
            path.display(),
            version_pointer.version
        );
    }
    let expected_path = Path::new("versions")
        .join("services")
        .join(service_id_artifact_path(&version_pointer.service_id)?)
        .join(file_name);
    if !path.ends_with(&expected_path) {
        anyhow::bail!(
            "service version pointer {} path does not match serviceId {}",
            path.display(),
            version_pointer.service_id
        );
    }
    Ok(())
}

fn parse_service_build_record(
    value: &Value,
    build_path: &Path,
    version_pointer: &ServiceVersionPointer,
) -> anyhow::Result<ArtifactIndexPointer> {
    let object = value.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "{} service build record must be an object",
            build_path.display()
        )
    })?;
    reject_unsupported_pointer_aliases(object, build_path)?;
    let schema_version = object_string(object, "schemaVersion")
        .ok_or_else(|| anyhow::anyhow!("{} schemaVersion is required", build_path.display()))?;
    if schema_version != SERVICE_BUILD_SCHEMA_VERSION {
        anyhow::bail!(
            "{} schemaVersion must be {SERVICE_BUILD_SCHEMA_VERSION}",
            build_path.display()
        );
    }
    let service_id = object_string(object, "serviceId")
        .ok_or_else(|| anyhow::anyhow!("{} serviceId is required", build_path.display()))?;
    let service_version = object_string(object, "serviceVersion")
        .ok_or_else(|| anyhow::anyhow!("{} serviceVersion is required", build_path.display()))?;
    let build_id = object_string(object, "buildId")
        .ok_or_else(|| anyhow::anyhow!("{} buildId is required", build_path.display()))?;
    if service_id != version_pointer.service_id {
        anyhow::bail!(
            "{} serviceId must match service version pointer serviceId",
            build_path.display()
        );
    }
    if service_version != version_pointer.version {
        anyhow::bail!(
            "{} serviceVersion must match service version pointer version",
            build_path.display()
        );
    }
    if build_id != version_pointer.build_id {
        anyhow::bail!(
            "{} buildId must match service version pointer buildId",
            build_path.display()
        );
    }
    validate_identity_prefix(
        &build_id,
        SERVICE_BUILD_IDENTITY_PREFIX,
        "service build record buildId",
    )?;
    reject_legacy_contract_identity_aliases(object, build_path)?;
    let contract_identity = object_string(object, "contractIdentity");
    if let Some(contract_identity) = contract_identity.as_deref() {
        identity_hash(contract_identity)?;
    }
    let service_assembly = parse_service_assembly_pointer(object, build_path)?;
    let service_unit_path = parse_service_unit_path(object)?;

    Ok(ArtifactIndexPointer {
        service_id,
        service_version: Some(service_version),
        build_id,
        contract_identity,
        implementation_identity: object_string(object, "implementationIdentity"),
        service_unit_path,
        service_assembly,
    })
}

fn reject_legacy_contract_identity_aliases(
    object: &serde_json::Map<String, Value>,
    build_path: &Path,
) -> anyhow::Result<()> {
    for alias in ["protocolIdentity", "serviceProtocolIdentity"] {
        if object.contains_key(alias) {
            anyhow::bail!(
                "{} {} is not supported in service build records; use contractIdentity",
                build_path.display(),
                alias
            );
        }
    }
    Ok(())
}

fn validate_service_build_file_name(
    path: &Path,
    entry: &ArtifactIndexPointer,
) -> anyhow::Result<()> {
    let build_id = entry.build_id.as_str();
    let build_hash = identity_hash_with_label(build_id, "buildId")?;
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        anyhow::bail!(
            "service build record {} has invalid file name",
            path.display()
        );
    };
    let Some(stem) = file_name.strip_suffix(".json") else {
        anyhow::bail!(
            "service build record {} must end with .json",
            path.display()
        );
    };
    if stem != build_hash {
        anyhow::bail!(
            "service build record {} file name {} does not match buildId hash {}",
            path.display(),
            stem,
            build_hash
        );
    }

    let expected_path = Path::new("builds")
        .join("services")
        .join(service_id_artifact_path(&entry.service_id)?)
        .join(file_name);
    if !path.ends_with(&expected_path) {
        anyhow::bail!(
            "service build record {} path does not match serviceId {}",
            path.display(),
            entry.service_id
        );
    }
    Ok(())
}

fn parse_service_unit_path(
    object: &serde_json::Map<String, Value>,
) -> anyhow::Result<Option<std::path::PathBuf>> {
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
    Ok(Some(std::path::PathBuf::from(path)))
}
