use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{json, Map, Value};
use skiff_artifact_model::ConfigShape;
use skiff_runtime_linked_program::{
    package_config_shape, LinkedProgramImage, PackageUnit, ServiceUnit,
};
#[cfg(test)]
use skiff_runtime_loader::ArtifactPointerFile;
use skiff_runtime_loader::{
    resolve_index_artifact_path, service_id_artifact_path, ArtifactIndexPointer,
    ArtifactRootRelativePath, RootedArtifactPointerFile,
};

use crate::{
    artifact_cache::RuntimeArtifactCaches,
    config_view::RuntimeConfigView,
    host::{DbProviderConfig, RuntimeServiceConfig},
};
use skiff_runtime_activation::RuntimeActivation;

use super::{
    assembly_identity::{
        validate_service_assembly_identity, validate_service_assembly_path_identity,
    },
    identity::identity_hash_with_label,
    program_loader::{LoadedRuntimeProgramParts, RuntimeProgramPartsLoader},
    service_http::parse_service_http_response_max_bytes,
    utils::read_json_file,
};

const LOCAL_DEV_CONFIG_PROFILE: &str = "dev";
const PACKAGE_TEST_ACTIVATION_ID_PREFIX: &str = "skiff-package-test-run-v1:";

#[cfg(test)]
pub(super) async fn load_services_from_artifact_pointers(
    artifact_root: &Path,
    base_runtime_id: &str,
    runtime_http_response_max_bytes: usize,
    pointer_files: Vec<ArtifactPointerFile>,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    let pointer_files = pointer_files
        .into_iter()
        .map(|pointer| RootedArtifactPointerFile::new(artifact_root.to_path_buf(), pointer))
        .collect();
    load_services_from_rooted_artifact_pointers(
        base_runtime_id,
        runtime_http_response_max_bytes,
        pointer_files,
    )
    .await
}

pub(super) async fn load_services_from_rooted_artifact_pointers(
    base_runtime_id: &str,
    runtime_http_response_max_bytes: usize,
    pointer_files: Vec<RootedArtifactPointerFile>,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    let artifact_caches = RuntimeArtifactCaches::new();
    load_services_from_rooted_artifact_pointers_with_caches(
        base_runtime_id,
        runtime_http_response_max_bytes,
        pointer_files,
        &artifact_caches,
        false,
    )
    .await
}

pub(super) async fn load_services_from_rooted_artifact_pointers_with_caches(
    base_runtime_id: &str,
    runtime_http_response_max_bytes: usize,
    pointer_files: Vec<RootedArtifactPointerFile>,
    artifact_caches: &RuntimeArtifactCaches,
    allow_missing_local_config: bool,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    let mut services = Vec::new();
    let mut seen_contracts = HashSet::new();

    for pointer_file in pointer_files {
        let artifact_root = pointer_file.artifact_root;
        let pointer_path = pointer_file.path;
        let entry = pointer_file.entry;
        let runtime_program_loader =
            RuntimeProgramPartsLoader::new(&artifact_root, artifact_caches);
        let route_key = format!("build:{}", entry.build_id);
        let contract_key = (entry.service_id.clone(), route_key.clone());
        if !seen_contracts.insert(contract_key) {
            anyhow::bail!(
                "duplicate artifact pointer for serviceId {} {}",
                entry.service_id,
                route_key
            );
        }

        let service_assembly_artifact_path =
            ArtifactRootRelativePath::new(&entry.service_assembly.path, "serviceAssembly")?;
        let service_assembly_path = resolve_index_artifact_path(
            &artifact_root,
            &service_assembly_artifact_path,
            "serviceAssembly",
        )?;
        let assembly = read_json_file(&service_assembly_path, "serviceAssembly")?;
        let artifact_identity = validate_service_assembly_identity(&entry, &assembly)?;
        validate_service_assembly_path_identity(
            &entry.service_assembly.path,
            &entry.service_id,
            &artifact_identity,
        )?;
        let short_sha = identity_hash_with_label(&artifact_identity, "serviceAssembly")?
            .get(..12)
            .ok_or_else(|| anyhow::anyhow!("serviceAssembly identity hash is unexpectedly short"))?
            .to_string();
        let artifact_identity_for_default = artifact_identity.clone();
        let implementation_identity = entry
            .implementation_identity
            .clone()
            .unwrap_or(artifact_identity_for_default);
        if implementation_identity.is_empty() {
            anyhow::bail!(
                "{} implementation identity must not be empty",
                pointer_path.display()
            );
        }

        let (service_unit, program_parts) = runtime_program_parts_for_pointer(
            &runtime_program_loader,
            &entry,
            &assembly,
            &service_assembly_path,
        )?;
        let revision_id = service_revision_id_from_assembly(&assembly, &service_assembly_path)?;
        validate_typed_service_metadata(&entry, &service_unit, &program_parts)?;
        let contract_identity = entry
            .contract_identity
            .clone()
            .unwrap_or_else(|| service_unit.protocol_identity.clone());
        if service_unit.protocol_identity != contract_identity {
            anyhow::bail!(
                "artifact pointer contractIdentity {} does not match service unit protocolIdentity {} for {}",
                contract_identity,
                service_unit.protocol_identity,
                service_assembly_path.display()
            );
        }
        let runtime_id = service_runtime_id(
            base_runtime_id,
            &program_parts.activation.service.id,
            &short_sha,
        );
        let selector_build_id = program_parts.identity.dynamic_build_id.clone();

        tracing::info!(
            event = "runtime.service_loaded",
            service_id = %program_parts.activation.service.id,
            revision_id = %revision_id,
            contract_identity = %contract_identity,
            pointer_build_id = %entry.build_id,
            build_id = %selector_build_id,
            runtime_id = %runtime_id,
            implementation_identity = %implementation_identity,
            artifact_identity = %artifact_identity,
            runtime_program = true,
            artifact = %service_assembly_path.display()
        );

        let (service_http_response_max_bytes, use_runtime_default_http_response_max_bytes) =
            parse_service_http_response_max_bytes(&assembly).map(|max_bytes| {
                (
                    max_bytes.unwrap_or(runtime_http_response_max_bytes),
                    max_bytes.is_none(),
                )
            })?;
        let config_shape = service_assembly_config_shape(&assembly, &service_assembly_path)?;
        let local_config = load_local_service_artifact_config(
            &artifact_root,
            &entry.service_id,
            &service_unit,
            &program_parts,
            config_shape,
            allow_missing_local_config,
        )?;

        services.push(RuntimeServiceConfig {
            runtime_program_identity: program_parts.identity.clone(),
            linked_image: program_parts.image.clone(),
            runtime_activation: program_parts.activation.clone(),
            http_response_max_bytes: service_http_response_max_bytes,
            use_runtime_default_http_response_max_bytes,
            runtime_id,
            revision_id,
            contract_identity,
            implementation_identity,
            artifact_identity,
            activation_identity: None,
            resolved_config_identity: None,
            config: local_config.service_config,
            package_configs: local_config.package_configs,
            service_db: local_config.service_db,
        });
    }

    artifact_caches.evict_lru_to_budget();
    Ok(services)
}

struct LocalServiceArtifactConfig {
    service_config: RuntimeConfigView,
    package_configs: Vec<RuntimeConfigView>,
    service_db: Option<DbProviderConfig>,
}

#[derive(Debug)]
pub(crate) struct PackageTestLocalConfig {
    pub(crate) service_config: RuntimeConfigView,
    pub(crate) package_configs: Vec<RuntimeConfigView>,
    pub(crate) service_db: Option<DbProviderConfig>,
}

fn load_local_service_artifact_config(
    artifact_root: &Path,
    service_id: &str,
    service_unit: &ServiceUnit,
    program_parts: &LoadedRuntimeProgramParts,
    config_shape: ConfigShape,
    allow_missing_local_config: bool,
) -> anyhow::Result<LocalServiceArtifactConfig> {
    let config_paths = local_service_config_paths(artifact_root, service_id)?;
    let Some(config_label_path) = config_paths.last() else {
        let service_config = if allow_missing_local_config {
            RuntimeConfigView::empty_unvalidated_with_shape(config_shape)
        } else {
            RuntimeConfigView::empty_with_shape(config_shape)?
        };
        let package_configs = if allow_missing_local_config {
            package_runtime_config_placeholder_views_from_program_parts(program_parts)?
        } else {
            package_runtime_config_views_from_program_parts(program_parts)?
        };
        return Ok(LocalServiceArtifactConfig {
            service_config,
            package_configs,
            service_db: None,
        });
    };

    let config = read_local_service_config_objects(&config_paths)?;
    let service_config = local_service_config_value(&config, config_label_path)?;
    let service_db = local_service_db_config(&service_config, config_label_path)?;
    let package_configs =
        apply_local_package_configs(&config, config_label_path, service_unit, program_parts)?;

    Ok(LocalServiceArtifactConfig {
        service_config: RuntimeConfigView::from_resolved_config(service_config, config_shape)?,
        package_configs,
        service_db,
    })
}

pub(crate) fn load_package_test_local_config(
    artifact_root: &Path,
    activation_id: &str,
    production_unit: &PackageUnit,
    synthetic_service_unit: &ServiceUnit,
    image: &LinkedProgramImage,
    activation: &RuntimeActivation,
    service_config_shape: ConfigShape,
) -> anyhow::Result<PackageTestLocalConfig> {
    let Some(config_path) = local_package_test_config_path(artifact_root, activation_id)? else {
        return Ok(PackageTestLocalConfig {
            service_config: RuntimeConfigView::empty_with_shape(service_config_shape).map_err(
                |error| {
                    anyhow::anyhow!(
                        "package-test activationId {} empty service config shape decode failed: {error}",
                        activation_id
                    )
                },
            )?,
            package_configs: package_runtime_config_views_from_parts(image, activation).map_err(
                |error| {
                    anyhow::anyhow!(
                        "package-test activationId {} package default config shape decode failed: {error}",
                        activation_id
                    )
                },
            )?,
            service_db: None,
        });
    };

    let config = read_local_config_object(&config_path, "local package-test config")?;
    let service_config = local_package_test_service_config_value(&config, &config_path)?;
    let service_db = local_top_level_service_db_config(&config, &config_path)?;
    let package_configs = apply_package_test_package_configs(
        &config,
        &config_path,
        production_unit,
        synthetic_service_unit,
        image,
        activation,
    )?;

    let service_config =
        RuntimeConfigView::from_resolved_config(service_config, service_config_shape).map_err(
            |error| {
                anyhow::anyhow!(
                    "local package-test config {} service config shape decode failed: {error}",
                    config_path.display()
                )
            },
        )?;

    Ok(PackageTestLocalConfig {
        service_config,
        package_configs,
        service_db,
    })
}

pub(crate) fn validate_package_test_activation_id(value: &str) -> anyhow::Result<()> {
    let Some(rest) = value.strip_prefix(PACKAGE_TEST_ACTIVATION_ID_PREFIX) else {
        anyhow::bail!(
            "package-test activationId must start with {PACKAGE_TEST_ACTIVATION_ID_PREFIX}, got {value}"
        );
    };
    if rest.is_empty() {
        anyhow::bail!("package-test activationId must not have an empty run suffix");
    }
    if rest.contains("..") {
        anyhow::bail!("package-test activationId must not contain .., got {value}");
    }
    if rest
        .bytes()
        .any(|byte| matches!(byte, b'/' | b'\\' | b'%') || byte.is_ascii_whitespace())
    {
        anyhow::bail!(
            "package-test activationId suffix must be a single URL/path safe segment, got {value}"
        );
    }
    if !rest.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'~' | b'-')
    }) {
        anyhow::bail!("package-test activationId suffix must match [A-Za-z0-9._:~-]+, got {value}");
    }
    Ok(())
}

fn local_package_test_config_path(
    artifact_root: &Path,
    activation_id: &str,
) -> anyhow::Result<Option<PathBuf>> {
    validate_package_test_activation_id(activation_id)?;
    let relative_path = ArtifactRootRelativePath::new(
        Path::new("configs")
            .join("package-tests")
            .join(activation_id)
            .join("config.yml"),
        "local package-test config",
    )?;
    let path = artifact_root.join(relative_path.as_path());
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(resolve_index_artifact_path(
        artifact_root,
        &relative_path,
        "local package-test config",
    )?))
}

fn local_service_config_paths(
    artifact_root: &Path,
    service_id: &str,
) -> anyhow::Result<Vec<PathBuf>> {
    let service_config_dir = Path::new("configs")
        .join("services")
        .join(service_id_artifact_path(service_id)?);
    let mut paths = Vec::new();
    push_local_service_config_path_if_present(
        artifact_root,
        &service_config_dir,
        "config.yml",
        &mut paths,
    )?;
    push_local_service_config_path_if_present(
        artifact_root,
        &service_config_dir,
        &format!("config.{LOCAL_DEV_CONFIG_PROFILE}.yml"),
        &mut paths,
    )?;
    push_local_service_config_path_if_present(
        artifact_root,
        &service_config_dir,
        &format!("config.{LOCAL_DEV_CONFIG_PROFILE}.secret.yml"),
        &mut paths,
    )?;
    Ok(paths)
}

fn push_local_service_config_path_if_present(
    artifact_root: &Path,
    service_config_dir: &Path,
    file_name: &str,
    paths: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    let relative_path =
        ArtifactRootRelativePath::new(service_config_dir.join(file_name), "local service config")?;
    let path = artifact_root.join(relative_path.as_path());
    if !path.is_file() {
        return Ok(());
    }
    paths.push(resolve_index_artifact_path(
        artifact_root,
        &relative_path,
        "local service config",
    )?);
    Ok(())
}

fn read_local_service_config_object(path: &Path) -> anyhow::Result<Map<String, Value>> {
    read_local_config_object(path, "local service config")
}

fn read_local_config_object(path: &Path, config_label: &str) -> anyhow::Result<Map<String, Value>> {
    let text = fs::read_to_string(path).map_err(|error| {
        anyhow::anyhow!("failed to read {config_label} {}: {error}", path.display())
    })?;
    let value: Value = serde_yaml::from_str(&text).map_err(|error| {
        anyhow::anyhow!(
            "failed to parse {config_label} {} as YAML: {error}",
            path.display()
        )
    })?;
    match value {
        Value::Object(object) => Ok(object),
        _ => anyhow::bail!("{config_label} {} must be a YAML object", path.display()),
    }
}

fn read_local_service_config_objects(paths: &[PathBuf]) -> anyhow::Result<Map<String, Value>> {
    let mut merged = Map::new();
    for path in paths {
        overlay_config_map(&mut merged, read_local_service_config_object(path)?);
    }
    Ok(merged)
}

fn local_service_config_value(config: &Map<String, Value>, path: &Path) -> anyhow::Result<Value> {
    match config.get("service") {
        Some(Value::Object(object)) => Ok(Value::Object(object.clone())),
        Some(_) => anyhow::bail!(
            "local service config {} field service must be a JSON object",
            path.display()
        ),
        None => Ok(Value::Object(Map::new())),
    }
}

fn local_package_test_service_config_value(
    config: &Map<String, Value>,
    path: &Path,
) -> anyhow::Result<Value> {
    match config.get("service") {
        Some(Value::Object(object)) => Ok(Value::Object(object.clone())),
        Some(_) => anyhow::bail!(
            "local package-test config {} field service must be a JSON object",
            path.display()
        ),
        None => Ok(Value::Object(Map::new())),
    }
}

fn local_service_db_config(
    service_config: &Value,
    path: &Path,
) -> anyhow::Result<Option<DbProviderConfig>> {
    let Some(service_db) = service_config.get("serviceDb") else {
        return Ok(None);
    };
    service_db_config_from_value(
        service_db,
        path,
        "local service config",
        "service.serviceDb",
    )
}

fn local_top_level_service_db_config(
    config: &Map<String, Value>,
    path: &Path,
) -> anyhow::Result<Option<DbProviderConfig>> {
    let Some(service_db) = config.get("serviceDb") else {
        return Ok(None);
    };
    service_db_config_from_value(service_db, path, "local package-test config", "serviceDb")
}

fn service_db_config_from_value(
    service_db: &Value,
    path: &Path,
    config_label: &str,
    field: &str,
) -> anyhow::Result<Option<DbProviderConfig>> {
    if service_db.is_null() {
        return Ok(None);
    }
    let Some(object) = service_db.as_object() else {
        anyhow::bail!(
            "{config_label} {} field {field} must be a JSON object",
            path.display(),
        );
    };
    match object.get("mongoUrl") {
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(DbProviderConfig::opaque(json!({ "mongoUrl": value }))))
        }
        Some(Value::String(_)) => anyhow::bail!(
            "{config_label} {} field {field}.mongoUrl must be a non-empty string",
            path.display(),
        ),
        Some(_) => anyhow::bail!(
            "{config_label} {} field {field}.mongoUrl must be a string",
            path.display(),
        ),
        None => anyhow::bail!(
            "{config_label} {} field {field}.mongoUrl is required when serviceDb is present",
            path.display(),
        ),
    }
}

fn apply_local_package_configs(
    config: &Map<String, Value>,
    path: &Path,
    service_unit: &ServiceUnit,
    program_parts: &LoadedRuntimeProgramParts,
) -> anyhow::Result<Vec<RuntimeConfigView>> {
    let mut package_config_values = package_config_values_from_program_parts(program_parts);
    let Some(packages) = config.get("packages") else {
        return package_config_views_from_values_for_program_parts(
            program_parts,
            package_config_values,
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "local package-test config {} package config shape decode failed: {error}",
                path.display()
            )
        });
    };
    let packages = packages.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "local service config {} field packages must be a JSON object",
            path.display()
        )
    })?;
    if packages.is_empty() {
        return package_config_views_from_values_for_program_parts(
            program_parts,
            package_config_values,
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "local package-test config {} package config shape decode failed: {error}",
                path.display()
            )
        });
    }

    let slots_by_alias = service_package_slots_by_alias(service_unit, &program_parts.image, path)?;
    let mut seen_slots = HashSet::new();
    for (alias, overlay) in packages {
        let slot = *slots_by_alias.get(alias).ok_or_else(|| {
            anyhow::anyhow!(
                "local service config {} packages.{} does not match a service package dependency alias",
                path.display(),
                alias
            )
        })?;
        if !seen_slots.insert(slot) {
            anyhow::bail!(
                "local service config {} declares multiple package configs for package slot {}",
                path.display(),
                slot
            );
        }
        if !overlay.is_object() {
            anyhow::bail!(
                "local service config {} packages.{} must be a JSON object",
                path.display(),
                alias
            );
        }
        let base_config = package_config_values
            .get(slot)
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let resolved_config = merge_config_object_overlay(
            base_config,
            overlay.clone(),
            &format!("local service config {} packages.{}", path.display(), alias),
        )?;
        if slot >= package_config_values.len() {
            package_config_values.resize_with(slot + 1, || Value::Object(Map::new()));
        }
        package_config_values[slot] = resolved_config;
    }

    package_config_views_from_values_for_program_parts(program_parts, package_config_values)
        .map_err(|error| {
            anyhow::anyhow!(
                "local package-test config {} package config shape decode failed: {error}",
                path.display()
            )
        })
}

fn apply_package_test_package_configs(
    config: &Map<String, Value>,
    path: &Path,
    production_unit: &PackageUnit,
    synthetic_service_unit: &ServiceUnit,
    image: &LinkedProgramImage,
    activation: &RuntimeActivation,
) -> anyhow::Result<Vec<RuntimeConfigView>> {
    let mut package_config_values = package_config_values_from_parts(image, activation);
    let Some(packages) = config.get("packages") else {
        return package_config_views_from_values_for_image(image, package_config_values);
    };
    let packages = packages.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "local package-test config {} field packages must be a JSON object",
            path.display()
        )
    })?;
    if packages.is_empty() {
        return package_config_views_from_values_for_image(image, package_config_values);
    }

    let slots_by_alias =
        package_test_package_slots_by_alias(production_unit, synthetic_service_unit, image, path)?;
    let mut seen_slots = HashMap::<usize, String>::new();
    for (alias, overlay) in packages {
        if alias == &production_unit.package_id || alias == &synthetic_service_unit.service.id {
            anyhow::bail!(
                "local package-test config {} packages.{} targets the package under test; use service config instead",
                path.display(),
                alias
            );
        }
        let slot = *slots_by_alias.get(alias).ok_or_else(|| {
            anyhow::anyhow!(
                "local package-test config {} packages.{} does not match a package under test dependency alias",
                path.display(),
                alias
            )
        })?;
        if let Some(first_alias) = seen_slots.insert(slot, alias.clone()) {
            anyhow::bail!(
                "local package-test config {} declares multiple package configs for package slot {} via aliases {} and {}",
                path.display(),
                slot,
                first_alias,
                alias
            );
        }
        if !overlay.is_object() {
            anyhow::bail!(
                "local package-test config {} packages.{} must be a JSON object",
                path.display(),
                alias
            );
        }
        let base_config = package_config_values
            .get(slot)
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let resolved_config = merge_config_object_overlay(
            base_config,
            overlay.clone(),
            &format!(
                "local package-test config {} packages.{}",
                path.display(),
                alias
            ),
        )?;
        if slot >= package_config_values.len() {
            package_config_values.resize_with(slot + 1, || Value::Object(Map::new()));
        }
        package_config_values[slot] = resolved_config;
    }

    package_config_views_from_values_for_image(image, package_config_values)
}

fn package_test_package_slots_by_alias(
    production_unit: &PackageUnit,
    synthetic_service_unit: &ServiceUnit,
    image: &LinkedProgramImage,
    path: &Path,
) -> anyhow::Result<HashMap<String, usize>> {
    let mut slots = HashMap::new();
    for dependency in &production_unit.dependencies {
        if dependency.alias.trim().is_empty() {
            anyhow::bail!(
                "local package-test config {} production package dependency {} declares an empty alias",
                path.display(),
                dependency.id
            );
        }
        if dependency.id == production_unit.package_id
            || dependency.alias == production_unit.package_id
            || dependency.alias == synthetic_service_unit.service.id
        {
            anyhow::bail!(
                "local package-test config {} production package dependency alias {} targets the package under test",
                path.display(),
                dependency.alias
            );
        }
        let slot = image
            .link_overlay
            .package_slot_for_dependency_ref(&dependency.alias)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "local package-test config {} package dependency alias {} package {} is not linked",
                    path.display(),
                    dependency.alias,
                    dependency.id
                )
            })?;
        let package = image.packages.get(slot).ok_or_else(|| {
            anyhow::anyhow!(
                "local package-test config {} package dependency alias {} resolved to missing package slot {}",
                path.display(),
                dependency.alias,
                slot
            )
        })?;
        if package.package_id != dependency.id || package.version != dependency.version {
            anyhow::bail!(
                "local package-test config {} package dependency alias {} resolved to {}@{} but production dependency declares {}@{}",
                path.display(),
                dependency.alias,
                package.package_id,
                package.version,
                dependency.id,
                dependency.version
            );
        }
        if slots.insert(dependency.alias.clone(), slot).is_some() {
            anyhow::bail!(
                "local package-test config {} production package unit declares duplicate package dependency alias {}",
                path.display(),
                dependency.alias
            );
        }
    }
    Ok(slots)
}

fn service_package_slots_by_alias(
    service_unit: &ServiceUnit,
    image: &LinkedProgramImage,
    path: &Path,
) -> anyhow::Result<HashMap<String, usize>> {
    let mut slots = HashMap::new();
    for dependency in &service_unit.package_dependencies {
        if dependency.alias.trim().is_empty() {
            continue;
        }
        let slot = image
            .link_overlay
            .package_slot_for_dependency_ref(&dependency.alias)
            .or_else(|| {
                image
                    .packages
                    .iter()
                    .position(|package| package.package_id == dependency.id)
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "local service config {} package dependency alias {} package {} is not loaded",
                    path.display(),
                    dependency.alias,
                    dependency.id
                )
            })?;
        if slots.insert(dependency.alias.clone(), slot).is_some() {
            anyhow::bail!(
                "local service config {} service unit declares duplicate package dependency alias {}",
                path.display(),
                dependency.alias
            );
        }
    }
    Ok(slots)
}

fn merge_config_object_overlay(base: Value, overlay: Value, label: &str) -> anyhow::Result<Value> {
    let mut base = match base {
        Value::Null => Map::new(),
        Value::Object(object) => object,
        _ => anyhow::bail!("{label} artifact default must be a JSON object"),
    };
    let overlay = match overlay {
        Value::Null => Map::new(),
        Value::Object(object) => object,
        _ => anyhow::bail!("{label} must be a JSON object"),
    };
    overlay_config_map(&mut base, overlay);
    Ok(Value::Object(base))
}

fn overlay_config_map(target: &mut Map<String, Value>, overlay: Map<String, Value>) {
    for (key, value) in overlay {
        if value.is_null() {
            target.remove(&key);
            continue;
        }
        if let Value::Object(overlay_object) = value {
            if let Some(Value::Object(target_object)) = target.get_mut(&key) {
                overlay_config_map(target_object, overlay_object);
            } else {
                target.insert(key, Value::Object(overlay_object));
            }
            continue;
        }
        target.insert(key, value);
    }
}

fn package_runtime_config_views_from_program_parts(
    program_parts: &LoadedRuntimeProgramParts,
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    package_runtime_config_views_from_parts(
        program_parts.image.as_ref(),
        program_parts.activation.as_ref(),
    )
}

fn package_runtime_config_views_from_parts(
    image: &LinkedProgramImage,
    activation: &RuntimeActivation,
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    package_config_views_from_values_for_image(
        image,
        package_config_values_from_parts(image, activation),
    )
}

fn package_config_views_from_values_for_program_parts(
    program_parts: &LoadedRuntimeProgramParts,
    values: Vec<Value>,
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    package_config_views_from_values_for_image(program_parts.image.as_ref(), values)
}

fn package_runtime_config_placeholder_views_from_program_parts(
    program_parts: &LoadedRuntimeProgramParts,
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    (0..program_parts.image.packages.len())
        .map(|slot| {
            let config_shape = package_config_shape_from_image(program_parts.image.as_ref(), slot)?;
            Ok(crate::config_view::RuntimeConfigView::empty_unvalidated_with_shape(config_shape))
        })
        .collect()
}

fn package_config_views_from_values_for_image(
    image: &LinkedProgramImage,
    values: Vec<Value>,
) -> anyhow::Result<Vec<crate::config_view::RuntimeConfigView>> {
    values
        .into_iter()
        .enumerate()
        .map(|(slot, config)| {
            let config_shape = package_config_shape_from_image(image, slot)?;
            crate::config_view::RuntimeConfigView::from_resolved_config(config, config_shape)
        })
        .collect()
}

fn package_config_values_from_program_parts(
    program_parts: &LoadedRuntimeProgramParts,
) -> Vec<Value> {
    package_config_values_from_parts(
        program_parts.image.as_ref(),
        program_parts.activation.as_ref(),
    )
}

fn package_config_values_from_parts(
    image: &LinkedProgramImage,
    activation: &RuntimeActivation,
) -> Vec<Value> {
    let mut values = activation.package_configs.clone();
    values.resize_with(image.packages.len(), || Value::Object(Map::new()));
    for value in &mut values {
        if value.is_null() {
            *value = Value::Object(Map::new());
        }
    }
    values
}

fn package_config_shape_from_image(
    image: &LinkedProgramImage,
    slot: usize,
) -> anyhow::Result<ConfigShape> {
    image
        .packages
        .get(slot)
        .map(|package| package_config_shape(package.as_ref()))
        .transpose()
        .map(|shape| shape.unwrap_or_else(ConfigShape::empty))
}

fn service_assembly_config_shape(
    assembly: &Value,
    service_assembly_path: &Path,
) -> anyhow::Result<ConfigShape> {
    let value = assembly.get("configShape").ok_or_else(|| {
        anyhow::anyhow!(
            "{} does not declare configShape",
            service_assembly_path.display()
        )
    })?;
    let shape: ConfigShape = serde_json::from_value(value.clone()).map_err(|error| {
        anyhow::anyhow!(
            "{} configShape must be a config shape object: {error}",
            service_assembly_path.display()
        )
    })?;
    shape.validate_schema_version()?;
    Ok(shape)
}

fn runtime_program_parts_for_pointer(
    loader: &RuntimeProgramPartsLoader<'_>,
    entry: &ArtifactIndexPointer,
    assembly: &Value,
    service_assembly_path: &Path,
) -> anyhow::Result<(Arc<ServiceUnit>, LoadedRuntimeProgramParts)> {
    if entry.service_unit_path.is_none() && !service_assembly_declares_service_unit(assembly) {
        anyhow::bail!(
            "{} does not declare canonical serviceUnit.unitPath; production runtime loading requires typed ServiceUnit",
            service_assembly_path.display()
        );
    }
    let loaded = loader
        .load_pointer_parts_with_service_unit(entry)
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to load runtime program parts for {} via serviceUnit: {error:#}",
                service_assembly_path.display()
            )
        })?;
    Ok((loaded.service_unit, loaded.parts))
}

fn service_assembly_declares_service_unit(assembly: &Value) -> bool {
    assembly.get("serviceUnit").is_some()
}

fn validate_typed_service_metadata(
    entry: &ArtifactIndexPointer,
    service_unit: &ServiceUnit,
    program_parts: &LoadedRuntimeProgramParts,
) -> anyhow::Result<()> {
    if service_unit.service.id.is_empty() {
        anyhow::bail!("service unit service id is required");
    }
    if service_unit.service.id != entry.service_id {
        anyhow::bail!(
            "artifact pointer serviceId {} does not match service unit service id {}",
            entry.service_id,
            service_unit.service.id
        );
    }
    if program_parts.activation.service.id != service_unit.service.id {
        anyhow::bail!(
            "runtime program service id {} does not match service unit service id {}",
            program_parts.activation.service.id,
            service_unit.service.id
        );
    }
    if service_unit.protocol_identity.is_empty() {
        anyhow::bail!("service unit protocolIdentity is required");
    }
    if !is_protocol_identity(&service_unit.protocol_identity) {
        anyhow::bail!(
            "service unit protocolIdentity must be skiff-protocol-v1:sha256:<64 lowercase hex>"
        );
    }
    Ok(())
}

fn service_revision_id_from_assembly(assembly: &Value, path: &Path) -> anyhow::Result<String> {
    let revision_id = assembly
        .pointer("/service/revisionId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} serviceAssembly.service.revisionId is required",
                path.display()
            )
        })?;
    if !is_bare_sha256(revision_id) {
        anyhow::bail!(
            "{} serviceAssembly.service.revisionId must be <64 lowercase hex>",
            path.display()
        );
    }
    Ok(revision_id.to_string())
}

fn is_bare_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_protocol_identity(value: &str) -> bool {
    let Some((prefix, hash)) = value.rsplit_once(":sha256:") else {
        return false;
    };
    prefix == "skiff-protocol-v1"
        && hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn service_runtime_id(
    base_runtime_id: &str,
    service_id: &str,
    short_sha: &str,
) -> String {
    format!("{base_runtime_id}:svc:{service_id}:artifact:{short_sha}")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::{json, Value};
    use skiff_artifact_identity::{file_ir_identity, package_abi_identity, package_build_identity};
    use skiff_artifact_model::{MetadataValue, PackageDependencyConstraint};
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        host::{RouterWriterMessage, RuntimeConfig, RuntimeHost},
        loader::{
            assembly_identity::service_assembly_hash_input, identity::identity_hash_with_label,
            load_services_from_artifact_roots_with_default, value_sha256, ArtifactLoadOptions,
            SERVICE_ASSEMBLY_IDENTITY_PREFIX, SERVICE_BUILD_IDENTITY_PREFIX,
        },
        program::RuntimeProgramLayers,
    };
    use skiff_runtime_boundary::type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt};
    use skiff_runtime_loader::ServiceAssemblyPointer;
    use skiff_runtime_request::RequestEnvelope;
    use skiff_runtime_transport::protocol::{
        decode_typed_binary_frame, RuntimeRegisterFrameHeader,
    };

    const PROTOCOL_IDENTITY: &str =
        "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SERVICE_REVISION_ID: &str =
        "875894aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const POINTER_BUILD_ID: &str = "skiff-service-build-v1:sha256:5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5cd5";
    const SERVICE_RUN_OPERATION_ABI_ID: &str = "operation:skiff.run/account:v1:svc.main.run";

    fn service_db_mongo_url(service_db: &DbProviderConfig) -> Option<&str> {
        service_db
            .as_value()
            .pointer("/mongoUrl")
            .and_then(Value::as_str)
    }

    fn expect_binary_router_message(message: RouterWriterMessage) -> Vec<u8> {
        match message {
            RouterWriterMessage::Binary(frame) => frame,
            other => panic!("expected binary router writer message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn artifact_runtime_config_registers_service_assembly_revision_id() {
        let temp = TempDir::new("runtime-config-assembly-revision");
        let root = temp.path().join("artifacts");
        fs::create_dir_all(&root).expect("artifact root should be created");

        write_file_ir(&root, "units/files/service.json");
        write_service_unit(&root, "units/services/account.json");
        let assembly_identity = write_service_assembly(&root, "local/assembly.json");

        let services = load_services_from_artifact_pointers(
            &root,
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            vec![ArtifactPointerFile {
                path: root.join("build-record.json"),
                entry: ArtifactIndexPointer {
                    service_id: "skiff.run/account".to_string(),
                    service_version: Some("v1".to_string()),
                    build_id: POINTER_BUILD_ID.to_string(),
                    contract_identity: Some(PROTOCOL_IDENTITY.to_string()),
                    implementation_identity: None,
                    service_unit_path: None,
                    service_assembly: ServiceAssemblyPointer {
                        path: PathBuf::from("local/assembly.json"),
                        assembly_identity: Some(assembly_identity),
                    },
                },
            }],
        )
        .await
        .expect("artifact runtime config should load");

        assert_eq!(services.len(), 1);
        let service = services.into_iter().next().expect("service should exist");
        assert_eq!(service.revision_id, SERVICE_REVISION_ID);
        assert_ne!(
            service.runtime_program_identity.dynamic_build_id,
            service.revision_id
        );
        assert!(service
            .runtime_program_identity
            .dynamic_build_id
            .starts_with("skiff-service-build-v1:sha256:"));

        let host = RuntimeHost::new(RuntimeConfig {
            db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
            services: vec![service.clone()],
            router_url: "ws://127.0.0.1:4001/runtime".to_string(),
            base_runtime_id: "runtime-base".to_string(),
            runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
            artifact_roots: Vec::new(),
            http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            http_egress_proxy: None,
        })
        .expect("runtime host should accept artifact service config");
        let (sender, mut receiver) = mpsc::unbounded_channel::<RouterWriterMessage>();
        host.queue_registers(sender)
            .expect("runtime register should serialize");
        let _capabilities_frame = expect_binary_router_message(
            receiver
                .recv()
                .await
                .expect("runtime capabilities frame should be queued"),
        );
        let frame = expect_binary_router_message(
            receiver
                .recv()
                .await
                .expect("register frame should be queued"),
        );
        let (register, payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("register frame should decode");

        assert!(payload.is_empty());
        assert_eq!(register.revision_id, SERVICE_REVISION_ID);
        assert_eq!(
            register.build_id,
            service.runtime_program_identity.dynamic_build_id
        );
        assert!(is_bare_sha256(&register.revision_id));
    }

    #[tokio::test]
    async fn dev_reload_later_artifact_root_overrides_same_service_pointer() {
        let temp = TempDir::new("runtime-config-multi-root-dev-override");
        let default_root = temp.path().join("default-artifacts");
        let override_root = temp.path().join("override-artifacts");

        write_file_ir(&default_root, "units/files/service.json");
        write_service_unit(&default_root, "units/services/account.json");
        let default_assembly_identity = write_service_assembly_with_revision(
            &default_root,
            "local/assembly.json",
            SERVICE_REVISION_ID,
        );
        write_dev_pointer(&default_root, &default_assembly_identity);

        write_file_ir(&override_root, "units/files/service.json");
        write_service_unit(&override_root, "units/services/account.json");
        let override_revision_id =
            "975894aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let override_assembly_identity = write_service_assembly_with_revision(
            &override_root,
            "local/assembly.json",
            override_revision_id,
        );
        write_dev_pointer(&override_root, &override_assembly_identity);

        let services = load_services_from_artifact_roots_with_default(
            &[default_root, override_root],
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            &ArtifactLoadOptions::dev_reload(),
        )
        .await
        .expect("multi-root dev reload should load");

        assert_eq!(services.len(), 1);
        assert_eq!(services[0].revision_id, override_revision_id);
        assert_eq!(services[0].artifact_identity, override_assembly_identity);
    }

    #[tokio::test]
    async fn runtime_host_lazy_loads_service_from_configured_artifact_roots() {
        let temp = TempDir::new("runtime-config-lazy-load");
        let root = temp.path().join("artifacts");
        write_file_ir(&root, "units/files/service.json");
        write_service_unit(&root, "units/services/account.json");
        let assembly_identity = write_service_assembly(&root, "local/assembly.json");
        write_release_pointer(&root, &assembly_identity);

        let services = load_services_from_artifact_roots_with_default(
            std::slice::from_ref(&root),
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            &ArtifactLoadOptions::release(),
        )
        .await
        .expect("fixture service should load");
        let service = services.first().expect("fixture service should exist");
        let build_id = service.runtime_program_identity.dynamic_build_id.clone();
        let target = service
            .linked_image
            .routes
            .keys()
            .next()
            .expect("fixture service should expose a route")
            .clone();

        let host = RuntimeHost::new(RuntimeConfig {
            db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
            services: Vec::new(),
            router_url: "ws://127.0.0.1:4001/runtime".to_string(),
            base_runtime_id: "runtime-base".to_string(),
            runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
            artifact_roots: vec![root],
            http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            http_egress_proxy: None,
        })
        .expect("runtime host should build with artifact roots");
        let (sender, mut receiver) = mpsc::unbounded_channel::<RouterWriterMessage>();
        let request = RequestEnvelope {
            request_id: "lazy-request-1".to_string(),
            mode: "unary".to_string(),
            target,
            operation_abi_id: Some(SERVICE_RUN_OPERATION_ABI_ID.to_string()),
            selector: Some(format!("operation:{SERVICE_RUN_OPERATION_ABI_ID}")),
            service_id: Some("skiff.run/account".to_string()),
            build_id: build_id.clone(),
            service_protocol_identity: PROTOCOL_IDENTITY.to_string(),
            contract_identity: None,
            activation_identity: None,
            http_adapter: None,
            websocket_adapter: None,
            binary_http: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        };

        let _operation = host
            .lookup_or_load_operation(&request, sender)
            .await
            .expect("runtime should lazy load service for request");

        let frame = expect_binary_router_message(
            receiver
                .recv()
                .await
                .expect("lazy load should queue runtime.register"),
        );
        let (register, payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("register frame should decode");
        assert!(payload.is_empty());
        assert_eq!(register.service_id, "skiff.run/account");
        assert_eq!(register.build_id, build_id);
    }

    #[tokio::test]
    async fn artifact_runtime_config_loads_local_config_file() {
        let temp = TempDir::new("runtime-config-local-file");
        let root = temp.path().join("artifacts");
        fs::create_dir_all(&root).expect("artifact root should be created");

        write_file_ir(&root, "units/files/service.json");
        write_service_unit_with_package_dependency(&root, "units/services/account.json");
        write_package_index(&root, "skiff.run/pkg", "v1", "units/packages/pkg.json");
        write_package_unit(&root, "units/packages/pkg.json");
        let assembly_identity = write_service_assembly(&root, "local/assembly.json");
        let mongo_url = "mongodb://127.0.0.1:27017/skiff-runtime-local";
        write_yaml(
            &root,
            "configs/services/skiff~run~~account/config.yml",
            &json!({
                "service": {
                    "feature": { "enabled": true },
                    "serviceDb": { "mongoUrl": mongo_url }
                },
                "packages": {
                    "pkg": {
                        "defaults": {
                            "local": "local"
                        },
                        "localOnly": 42
                    }
                }
            }),
        );

        let services = load_services_from_artifact_pointers(
            &root,
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            vec![ArtifactPointerFile {
                path: root.join("build-record.json"),
                entry: ArtifactIndexPointer {
                    service_id: "skiff.run/account".to_string(),
                    service_version: Some("v1".to_string()),
                    build_id: POINTER_BUILD_ID.to_string(),
                    contract_identity: Some(PROTOCOL_IDENTITY.to_string()),
                    implementation_identity: None,
                    service_unit_path: None,
                    service_assembly: ServiceAssemblyPointer {
                        path: PathBuf::from("local/assembly.json"),
                        assembly_identity: Some(assembly_identity),
                    },
                },
            }],
        )
        .await
        .expect("artifact runtime config should load local config");

        let service = services.first().expect("service should load");
        assert_eq!(
            service
                .config
                .resolved_config_value()
                .pointer("/feature/enabled"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            service.service_db.as_ref().and_then(service_db_mongo_url),
            Some(mongo_url)
        );
        let package_config = service
            .package_configs
            .first()
            .expect("package config should exist")
            .resolved_config_value();
        assert_eq!(
            package_config.pointer("/defaults/fromArtifact"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            package_config.pointer("/defaults/local"),
            Some(&Value::String("local".to_string()))
        );
        assert_eq!(
            package_config.pointer("/artifactOnly"),
            Some(&Value::String("kept".to_string()))
        );
        assert_eq!(package_config.pointer("/localOnly"), Some(&json!(42)));
    }

    #[tokio::test]
    async fn artifact_runtime_config_loads_local_dev_profile_config_file() {
        let temp = TempDir::new("runtime-config-local-dev-file");
        let root = temp.path().join("artifacts");
        fs::create_dir_all(&root).expect("artifact root should be created");

        write_file_ir(&root, "units/files/service.json");
        write_service_unit_with_package_dependency(&root, "units/services/account.json");
        write_package_index(&root, "skiff.run/pkg", "v1", "units/packages/pkg.json");
        write_package_unit_with_config_shape(
            &root,
            "units/packages/pkg.json",
            json!({
                "schemaVersion": "skiff-config-shape-v1",
                "entries": [
                    { "path": "cookieName", "type": "string", "required": true },
                    { "path": "maxAgeSeconds", "type": "number", "required": true }
                ]
            }),
        );
        let assembly_identity = write_service_assembly(&root, "local/assembly.json");
        write_yaml(
            &root,
            "configs/services/skiff~run~~account/config.yml",
            &json!({
                "service": {
                    "feature": { "enabled": false },
                    "source": "base"
                },
                "packages": {
                    "pkg": {
                        "defaults": {
                            "local": "base"
                        },
                        "cookieName": "base_session"
                    }
                }
            }),
        );
        write_yaml(
            &root,
            "configs/services/skiff~run~~account/config.dev.yml",
            &json!({
                "service": {
                    "feature": { "enabled": true },
                    "source": "dev"
                },
                "packages": {
                    "pkg": {
                        "defaults": {
                            "local": "dev"
                        },
                        "cookieName": "skiff_session",
                        "maxAgeSeconds": 60,
                        "devOnly": true
                    }
                }
            }),
        );
        write_yaml(
            &root,
            "configs/services/skiff~run~~account/config.dev.secret.yml",
            &json!({
                "service": {
                    "secretOnly": true
                },
                "packages": {
                    "pkg": {
                        "cookieName": "secret_session"
                    }
                }
            }),
        );
        write_yaml(
            &root,
            "configs/services/skiff~run~~account/config.prod.yml",
            &json!({
                "service": {
                    "feature": { "enabled": false },
                    "prodOnly": true,
                    "source": "prod"
                },
                "packages": {
                    "pkg": {
                        "cookieName": "prod_session",
                        "maxAgeSeconds": 999,
                        "prodOnly": true
                    }
                }
            }),
        );

        let services = load_services_from_artifact_pointers(
            &root,
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            vec![ArtifactPointerFile {
                path: root.join("build-record.json"),
                entry: ArtifactIndexPointer {
                    service_id: "skiff.run/account".to_string(),
                    service_version: Some("v1".to_string()),
                    build_id: POINTER_BUILD_ID.to_string(),
                    contract_identity: Some(PROTOCOL_IDENTITY.to_string()),
                    implementation_identity: None,
                    service_unit_path: None,
                    service_assembly: ServiceAssemblyPointer {
                        path: PathBuf::from("local/assembly.json"),
                        assembly_identity: Some(assembly_identity),
                    },
                },
            }],
        )
        .await
        .expect("artifact runtime config should load local dev profile config");

        let service = services.first().expect("service should load");
        let service_config = service.config.resolved_config_value();
        assert_eq!(
            service_config.pointer("/feature/enabled"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            service_config.pointer("/source"),
            Some(&Value::String("dev".to_string()))
        );
        assert_eq!(
            service_config.pointer("/secretOnly"),
            Some(&Value::Bool(true))
        );
        assert_eq!(service_config.pointer("/prodOnly"), None);
        let package_config = service
            .package_configs
            .first()
            .expect("package config should exist")
            .resolved_config_value();
        assert_eq!(
            package_config.pointer("/defaults/fromArtifact"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            package_config.pointer("/defaults/local"),
            Some(&Value::String("dev".to_string()))
        );
        assert_eq!(
            package_config.pointer("/artifactOnly"),
            Some(&Value::String("kept".to_string()))
        );
        assert_eq!(
            package_config.pointer("/cookieName"),
            Some(&Value::String("secret_session".to_string()))
        );
        assert_eq!(package_config.pointer("/maxAgeSeconds"), Some(&json!(60)));
        assert_eq!(package_config.pointer("/devOnly"), Some(&Value::Bool(true)));
        assert_eq!(package_config.pointer("/prodOnly"), None);
    }

    #[tokio::test]
    async fn artifact_runtime_config_uses_empty_local_config_when_missing() {
        let temp = TempDir::new("runtime-config-missing-local-file");
        let root = temp.path().join("artifacts");
        fs::create_dir_all(&root).expect("artifact root should be created");

        write_file_ir(&root, "units/files/service.json");
        write_service_unit_with_package_dependency(&root, "units/services/account.json");
        write_package_index(&root, "skiff.run/pkg", "v1", "units/packages/pkg.json");
        write_package_unit(&root, "units/packages/pkg.json");
        let assembly_identity = write_service_assembly(&root, "local/assembly.json");

        let services = load_services_from_artifact_pointers(
            &root,
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            vec![ArtifactPointerFile {
                path: root.join("build-record.json"),
                entry: ArtifactIndexPointer {
                    service_id: "skiff.run/account".to_string(),
                    service_version: Some("v1".to_string()),
                    build_id: POINTER_BUILD_ID.to_string(),
                    contract_identity: Some(PROTOCOL_IDENTITY.to_string()),
                    implementation_identity: None,
                    service_unit_path: None,
                    service_assembly: ServiceAssemblyPointer {
                        path: PathBuf::from("local/assembly.json"),
                        assembly_identity: Some(assembly_identity),
                    },
                },
            }],
        )
        .await
        .expect("artifact runtime config should fall back when local config is missing");

        let service = services.first().expect("service should load");
        assert_eq!(
            service.config.resolved_config_value(),
            &Value::Object(Map::new())
        );
        assert!(service.service_db.is_none());
        let package_config = service
            .package_configs
            .first()
            .expect("package config should preserve artifact defaults")
            .resolved_config_value();
        assert_eq!(
            package_config.pointer("/defaults/fromArtifact"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            package_config.pointer("/defaults/local"),
            Some(&Value::String("artifact".to_string()))
        );
        assert_eq!(
            package_config.pointer("/artifactOnly"),
            Some(&Value::String("kept".to_string()))
        );
    }

    #[tokio::test]
    async fn artifact_runtime_config_validates_package_config_against_published_shape() {
        let temp = TempDir::new("runtime-config-package-shape");
        let root = temp.path().join("artifacts");
        fs::create_dir_all(&root).expect("artifact root should be created");

        write_file_ir(&root, "units/files/service.json");
        write_service_unit_with_package_dependency(&root, "units/services/account.json");
        write_package_index(&root, "skiff.run/pkg", "v1", "units/packages/pkg.json");
        write_package_unit_with_config_shape(
            &root,
            "units/packages/pkg.json",
            json!({
                "schemaVersion": "skiff-config-shape-v1",
                "entries": [
                    { "path": "requiredSecret", "type": "string", "required": true }
                ]
            }),
        );
        let assembly_identity = write_service_assembly(&root, "local/assembly.json");

        let error = match load_services_from_artifact_pointers(
            &root,
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            vec![ArtifactPointerFile {
                path: root.join("build-record.json"),
                entry: ArtifactIndexPointer {
                    service_id: "skiff.run/account".to_string(),
                    service_version: Some("v1".to_string()),
                    build_id: POINTER_BUILD_ID.to_string(),
                    contract_identity: Some(PROTOCOL_IDENTITY.to_string()),
                    implementation_identity: None,
                    service_unit_path: None,
                    service_assembly: ServiceAssemblyPointer {
                        path: PathBuf::from("local/assembly.json"),
                        assembly_identity: Some(assembly_identity),
                    },
                },
            }],
        )
        .await
        {
            Ok(_) => panic!("missing required package config should be rejected"),
            Err(error) => error,
        };
        let message = format!("{error:#}");
        assert!(message.contains("requiredSecret"), "{message}");
        assert!(
            message.contains("required value is missing or null"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn artifact_runtime_config_rejects_invalid_local_service_db_shape() {
        let temp = TempDir::new("runtime-config-invalid-service-db");
        let root = temp.path().join("artifacts");
        fs::create_dir_all(&root).expect("artifact root should be created");

        write_file_ir(&root, "units/files/service.json");
        write_service_unit(&root, "units/services/account.json");
        let assembly_identity = write_service_assembly(&root, "local/assembly.json");
        write_yaml(
            &root,
            "configs/services/skiff~run~~account/config.yml",
            &json!({
                "service": {
                    "serviceDb": {
                        "mongoUrl": ""
                    }
                }
            }),
        );

        let error = match load_services_from_artifact_pointers(
            &root,
            "runtime-base",
            crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            vec![ArtifactPointerFile {
                path: root.join("build-record.json"),
                entry: ArtifactIndexPointer {
                    service_id: "skiff.run/account".to_string(),
                    service_version: Some("v1".to_string()),
                    build_id: POINTER_BUILD_ID.to_string(),
                    contract_identity: Some(PROTOCOL_IDENTITY.to_string()),
                    implementation_identity: None,
                    service_unit_path: None,
                    service_assembly: ServiceAssemblyPointer {
                        path: PathBuf::from("local/assembly.json"),
                        assembly_identity: Some(assembly_identity),
                    },
                },
            }],
        )
        .await
        {
            Ok(_) => panic!("invalid local serviceDb should be rejected"),
            Err(error) => error,
        };

        assert!(
            format!("{error:#}").contains("service.serviceDb.mongoUrl must be a non-empty string")
        );
    }

    #[test]
    fn package_test_local_config_uses_empty_service_config_when_missing() {
        let temp = TempDir::new("runtime-config-package-test-missing");
        let (production_unit, synthetic_service, layers) =
            package_test_runtime_layers(vec![package_dependency(
                "example.com/dep",
                "1.0.0",
                "dep",
                json!({
                    "dep": {
                        "secret": "artifact-default"
                    }
                }),
            )]);

        let config = load_package_test_local_config(
            temp.path(),
            PACKAGE_TEST_ACTIVATION_ID,
            &production_unit,
            synthetic_service.as_ref(),
            layers.image.as_ref(),
            layers.activation.as_ref(),
            ConfigShape::empty(),
        )
        .expect("missing package-test config should use empty service config");

        assert_eq!(
            config.service_config.resolved_config_value(),
            &Value::Object(Map::new())
        );
        assert!(config.service_db.is_none());
        assert_eq!(
            config.package_configs[0]
                .resolved_config_value()
                .pointer("/dep/secret"),
            Some(&json!("artifact-default"))
        );
    }

    #[test]
    fn package_test_local_config_loads_service_db_and_dependency_overlay() {
        let temp = TempDir::new("runtime-config-package-test-wrapper");
        let (production_unit, synthetic_service, layers) =
            package_test_runtime_layers(vec![package_dependency(
                "example.com/dep",
                "1.0.0",
                "dep",
                json!({
                    "dep": {
                        "secret": "artifact-default",
                        "kept": true
                    }
                }),
            )]);
        write_yaml(
            temp.path(),
            &format!("configs/package-tests/{PACKAGE_TEST_ACTIVATION_ID}/config.yml"),
            &json!({
                "serviceDb": {
                    "mongoUrl": "mongodb://127.0.0.1:27017/package-test"
                },
                "service": {
                    "app": {
                        "secret": "service-secret",
                        "optional": "optional-value"
                    },
                    "serviceDb": {
                        "mongoUrl": "business-config"
                    }
                },
                "packages": {
                    "dep": {
                        "dep": {
                            "secret": "overlay-secret"
                        }
                    }
                }
            }),
        );

        let config = load_package_test_local_config(
            temp.path(),
            PACKAGE_TEST_ACTIVATION_ID,
            &production_unit,
            synthetic_service.as_ref(),
            layers.image.as_ref(),
            layers.activation.as_ref(),
            package_test_service_shape(),
        )
        .expect("package-test config should load");

        assert_eq!(
            config.service_db.as_ref().and_then(service_db_mongo_url),
            Some("mongodb://127.0.0.1:27017/package-test")
        );
        assert_eq!(
            config
                .service_config
                .dispatch_typed_config_target(
                    "config.require",
                    &[json!("app.secret")],
                    Some(&type_plan("string")),
                )
                .expect("service required config should decode"),
            json!("service-secret")
        );
        assert_eq!(
            config
                .service_config
                .dispatch_typed_config_target(
                    "config.optional",
                    &[json!("app.optional")],
                    Some(&type_plan("string")),
                )
                .expect("service optional config should decode"),
            json!("optional-value")
        );
        assert_eq!(
            config
                .service_config
                .dispatch_typed_config_target("config.has", &[json!("app.secret")], None)
                .expect("service has config should decode"),
            json!(true)
        );
        assert_eq!(
            config
                .service_config
                .resolved_config_value()
                .pointer("/serviceDb/mongoUrl"),
            Some(&json!("business-config"))
        );
        assert_eq!(
            config.package_configs[0]
                .resolved_config_value()
                .pointer("/dep/secret"),
            Some(&json!("overlay-secret"))
        );
        assert_eq!(
            config.package_configs[0]
                .resolved_config_value()
                .pointer("/dep/kept"),
            Some(&json!(true))
        );
    }

    #[test]
    fn package_test_local_config_does_not_parse_service_service_db_as_activation() {
        let temp = TempDir::new("runtime-config-package-test-business-service-db");
        let (production_unit, synthetic_service, layers) = package_test_runtime_layers(Vec::new());
        write_yaml(
            temp.path(),
            &format!("configs/package-tests/{PACKAGE_TEST_ACTIVATION_ID}/config.yml"),
            &json!({
                "service": {
                    "app": {
                        "secret": "service-secret"
                    },
                    "serviceDb": {
                        "mongoUrl": "business-config"
                    }
                }
            }),
        );

        let config = load_package_test_local_config(
            temp.path(),
            PACKAGE_TEST_ACTIVATION_ID,
            &production_unit,
            synthetic_service.as_ref(),
            layers.image.as_ref(),
            layers.activation.as_ref(),
            package_test_service_shape(),
        )
        .expect("service.serviceDb should be ordinary service config");

        assert!(config.service_db.is_none());
        assert_eq!(
            config
                .service_config
                .resolved_config_value()
                .pointer("/serviceDb/mongoUrl"),
            Some(&json!("business-config"))
        );
    }

    #[test]
    fn package_test_local_config_rejects_unknown_self_and_duplicate_slot_overlays() {
        let temp = TempDir::new("runtime-config-package-test-invalid-overlays");
        let (production_unit, synthetic_service, layers) =
            package_test_runtime_layers(vec![package_dependency(
                "example.com/dep",
                "1.0.0",
                "dep",
                Value::Null,
            )]);

        write_package_test_config(temp.path(), json!({ "packages": { "unknown": {} } }));
        let error = load_package_test_local_config(
            temp.path(),
            PACKAGE_TEST_ACTIVATION_ID,
            &production_unit,
            synthetic_service.as_ref(),
            layers.image.as_ref(),
            layers.activation.as_ref(),
            ConfigShape::empty(),
        )
        .expect_err("unknown package alias must fail closed");
        assert!(format!("{error:#}").contains("packages.unknown"));

        write_package_test_config(
            temp.path(),
            json!({ "packages": { "example.com/pkg": {} } }),
        );
        let error = load_package_test_local_config(
            temp.path(),
            PACKAGE_TEST_ACTIVATION_ID,
            &production_unit,
            synthetic_service.as_ref(),
            layers.image.as_ref(),
            layers.activation.as_ref(),
            ConfigShape::empty(),
        )
        .expect_err("self package config must fail closed");
        assert!(format!("{error:#}").contains("package under test"));

        let (production_unit, synthetic_service, layers) = package_test_runtime_layers(vec![
            package_dependency("example.com/dep", "1.0.0", "dep", Value::Null),
            package_dependency("example.com/dep", "1.0.0", "depAlt", Value::Null),
        ]);
        write_package_test_config(
            temp.path(),
            json!({
                "packages": {
                    "dep": {},
                    "depAlt": {}
                }
            }),
        );
        let error = load_package_test_local_config(
            temp.path(),
            PACKAGE_TEST_ACTIVATION_ID,
            &production_unit,
            synthetic_service.as_ref(),
            layers.image.as_ref(),
            layers.activation.as_ref(),
            ConfigShape::empty(),
        )
        .expect_err("two overlays to the same slot must fail closed");
        assert!(format!("{error:#}").contains("multiple package configs"));
    }

    #[test]
    fn package_test_local_config_rejects_illegal_activation_ids_before_path_use() {
        let temp = TempDir::new("runtime-config-package-test-illegal-activation");
        let (production_unit, synthetic_service, layers) = package_test_runtime_layers(Vec::new());

        for activation_id in [
            "skiff-package-test-run-v1:",
            "skiff-package-test-run-v1:..",
            "skiff-package-test-run-v1:run/1",
            "skiff-package-test-run-v1:run\\1",
            "skiff-package-test-run-v1:run%1",
            "skiff-package-test-run-v1:run 1",
            "skiff-package-test-run-v1:%2f..%2f",
        ] {
            let error = load_package_test_local_config(
                temp.path(),
                activation_id,
                &production_unit,
                synthetic_service.as_ref(),
                layers.image.as_ref(),
                layers.activation.as_ref(),
                ConfigShape::empty(),
            )
            .expect_err("illegal activation id must fail closed");
            assert!(
                format!("{error:#}").contains("activationId"),
                "{activation_id} error was {error:#}"
            );
        }
    }

    fn write_service_assembly(root: &Path, relative_path: &str) -> String {
        write_service_assembly_with_revision(root, relative_path, SERVICE_REVISION_ID)
    }

    const PACKAGE_TEST_ACTIVATION_ID: &str = "skiff-package-test-run-v1:example~com~~pkg:run:1";

    fn package_test_runtime_layers(
        dependencies: Vec<PackageDependencyConstraint>,
    ) -> (PackageUnit, Arc<ServiceUnit>, Arc<RuntimeProgramLayers>) {
        let mut production_unit = PackageUnit::empty(
            "example.com/pkg",
            "1.0.0",
            "skiff-package-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "skiff-package-abi-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        production_unit.dependencies = dependencies.clone();

        let mut synthetic_service = ServiceUnit::empty(
            "__skiff.package-test/example.com/pkg",
            "1.0.0",
            "skiff-package-test-build-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        );
        synthetic_service.package_dependencies = dependencies;
        let synthetic_service = Arc::new(synthetic_service);

        let mut dep_unit = PackageUnit::empty(
            "example.com/dep",
            "1.0.0",
            "skiff-package-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "skiff-package-abi-v1:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        );
        dep_unit.config_and_effect_metadata.config.insert(
            "shape".to_string(),
            MetadataValue::from_json(json!({
                "schemaVersion": "skiff-config-shape-v1",
                "entries": [
                    { "path": "dep.secret", "type": "string", "required": false }
                ]
            })),
        );

        let layers = crate::program::link_runtime_program_layers(
            synthetic_service.clone(),
            Vec::new(),
            vec![Arc::new(dep_unit)],
            vec![Vec::new()],
        )
        .expect("package-test runtime layers should link");

        (production_unit, synthetic_service, Arc::new(layers))
    }

    fn package_dependency(
        id: &str,
        version: &str,
        alias: &str,
        config: Value,
    ) -> PackageDependencyConstraint {
        PackageDependencyConstraint {
            id: id.to_string(),
            version: version.to_string(),
            alias: alias.to_string(),
            config,
        }
    }

    fn package_test_service_shape() -> ConfigShape {
        serde_json::from_value(json!({
            "schemaVersion": "skiff-config-shape-v1",
            "entries": [
                { "path": "app.secret", "type": "string", "required": true },
                { "path": "app.optional", "type": "string", "required": false },
                { "path": "serviceDb.mongoUrl", "type": "string", "required": false }
            ]
        }))
        .expect("test package service shape should parse")
    }

    fn empty_config_shape_json() -> Value {
        json!({
            "schemaVersion": "skiff-config-shape-v1",
            "entries": []
        })
    }

    fn type_plan(name: &str) -> RuntimeTypePlan {
        RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": name, "args": [] }))
            .expect("config test type plan should build")
    }

    fn write_package_test_config(root: &Path, value: Value) {
        write_yaml(
            root,
            &format!("configs/package-tests/{PACKAGE_TEST_ACTIVATION_ID}/config.yml"),
            &value,
        );
    }

    fn write_service_assembly_with_revision(
        root: &Path,
        relative_path: &str,
        revision_id: &str,
    ) -> String {
        let mut assembly = json!({
            "schemaVersion": "skiff-service-assembly-v1",
            "kind": "service",
            "service": {
                "id": "skiff.run/account",
                "revisionId": revision_id,
                "protocolIdentity": PROTOCOL_IDENTITY
            },
            "serviceUnit": {
                "unitPath": "units/services/account.json"
            },
            "files": [],
            "operations": [],
            "gateway": {},
            "configShape": empty_config_shape_json()
        });
        let hash = value_sha256(
            &service_assembly_hash_input(&assembly)
                .expect("service assembly hash input should build"),
        )
        .expect("service assembly hash should compute");
        let assembly_identity = format!("{SERVICE_ASSEMBLY_IDENTITY_PREFIX}:sha256:{hash}");
        assembly["service"]["assemblyIdentity"] = Value::String(assembly_identity.clone());
        write_json(root, relative_path, assembly);
        assembly_identity
    }

    fn write_dev_pointer(root: &Path, assembly_identity: &str) {
        let assembly_hash = assembly_identity
            .rsplit_once(":sha256:")
            .expect("assembly identity should contain sha256")
            .1;
        let protocol_hash = PROTOCOL_IDENTITY
            .rsplit_once(":sha256:")
            .expect("protocol identity should contain sha256")
            .1;
        write_json(
            root,
            "dev/services/skiff~run~~account.json",
            json!({
                "mode": "dev",
                "serviceId": "skiff.run/account",
                "profile": "test",
                "protocolIdentity": PROTOCOL_IDENTITY,
                "contractHash": format!("sha256:{protocol_hash}"),
                "buildId": format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{assembly_hash}"),
                "serviceAssembly": {
                    "assemblyIdentity": assembly_identity,
                    "assemblyPath": "local/assembly.json"
                }
            }),
        );
    }

    fn write_release_pointer(root: &Path, assembly_identity: &str) {
        let assembly_hash = assembly_identity
            .rsplit_once(":sha256:")
            .expect("assembly identity should contain sha256")
            .1;
        let build_id = format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{assembly_hash}");
        let build_hash =
            identity_hash_with_label(&build_id, "buildId").expect("build id hash should compute");
        write_json(
            root,
            "versions/services/skiff~run~~account/v1.json",
            json!({
                "schemaVersion": "skiff-service-version-pointer-v1",
                "serviceId": "skiff.run/account",
                "version": "v1",
                "buildId": build_id
            }),
        );
        write_json(
            root,
            &format!("builds/services/skiff~run~~account/{build_hash}.json"),
            json!({
                "schemaVersion": "skiff-service-build-v1",
                "serviceId": "skiff.run/account",
                "serviceVersion": "v1",
                "buildId": build_id,
                "contractIdentity": PROTOCOL_IDENTITY,
                "serviceAssembly": {
                    "assemblyIdentity": assembly_identity,
                    "assemblyPath": "local/assembly.json"
                }
            }),
        );
    }

    fn service_publication_abi_json() -> Value {
        let operation = service_operation_ref_json();
        json!({
            "schemaVersion": "skiff-publication-abi-unit-v1",
            "publicationId": "skiff.run/account",
            "version": "v1",
            "abiIdentity": PROTOCOL_IDENTITY,
            "operationExports": [operation.clone()],
            "operationAbi": [
                {
                    "operation": operation.clone(),
                    "publicSignature": {
                        "params": [],
                        "returnType": { "kind": "builtin", "name": "Json" },
                        "maySuspend": false
                    }
                }
            ],
            "sourceCallOperationIndex": [
                {
                    "sourceCallPath": "run",
                    "operation": operation
                }
            ]
        })
    }

    fn service_operation_ref_json() -> Value {
        json!({
            "operationAbiId": SERVICE_RUN_OPERATION_ABI_ID,
            "kind": "publicFunction",
            "publicPath": "run",
            "displayName": "svc.main.run"
        })
    }

    fn service_operation_json() -> Value {
        json!({
            "kind": "localExecutable",
            "operation": service_operation_ref_json(),
            "executable": {
                "fileRef": {
                    "fileIrIdentity": "file:service",
                    "modulePath": "svc.main"
                },
                "executableIndex": 0,
                "callableAbiId": "callable:svc.main.run",
                "callableKind": "publicFunction"
            }
        })
    }

    fn empty_publication_abi_json(
        publication_id: &str,
        version: &str,
        abi_identity: &str,
    ) -> Value {
        json!({
            "schemaVersion": "skiff-publication-abi-unit-v1",
            "publicationId": publication_id,
            "version": version,
            "abiIdentity": abi_identity
        })
    }

    fn write_service_unit(root: &Path, relative_path: &str) {
        write_json(
            root,
            relative_path,
            json!({
                "schemaVersion": "skiff-service-unit-v1",
                "service": {
                    "id": "skiff.run/account",
                    "displayName": "Account"
                },
                "version": "v1",
                "protocolIdentity": PROTOCOL_IDENTITY,
                "publicationAbi": service_publication_abi_json(),
                "files": [
                    {
                        "fileIrIdentity": "file:service",
                        "modulePath": "svc.main",
                        "artifactPath": "units/files/service.json",
                        "sourceAstHash": "source:file:service"
                    }
                ],
                "packageDependencies": [],
                "packageAbiExpectations": [],
                "operations": [service_operation_json()],
                "gateway": {},
                "config": {}
            }),
        );
    }

    fn write_service_unit_with_package_dependency(root: &Path, relative_path: &str) {
        write_json(
            root,
            relative_path,
            json!({
                "schemaVersion": "skiff-service-unit-v1",
                "service": {
                    "id": "skiff.run/account",
                    "displayName": "Account"
                },
                "version": "v1",
                "protocolIdentity": PROTOCOL_IDENTITY,
                "publicationAbi": service_publication_abi_json(),
                "files": [
                    {
                        "fileIrIdentity": "file:service",
                        "modulePath": "svc.main",
                        "artifactPath": "units/files/service.json",
                        "sourceAstHash": "source:file:service"
                    }
                ],
                "packageDependencies": [
                    {
                        "id": "skiff.run/pkg",
                        "version": "v1",
                        "alias": "pkg",
                        "config": {
                            "defaults": {
                                "fromArtifact": true,
                                "local": "artifact"
                            },
                            "artifactOnly": "kept"
                        }
                    }
                ],
                "packageAbiExpectations": [],
                "operations": [service_operation_json()],
                "gateway": {},
                "config": {}
            }),
        );
    }

    fn write_file_ir(root: &Path, relative_path: &str) {
        write_json(
            root,
            relative_path,
            json!({
                "schemaVersion": "skiff-file-ir-v3",
                "fileIrIdentity": "file:service",
                "sourceAstHash": "source:file:service",
                "modulePath": "svc.main",
                "irFormatVersion": "skiff-file-ir-format-v1",
                "opcodeTableVersion": "skiff-opcode-table-v1",
                "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
                "declarations": { "interfaces": {} },
                "linkTargets": {
                    "executables": {
                        "run": { "executableIndex": 0 }
                    }
                },
                "typeTable": [],
                "constants": [],
                "executables": [
                    {
                        "kind": "function",
                        "symbol": "run",
                        "returnType": { "kind": "builtin", "name": "Json" },
                        "slots": { "slots": [], "frameSize": 0 },
                        "maySuspend": false,
                        "body": {}
                    }
                ],
                "externalRefs": {}
            }),
        );
    }

    fn write_package_index(root: &Path, package_id: &str, version: &str, package_unit_path: &str) {
        write_json(
            root,
            &format!(
                "indexes/packages/{}/versions/{version}.json",
                storage_segment_for_test(package_id)
            ),
            json!({
                "schemaVersion": "skiff-package-unit-index-v1",
                "packageId": package_id,
                "version": version,
                "packageUnit": {
                    "unitPath": package_unit_path
                }
            }),
        );
    }

    fn write_package_unit(root: &Path, relative_path: &str) {
        write_package_unit_with_metadata(root, relative_path, json!({}));
    }

    fn write_package_unit_with_config_shape(root: &Path, relative_path: &str, shape: Value) {
        write_package_unit_with_metadata(
            root,
            relative_path,
            json!({
                "config": {
                    "shape": shape
                }
            }),
        );
    }

    fn write_package_unit_with_metadata(root: &Path, relative_path: &str, metadata: Value) {
        write_json(
            root,
            relative_path,
            json!({
                "schemaVersion": "skiff-package-unit-v1",
                "packageId": "skiff.run/pkg",
                "version": "v1",
                "buildIdentity": "skiff-package-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "abiIdentity": "skiff-publication-abi-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "publicationAbi": empty_publication_abi_json(
                    "skiff.run/pkg",
                    "v1",
                    "skiff-publication-abi-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                ),
                "files": [],
                "implementationLinks": {},
                "dependencies": [],
                "configAndEffectMetadata": metadata
            }),
        );
    }

    fn storage_segment_for_test(publication_id: &str) -> String {
        publication_id.replace('.', "~").replace('/', "~~")
    }

    fn write_yaml(root: &Path, relative_path: &str, value: &Value) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("artifact path should have parent"))
            .expect("artifact directory should be created");
        fs::write(
            path,
            serde_yaml::to_string(value).expect("test YAML should serialize"),
        )
        .expect("artifact file should be written");
    }

    fn write_json(root: &Path, relative_path: &str, value: Value) {
        let value = canonicalize_test_artifact_json(value);
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("artifact path should have parent"))
            .expect("artifact directory should be created");
        fs::write(
            path,
            serde_json::to_vec_pretty(&value).expect("test JSON should serialize"),
        )
        .expect("artifact file should be written");
    }

    fn canonicalize_test_artifact_json(mut value: Value) -> Value {
        match value.get("schemaVersion").and_then(Value::as_str) {
            Some("skiff-file-ir-v3") => {
                let unit: skiff_artifact_model::FileIrUnit =
                    serde_json::from_value(value.clone()).expect("test File IR should parse");
                let identity =
                    file_ir_identity(&unit).expect("test File IR identity should compute");
                value["fileIrIdentity"] = json!(identity.clone());
                value["sourceAstHash"] = json!(format!("source:{identity}"));
                value
            }
            Some("skiff-package-unit-v1") => {
                let unit: skiff_artifact_model::PackageUnit =
                    serde_json::from_value(value.clone()).expect("test package unit should parse");
                value["buildIdentity"] =
                    json!(package_build_identity(&unit).expect("package build identity"));
                value["abiIdentity"] =
                    json!(package_abi_identity(&unit).expect("package ABI identity"));
                value
            }
            Some("skiff-service-unit-v1") => {
                rewrite_service_file_identity_aliases(&mut value);
                value
            }
            _ => value,
        }
    }

    fn rewrite_service_file_identity_aliases(value: &mut Value) {
        match value {
            Value::Object(object) => {
                if object.get("fileIrIdentity").and_then(Value::as_str) == Some("file:service") {
                    let identity = service_file_identity_for_test();
                    object.insert("fileIrIdentity".to_string(), json!(identity.clone()));
                    if object.contains_key("sourceAstHash") {
                        object.insert(
                            "sourceAstHash".to_string(),
                            json!(format!("source:{identity}")),
                        );
                    }
                }
                for child in object.values_mut() {
                    rewrite_service_file_identity_aliases(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    rewrite_service_file_identity_aliases(item);
                }
            }
            _ => {}
        }
    }

    fn service_file_identity_for_test() -> String {
        let unit: skiff_artifact_model::FileIrUnit = serde_json::from_value(json!({
            "schemaVersion": "skiff-file-ir-v3",
            "fileIrIdentity": "file:service",
            "sourceAstHash": "source:file:service",
            "modulePath": "svc.main",
            "irFormatVersion": "skiff-file-ir-format-v1",
            "opcodeTableVersion": "skiff-opcode-table-v1",
            "sourceMap": { "format": "skiff-file-ir-source-map-v1" },
            "declarations": { "interfaces": {} },
            "linkTargets": {
                "executables": {
                    "run": { "executableIndex": 0 }
                }
            },
            "typeTable": [],
            "constants": [],
            "executables": [
                {
                    "kind": "function",
                    "symbol": "run",
                    "returnType": { "kind": "builtin", "name": "Json" },
                    "slots": { "slots": [], "frameSize": 0 },
                    "maySuspend": false,
                    "body": {}
                }
            ],
            "externalRefs": {}
        }))
        .expect("test service File IR should parse");
        file_ir_identity(&unit).expect("test service File IR identity should compute")
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
