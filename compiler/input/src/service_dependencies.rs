use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Deserialize};
use serde_json::Value;
use skiff_artifact_identity::{
    service_build_identity_from_assembly_identity, service_build_identity_hash,
    validate_publication_abi_identity, validate_service_artifact_closure, PackageUnitArtifactRef,
    ServiceAssemblyArtifactRef, ServiceUnitArtifactRef,
};
use skiff_artifact_model::{
    schema::{SERVICE_BUILD_SCHEMA_VERSION, SERVICE_VERSION_POINTER_SCHEMA_VERSION},
    PublicationAbiUnit, ServiceDependencyConstraint, ServiceUnit,
};
use skiff_compiler_core::id::PublicationId;

use crate::{
    error::InputAssemblyError, ResolvedServiceDependencies, ServiceDependency,
    ServiceDependencyLockEntry,
};

pub fn service_dependency_aliases(dependencies: &[ServiceDependency]) -> BTreeSet<String> {
    dependencies
        .iter()
        .map(|dependency| dependency.alias.clone())
        .collect()
}

pub fn resolve_service_dependencies(
    dependencies: &[ServiceDependency],
    artifact_roots: &[PathBuf],
) -> Result<ResolvedServiceDependencies, InputAssemblyError> {
    if dependencies.is_empty() {
        return Ok(ResolvedServiceDependencies::default());
    }
    if artifact_roots.is_empty() {
        return Err(InputAssemblyError::Validation {
            message:
                "service dependencies require --service-artifact-root so callee artifacts can be resolved"
                    .to_string(),
        });
    }

    let mut constraints = Vec::with_capacity(dependencies.len());
    let mut lock = Vec::with_capacity(dependencies.len());
    for dependency in dependencies {
        let pointer = resolve_service_artifact_pointer(artifact_roots, dependency)?;
        let resolved = load_resolved_service_dependency_artifact(dependency, &pointer)?;
        lock.push(ServiceDependencyLockEntry::from_resolved_service(
            dependency, &resolved,
        ));
        constraints.push(resolved);
    }
    Ok(ResolvedServiceDependencies::new(constraints, lock))
}

#[derive(Debug)]
struct ResolvedServiceArtifactPointer {
    root: PathBuf,
    pointer_path: PathBuf,
    service_assembly: ServiceAssemblyArtifactRef,
    service_unit: ServiceUnitArtifactRef,
    package_units: Vec<PackageUnitArtifactRef>,
}

fn load_resolved_service_dependency_artifact(
    dependency: &ServiceDependency,
    pointer: &ResolvedServiceArtifactPointer,
) -> Result<ServiceDependencyConstraint, InputAssemblyError> {
    let validated = validate_service_artifact_closure(
        &pointer.root,
        &dependency.id,
        &pointer.service_assembly.assembly_identity,
        &pointer.service_assembly.assembly_path,
        &pointer.service_unit,
        &pointer.package_units,
    )
    .map_err(|error| InputAssemblyError::Validation {
        message: format!(
            "{} service artifact closure validation failed: {error}",
            pointer.pointer_path.display()
        ),
    })?;
    let service_unit: ServiceUnit =
        serde_json::from_value(validated.service_unit.value).map_err(|error| {
            InputAssemblyError::Validation {
                message: format!(
                    "{} validated service unit is invalid: {error}",
                    validated.service_unit.path
                ),
            }
        })?;
    validate_service_dependency_unit(&service_unit, dependency, &validated.service_unit.path)?;

    Ok(ServiceDependencyConstraint {
        id: dependency.id.clone(),
        version: dependency.version.clone(),
        alias: dependency.alias.clone(),
        build_id: validated.dynamic_build_id,
        service_protocol_identity: service_unit.protocol_identity,
        publication_abi: service_unit.publication_abi,
    })
}

fn resolve_service_artifact_pointer(
    roots: &[PathBuf],
    dependency: &ServiceDependency,
) -> Result<ResolvedServiceArtifactPointer, InputAssemblyError> {
    let service_path = service_artifact_path(dependency)?;
    let mut searched_roots = Vec::with_capacity(roots.len());
    for root in roots {
        searched_roots.push(root.display().to_string());
        if let Some(pointer) =
            resolve_service_artifact_pointer_from_root(root, dependency, &service_path)?
        {
            return Ok(pointer);
        }
    }
    Err(InputAssemblyError::Validation {
        message: format!(
            "service dependency {}@{} was not found under service artifact roots {}",
            dependency.id,
            dependency.version,
            searched_roots.join(", ")
        ),
    })
}

fn resolve_service_artifact_pointer_from_root(
    root: &Path,
    dependency: &ServiceDependency,
    service_path: &str,
) -> Result<Option<ResolvedServiceArtifactPointer>, InputAssemblyError> {
    let dev_path = root
        .join("dev")
        .join("services")
        .join(format!("{service_path}.json"));
    if dev_path.is_file() {
        let value = read_json(&dev_path)?;
        return parse_dev_reload_pointer(root, dependency, &dev_path, &value).map(Some);
    }

    let version_path = root
        .join("versions")
        .join("services")
        .join(service_path)
        .join(format!("{}.json", dependency.version));
    if !version_path.is_file() {
        return Ok(None);
    }
    let version_pointer: ServiceVersionPointer = parse_typed(
        read_json(&version_path)?,
        &version_path,
        "service version pointer",
    )?;
    validate_service_version_pointer(&version_pointer, dependency, &version_path)?;
    let build_hash = service_build_identity_hash(&version_pointer.build_id).map_err(|error| {
        InputAssemblyError::Validation {
            message: format!("{} buildId is invalid: {error}", version_path.display()),
        }
    })?;
    let build_path = root
        .join("builds")
        .join("services")
        .join(service_path)
        .join(format!("{build_hash}.json"));
    let build_value = read_json(&build_path)?;
    parse_service_build_record(
        root,
        dependency,
        &version_pointer,
        &build_path,
        &build_value,
    )
    .map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ServiceVersionPointer {
    schema_version: String,
    service_id: String,
    version: String,
    build_id: String,
}

fn validate_service_version_pointer(
    pointer: &ServiceVersionPointer,
    dependency: &ServiceDependency,
    path: &Path,
) -> Result<(), InputAssemblyError> {
    if pointer.schema_version != SERVICE_VERSION_POINTER_SCHEMA_VERSION {
        return invalid_pointer(
            path,
            format!("schemaVersion must be {SERVICE_VERSION_POINTER_SCHEMA_VERSION}"),
        );
    }
    if pointer.service_id != dependency.id || pointer.version != dependency.version {
        return invalid_pointer(
            path,
            format!(
                "coordinates {}@{} do not match dependency {}@{}",
                pointer.service_id, pointer.version, dependency.id, dependency.version
            ),
        );
    }
    service_build_identity_hash(&pointer.build_id).map_err(|error| {
        InputAssemblyError::Validation {
            message: format!("{} buildId is invalid: {error}", path.display()),
        }
    })?;
    Ok(())
}

fn parse_dev_reload_pointer(
    root: &Path,
    dependency: &ServiceDependency,
    path: &Path,
    value: &Value,
) -> Result<ResolvedServiceArtifactPointer, InputAssemblyError> {
    let object = pointer_object(value, path, "dev reload pointer")?;
    reject_pointer_aliases(object, path)?;
    require_exact_string(object, "mode", "dev", path)?;
    require_exact_string(object, "serviceId", &dependency.id, path)?;
    if let Some(version) = object.get("serviceVersion") {
        let version = version
            .as_str()
            .ok_or_else(|| InputAssemblyError::Validation {
                message: format!("{} serviceVersion must be a string", path.display()),
            })?;
        if version != dependency.version {
            return invalid_pointer(
                path,
                format!(
                    "serviceVersion {version} does not match dependency version {}",
                    dependency.version
                ),
            );
        }
    }
    let build_id = required_string(object, "buildId", path)?;
    service_build_identity_hash(build_id).map_err(|error| InputAssemblyError::Validation {
        message: format!("{} buildId is invalid: {error}", path.display()),
    })?;
    let pointer = artifact_pointer_refs(root, path, object)?;
    let expected_build_id =
        service_build_identity_from_assembly_identity(&pointer.service_assembly.assembly_identity)
            .map_err(|error| InputAssemblyError::Validation {
                message: format!(
                    "{} serviceAssembly identity is invalid: {error}",
                    path.display()
                ),
            })?;
    if build_id != expected_build_id {
        return invalid_pointer(path, "buildId must match serviceAssembly.assemblyIdentity");
    }
    Ok(pointer)
}

fn parse_service_build_record(
    root: &Path,
    dependency: &ServiceDependency,
    version_pointer: &ServiceVersionPointer,
    path: &Path,
    value: &Value,
) -> Result<ResolvedServiceArtifactPointer, InputAssemblyError> {
    let object = pointer_object(value, path, "service build record")?;
    reject_pointer_aliases(object, path)?;
    require_exact_string(object, "schemaVersion", SERVICE_BUILD_SCHEMA_VERSION, path)?;
    require_exact_string(object, "serviceId", &dependency.id, path)?;
    require_exact_string(object, "serviceVersion", &dependency.version, path)?;
    require_exact_string(object, "buildId", &version_pointer.build_id, path)?;
    artifact_pointer_refs(root, path, object)
}

fn artifact_pointer_refs(
    root: &Path,
    path: &Path,
    object: &serde_json::Map<String, Value>,
) -> Result<ResolvedServiceArtifactPointer, InputAssemblyError> {
    Ok(ResolvedServiceArtifactPointer {
        root: root.to_path_buf(),
        pointer_path: path.to_path_buf(),
        service_assembly: parse_pointer_field(object, "serviceAssembly", path)?,
        service_unit: parse_pointer_field(object, "serviceUnit", path)?,
        package_units: parse_pointer_field(object, "packageUnits", path)?,
    })
}

fn parse_pointer_field<T: DeserializeOwned>(
    object: &serde_json::Map<String, Value>,
    field: &str,
    path: &Path,
) -> Result<T, InputAssemblyError> {
    let value = object
        .get(field)
        .cloned()
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{} {field} is required", path.display()),
        })?;
    parse_typed(value, path, field)
}

fn parse_typed<T: DeserializeOwned>(
    value: Value,
    path: &Path,
    label: &str,
) -> Result<T, InputAssemblyError> {
    serde_json::from_value(value).map_err(|error| InputAssemblyError::Validation {
        message: format!("{} {label} is invalid: {error}", path.display()),
    })
}

fn pointer_object<'a>(
    value: &'a Value,
    path: &Path,
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, InputAssemblyError> {
    value
        .as_object()
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{} {label} must be an object", path.display()),
        })
}

fn reject_pointer_aliases(
    object: &serde_json::Map<String, Value>,
    path: &Path,
) -> Result<(), InputAssemblyError> {
    for alias in [
        "serviceIr",
        "serviceIrPath",
        "artifactIdentity",
        "serviceAssemblyRef",
        "service_id",
        "service_version",
        "build_id",
        "version",
        "service_unit",
        "service_assembly",
        "package_units",
    ] {
        if object.contains_key(alias) {
            return invalid_pointer(
                path,
                format!("legacy pointer field {alias} is not supported"),
            );
        }
    }
    Ok(())
}

fn require_exact_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
    expected: &str,
    path: &Path,
) -> Result<(), InputAssemblyError> {
    let actual = required_string(object, field, path)?;
    if actual != expected {
        return invalid_pointer(path, format!("{field} must be {expected}, got {actual}"));
    }
    Ok(())
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    path: &Path,
) -> Result<&'a str, InputAssemblyError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{} {field} must be a non-empty string", path.display()),
        })
}

fn service_artifact_path(dependency: &ServiceDependency) -> Result<String, InputAssemblyError> {
    Ok(PublicationId::parse(&dependency.id)
        .map_err(|error| InputAssemblyError::Validation {
            message: format!(
                "service dependency {} has invalid id: {error}",
                dependency.id
            ),
        })?
        .artifact_path())
}

fn validate_service_dependency_unit(
    service_unit: &ServiceUnit,
    dependency: &ServiceDependency,
    service_unit_path: &str,
) -> Result<(), InputAssemblyError> {
    if service_unit.service.id != dependency.id || service_unit.version != dependency.version {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} coordinates {}@{} do not match dependency {}@{}",
                service_unit.service.id, service_unit.version, dependency.id, dependency.version
            ),
        });
    }
    validate_service_dependency_publication_abi(
        &service_unit.publication_abi,
        service_unit_path,
        dependency,
    )
}

fn validate_service_dependency_publication_abi(
    publication_abi: &PublicationAbiUnit,
    service_unit_path: &str,
    dependency: &ServiceDependency,
) -> Result<(), InputAssemblyError> {
    if publication_abi.publication_id != dependency.id
        || publication_abi.version != dependency.version
    {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} publicationAbi coordinates {}@{} do not match dependency {}@{}",
                publication_abi.publication_id,
                publication_abi.version,
                dependency.id,
                dependency.version
            ),
        });
    }
    validate_publication_abi_identity(publication_abi).map_err(|error| {
        InputAssemblyError::Validation {
            message: format!("{service_unit_path} publicationAbi validation failed: {error}"),
        }
    })
}

fn read_json(path: &Path) -> Result<Value, InputAssemblyError> {
    let text = fs::read_to_string(path).map_err(|source| InputAssemblyError::Read {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| InputAssemblyError::Validation {
        message: format!("{} is invalid JSON: {source}", path.display()),
    })
}

fn invalid_pointer<T>(path: &Path, message: impl Into<String>) -> Result<T, InputAssemblyError> {
    Err(InputAssemblyError::Validation {
        message: format!("{} {}", path.display(), message.into()),
    })
}

#[cfg(test)]
#[path = "service_dependencies/tests.rs"]
mod tests;
