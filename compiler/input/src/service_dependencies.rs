use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};
use skiff_artifact_model::{
    PublicationAbiUnit, ServiceDependencyConstraint, ServiceUnit, SERVICE_UNIT_SCHEMA_VERSION,
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
    build_id_for_root: impl Fn(&Path, &ServiceUnit) -> Result<String, String>,
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
        let resolved = resolve_service_dependency(artifact_roots, dependency, &build_id_for_root)?;
        lock.push(service_dependency_lock_entry(dependency, &resolved));
        constraints.push(resolved);
    }
    Ok(ResolvedServiceDependencies::new(constraints, lock))
}

fn resolve_service_dependency(
    roots: &[PathBuf],
    dependency: &ServiceDependency,
    build_id_for_root: &impl Fn(&Path, &ServiceUnit) -> Result<String, String>,
) -> Result<ServiceDependencyConstraint, InputAssemblyError> {
    let service_path = service_artifact_path(dependency)?;
    let resolved = load_resolved_service_dependency_artifact(
        roots,
        dependency,
        &service_path,
        build_id_for_root,
    )?;

    Ok(ServiceDependencyConstraint {
        id: dependency.id.clone(),
        version: dependency.version.clone(),
        alias: dependency.alias.clone(),
        build_id: resolved.build_id,
        service_protocol_identity: resolved.service_protocol_identity,
        publication_abi: resolved.publication_abi,
    })
}

#[derive(Debug)]
struct ResolvedServiceArtifactPointer {
    root: PathBuf,
    pointer_path: PathBuf,
    service_unit_path: String,
}

#[derive(Debug)]
struct LoadedServiceDependencyArtifact {
    build_id: String,
    service_protocol_identity: String,
    publication_abi: PublicationAbiUnit,
}

fn load_resolved_service_dependency_artifact(
    roots: &[PathBuf],
    dependency: &ServiceDependency,
    service_path: &str,
    build_id_for_root: &impl Fn(&Path, &ServiceUnit) -> Result<String, String>,
) -> Result<LoadedServiceDependencyArtifact, InputAssemblyError> {
    let resolved = resolve_service_artifact_pointer(roots, dependency, service_path)?;
    let service_unit = read_json(&resolved.root.join(&resolved.service_unit_path))?;
    validate_service_unit(&service_unit, dependency, &resolved.service_unit_path)?;
    let typed_service_unit: ServiceUnit =
        serde_json::from_value(service_unit.clone()).map_err(|error| {
            InputAssemblyError::Validation {
                message: format!(
                    "{} service unit is invalid: {error}",
                    resolved.service_unit_path
                ),
            }
        })?;
    let build_id = build_id_for_root(&resolved.root, &typed_service_unit).map_err(|message| {
        InputAssemblyError::Validation {
            message: message.to_string(),
        }
    })?;
    let publication_abi =
        service_dependency_publication_abi(&service_unit, &resolved.service_unit_path)?;
    validate_service_dependency_publication_abi(
        &publication_abi,
        &resolved.service_unit_path,
        dependency,
    )?;
    Ok(LoadedServiceDependencyArtifact {
        build_id,
        service_protocol_identity: typed_service_unit.protocol_identity,
        publication_abi,
    })
}

fn resolve_service_artifact_pointer(
    roots: &[PathBuf],
    dependency: &ServiceDependency,
    service_path: &str,
) -> Result<ResolvedServiceArtifactPointer, InputAssemblyError> {
    let mut errors = Vec::new();
    for root in roots {
        match resolve_service_artifact_pointer_from_root(root, dependency, service_path)? {
            Some(resolved) => return Ok(resolved),
            None => errors.push(root.display().to_string()),
        }
    }
    Err(InputAssemblyError::Validation {
        message: format!(
            "service dependency {}@{} was not found under service artifact roots {}",
            dependency.id,
            dependency.version,
            errors.join(", ")
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
        let pointer = read_json(&dev_path)?;
        if pointer_matches_dependency(&pointer, dependency, true)? {
            let service_unit_path = service_unit_path_from_index(&pointer, &dev_path)?;
            return Ok(Some(ResolvedServiceArtifactPointer {
                root: root.to_path_buf(),
                pointer_path: dev_path,
                service_unit_path,
            }));
        }
    }

    let release_path = root
        .join("versions")
        .join("services")
        .join(service_path)
        .join(format!("{}.json", dependency.version));
    if release_path.is_file() {
        let version_pointer = read_json(&release_path)?;
        if !pointer_matches_dependency(&version_pointer, dependency, false)? {
            return Ok(None);
        }
        let build_id = required_string(
            &version_pointer,
            "buildId",
            &format!("{} buildId", release_path.display()),
        )?;
        let build_hash = identity_hash(&build_id, &format!("{} buildId", release_path.display()))?;
        let build_path = root
            .join("builds")
            .join("services")
            .join(service_path)
            .join(format!("{build_hash}.json"));
        let pointer = read_json(&build_path)?;
        if pointer_matches_dependency(&pointer, dependency, false)? {
            let service_unit_path = service_unit_path_from_index(&pointer, &build_path)?;
            return Ok(Some(ResolvedServiceArtifactPointer {
                root: root.to_path_buf(),
                pointer_path: build_path,
                service_unit_path,
            }));
        }
    }

    let legacy_index_dir = root.join("indexes").join("services").join(service_path);
    if legacy_index_dir.is_dir() {
        let mut matches = Vec::new();
        for entry in fs::read_dir(&legacy_index_dir).map_err(|source| InputAssemblyError::Read {
            path: legacy_index_dir.display().to_string(),
            source,
        })? {
            let entry = entry.map_err(|source| InputAssemblyError::Read {
                path: legacy_index_dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let pointer = read_json(&path)?;
            if pointer_matches_dependency(&pointer, dependency, true)? {
                let service_unit_path = service_unit_path_from_index(&pointer, &path)?;
                matches.push(ResolvedServiceArtifactPointer {
                    root: root.to_path_buf(),
                    pointer_path: path,
                    service_unit_path,
                });
            }
        }
        matches.sort_by(|left, right| left.pointer_path.cmp(&right.pointer_path));
        match matches.len() {
            0 => {}
            1 => return Ok(matches.pop()),
            _ => {
                return Err(InputAssemblyError::Validation {
                    message: format!(
                        "service dependency {}@{} resolved to multiple legacy artifact indexes under {}",
                        dependency.id,
                        dependency.version,
                        legacy_index_dir.display()
                    ),
                });
            }
        }
    }

    Ok(None)
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

fn pointer_matches_dependency(
    pointer: &Value,
    dependency: &ServiceDependency,
    allow_missing_version: bool,
) -> Result<bool, InputAssemblyError> {
    let service_id = pointer
        .get("serviceId")
        .or_else(|| pointer.get("service_id"))
        .and_then(Value::as_str);
    if service_id != Some(dependency.id.as_str()) {
        return Ok(false);
    }
    let version = pointer
        .get("serviceVersion")
        .or_else(|| pointer.get("service_version"))
        .or_else(|| pointer.get("version"))
        .and_then(Value::as_str);
    Ok(if allow_missing_version {
        version.is_none_or(|version| version == dependency.version)
    } else {
        version == Some(dependency.version.as_str())
    })
}

fn service_unit_path_from_index(
    index: &Value,
    index_path: &Path,
) -> Result<String, InputAssemblyError> {
    let service_unit = index
        .get("serviceUnit")
        .and_then(Value::as_object)
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{} is missing serviceUnit", index_path.display()),
        })?;
    required_object_string(
        service_unit,
        "unitPath",
        &format!("{} serviceUnit.unitPath", index_path.display()),
    )
}

fn validate_service_unit(
    service_unit: &Value,
    dependency: &ServiceDependency,
    service_unit_path: &str,
) -> Result<(), InputAssemblyError> {
    if service_unit.get("schemaVersion").and_then(Value::as_str)
        != Some(SERVICE_UNIT_SCHEMA_VERSION)
    {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} schemaVersion must be {SERVICE_UNIT_SCHEMA_VERSION}"
            ),
        });
    }
    let service_id = service_unit
        .pointer("/service/id")
        .and_then(Value::as_str)
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{service_unit_path} service.id is required"),
        })?;
    if service_id != dependency.id {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} service.id {service_id} does not match dependency {}",
                dependency.id
            ),
        });
    }
    let version = service_unit
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{service_unit_path} version is required"),
        })?;
    if version != dependency.version {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} version {version} does not match dependency {}@{}",
                dependency.id, dependency.version
            ),
        });
    }
    Ok(())
}

fn service_dependency_publication_abi(
    service_unit: &Value,
    service_unit_path: &str,
) -> Result<PublicationAbiUnit, InputAssemblyError> {
    let value = service_unit.get("publicationAbi").ok_or_else(|| {
        InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} missing publicationAbi; service dependencies require publication ABI"
            ),
        }
    })?;
    let publication_abi: PublicationAbiUnit =
        serde_json::from_value(value.clone()).map_err(|error| InputAssemblyError::Validation {
            message: format!("{service_unit_path} publicationAbi is invalid: {error}"),
        })?;
    if publication_abi.abi_identity.is_empty() {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} publicationAbi.abiIdentity must be a non-empty string"
            ),
        });
    }
    Ok(publication_abi)
}

fn validate_service_dependency_publication_abi(
    publication_abi: &PublicationAbiUnit,
    service_unit_path: &str,
    dependency: &ServiceDependency,
) -> Result<(), InputAssemblyError> {
    if publication_abi.publication_id != dependency.id {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} publicationAbi.publicationId {} does not match dependency {}",
                publication_abi.publication_id, dependency.id
            ),
        });
    }
    if publication_abi.version != dependency.version {
        return Err(InputAssemblyError::Validation {
            message: format!(
                "{service_unit_path} publicationAbi.version {} does not match dependency {}@{}",
                publication_abi.version, dependency.id, dependency.version
            ),
        });
    }
    let mut operation_exports = BTreeSet::new();
    for operation in &publication_abi.operation_exports {
        if operation.operation_abi_id.is_empty() {
            return Err(InputAssemblyError::Validation {
                message: format!(
                    "{service_unit_path} publicationAbi.operationExports contains empty operationAbiId"
                ),
            });
        }
        if !operation_exports.insert(operation.operation_abi_id.as_str()) {
            return Err(InputAssemblyError::Validation {
                message: format!(
                    "{service_unit_path} publicationAbi.operationExports duplicates operationAbiId {}",
                    operation.operation_abi_id
                ),
            });
        }
    }
    let mut operation_abi = BTreeSet::new();
    for operation in &publication_abi.operation_abi {
        let operation_abi_id = operation.operation.operation_abi_id.as_str();
        if operation_abi_id.is_empty() {
            return Err(InputAssemblyError::Validation {
                message: format!(
                    "{service_unit_path} publicationAbi.operationAbi contains empty operationAbiId"
                ),
            });
        }
        if !operation_exports.contains(operation_abi_id) {
            return Err(InputAssemblyError::Validation {
                message: format!(
                    "{service_unit_path} publicationAbi.operationAbi references non-exported operationAbiId {operation_abi_id}"
                ),
            });
        }
        if !operation_abi.insert(operation_abi_id) {
            return Err(InputAssemblyError::Validation {
                message: format!(
                    "{service_unit_path} publicationAbi.operationAbi duplicates operationAbiId {operation_abi_id}"
                ),
            });
        }
    }
    for public_instance in &publication_abi.public_instances {
        for operation in &public_instance.method_operations {
            if operation.operation_abi_id.is_empty() {
                return Err(InputAssemblyError::Validation {
                    message: format!(
                        "{service_unit_path} publicationAbi.publicInstances {} contains empty method operationAbiId",
                        public_instance.public_instance_key
                    ),
                });
            }
            if !operation_exports.contains(operation.operation_abi_id.as_str()) {
                return Err(InputAssemblyError::Validation {
                    message: format!(
                        "{service_unit_path} publicationAbi.publicInstances {} method operationAbiId {} is not exported",
                        public_instance.public_instance_key, operation.operation_abi_id
                    ),
                });
            }
        }
    }
    Ok(())
}

fn service_dependency_lock_entry(
    declared: &ServiceDependency,
    resolved: &ServiceDependencyConstraint,
) -> ServiceDependencyLockEntry {
    ServiceDependencyLockEntry::from_resolved_service(declared, resolved)
}

fn required_string(value: &Value, field: &str, label: &str) -> Result<String, InputAssemblyError> {
    value
        .get(field)
        .or_else(|| snake_case_field(value, field))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{label} must be a non-empty string"),
        })
}

fn required_object_string(
    object: &Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<String, InputAssemblyError> {
    object
        .get(field)
        .or_else(|| object.get(&camel_to_snake(field)))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| InputAssemblyError::Validation {
            message: format!("{label} must be a non-empty string"),
        })
}

fn snake_case_field<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
    value.as_object()?.get(&camel_to_snake(field))
}

fn camel_to_snake(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_ascii_uppercase() {
            result.push('_');
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

fn read_json(path: &Path) -> Result<Value, InputAssemblyError> {
    let text = fs::read_to_string(path).map_err(|source| InputAssemblyError::Read {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|error| InputAssemblyError::Validation {
        message: format!("failed to parse {}: {error}", path.display()),
    })
}

fn identity_hash(identity: &str, label: &str) -> Result<String, InputAssemblyError> {
    let Some((_, hash)) = identity.rsplit_once(":sha256:") else {
        return Err(InputAssemblyError::Validation {
            message: format!("{label} must include :sha256:"),
        });
    };
    if hash.len() != 64
        || !hash
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
    {
        return Err(InputAssemblyError::Validation {
            message: format!("{label} sha256 hash must be 64 lowercase hex characters"),
        });
    }
    Ok(hash.to_string())
}

#[cfg(test)]
mod tests {
    use super::service_dependency_publication_abi;

    #[test]
    fn service_dependency_missing_publication_abi_fails_closed() {
        let service_unit = serde_json::json!({
            "schemaVersion": skiff_artifact_model::SERVICE_UNIT_SCHEMA_VERSION,
            "service": { "id": "skiff.run/remotellm" },
            "version": "0.1.0",
            "protocolIdentity": "protocol:test",
            "operations": []
        });

        let error = service_dependency_publication_abi(&service_unit, "remoteLlm.service.json")
            .expect_err("missing publicationAbi must fail closed")
            .to_string();

        assert!(
            error.contains("missing publicationAbi"),
            "unexpected missing publicationAbi error: {error}"
        );
    }
}
