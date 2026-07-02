use std::collections::BTreeMap;

use serde_json::Value;

use super::{
    dependency_config_is_empty, empty_dependency_config, import_resolution::resolve_package,
    package_manifest_key, PackageConfigError, PackageDependency, PackageManifest,
    PackageManifestKey,
};
use crate::ResolvedPackage;

pub(super) fn add_transitive_package_dependencies(
    selected: &mut BTreeMap<String, ResolvedPackage>,
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
    direct_dependencies: &[PackageDependency],
) -> Result<(), PackageConfigError> {
    let direct_configs = direct_dependencies
        .iter()
        .map(|dependency| (dependency.id.clone(), dependency.config.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut upstream_defaults = BTreeMap::<String, Vec<ConfigContribution>>::new();
    let mut stack = selected
        .values()
        .flat_map(|package| {
            package.manifest.dependencies.iter().map(|dependency| {
                (
                    package.manifest.id.to_string(),
                    dependency.id.clone(),
                    dependency.version.clone(),
                    dependency.config.clone(),
                )
            })
        })
        .collect::<Vec<_>>();
    let mut violations = Vec::new();

    while let Some((source_package_id, package_id, version, config)) = stack.pop() {
        if !dependency_config_is_empty(&config) {
            upstream_defaults
                .entry(package_id.clone())
                .or_default()
                .push(ConfigContribution {
                    source_package_id: source_package_id.clone(),
                    config,
                });
        }

        if let Some(selected_package) = selected.get(&package_id) {
            if selected_package.manifest.version != version {
                violations.push(format!(
                    "package dependency {package_id} version {version} does not match selected package.yml version {}",
                    selected_package.manifest.version
                ));
            }
            continue;
        }
        let Some(manifest) = manifest_for_dependency(&package_id, &version, available) else {
            violations.push(format!(
                "package dependency {package_id} version {version} has no matching package.yml"
            ));
            continue;
        };
        if manifest.version != version {
            violations.push(format!(
                "package dependency {package_id} version {version} does not match package.yml version {}",
                manifest.version
            ));
            continue;
        }
        stack.extend(manifest.dependencies.iter().map(|dependency| {
            (
                manifest.id.to_string(),
                dependency.id.clone(),
                dependency.version.clone(),
                dependency.config.clone(),
            )
        }));
        selected.insert(manifest.id.to_string(), resolve_package(manifest));
    }

    if violations.is_empty() {
        for (package_id, package) in selected.iter_mut() {
            package.config = merge_package_config(
                package_id,
                upstream_defaults
                    .get(package_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                direct_configs.get(package_id),
                &mut violations,
            );
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(PackageConfigError::Validation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

fn manifest_for_dependency<'a>(
    package_id: &str,
    version: &str,
    available: &'a BTreeMap<PackageManifestKey, PackageManifest>,
) -> Option<&'a PackageManifest> {
    available.get(&package_manifest_key(package_id, version))
}

#[derive(Debug)]
struct ConfigContribution {
    source_package_id: String,
    config: Value,
}

fn merge_package_config(
    package_id: &str,
    defaults: &[ConfigContribution],
    direct: Option<&Value>,
    violations: &mut Vec<String>,
) -> Value {
    let direct = direct.unwrap_or(&Value::Null);
    let mut merged = empty_dependency_config();
    let mut sources = BTreeMap::<String, String>::new();

    for contribution in defaults {
        merge_upstream_config(
            package_id,
            &mut merged,
            &mut sources,
            "",
            &contribution.config,
            &contribution.source_package_id,
            direct,
            violations,
        );
    }
    if matches!(direct, Value::Object(_)) {
        overlay_config(&mut merged, direct);
    }
    merged
}

fn merge_upstream_config(
    package_id: &str,
    target: &mut Value,
    sources: &mut BTreeMap<String, String>,
    path: &str,
    incoming: &Value,
    source_package_id: &str,
    direct: &Value,
    violations: &mut Vec<String>,
) {
    let (Value::Object(target_object), Value::Object(incoming_object)) = (target, incoming) else {
        return;
    };
    for (key, incoming_value) in incoming_object {
        let field_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        match target_object.get_mut(key) {
            Some(existing @ Value::Object(_)) if incoming_value.is_object() => {
                merge_upstream_config(
                    package_id,
                    existing,
                    sources,
                    &field_path,
                    incoming_value,
                    source_package_id,
                    direct,
                    violations,
                );
            }
            Some(existing) if existing == incoming_value => {}
            Some(_) if config_has_path(direct, &field_path) => {}
            Some(_) => {
                let first_source = sources
                    .get(&field_path)
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_string());
                violations.push(format!(
                    "package dependency {package_id} config field {field_path} has conflicting defaults from {first_source} and {source_package_id}; choose one package.yml default or move runtime config to config source packages.<alias>"
                ));
            }
            None => {
                target_object.insert(key.clone(), incoming_value.clone());
                for leaf in config_leaf_paths(&field_path, incoming_value) {
                    sources.insert(leaf, source_package_id.to_string());
                }
            }
        }
    }
}

fn overlay_config(target: &mut Value, overlay: &Value) {
    let (Value::Object(target_object), Value::Object(overlay_object)) = (target, overlay) else {
        return;
    };
    for (key, overlay_value) in overlay_object {
        match (target_object.get_mut(key), overlay_value) {
            (Some(existing @ Value::Object(_)), Value::Object(_)) => {
                overlay_config(existing, overlay_value);
            }
            _ => {
                target_object.insert(key.clone(), overlay_value.clone());
            }
        }
    }
}

fn config_has_path(config: &Value, path: &str) -> bool {
    let mut current = config;
    for segment in path.split('.') {
        let Some(next) = current.get(segment) else {
            return false;
        };
        current = next;
    }
    true
}

fn config_leaf_paths(path: &str, value: &Value) -> Vec<String> {
    let Value::Object(object) = value else {
        return vec![path.to_string()];
    };
    if object.is_empty() {
        return vec![path.to_string()];
    }
    let mut paths = Vec::new();
    for (key, nested) in object {
        let nested_path = format!("{path}.{key}");
        paths.extend(config_leaf_paths(&nested_path, nested));
    }
    paths
}
