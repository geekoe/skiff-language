use std::collections::BTreeMap;

use super::{
    is_standard_package_id, package_manifest_key, PackageConfigError, PackageDependency,
    PackageExportResolver, PackageManifest, PackageManifestKey,
};
use crate::ResolvedPackage;

pub(super) fn resolve_package_imports(
    imports: &[Vec<String>],
    dependencies: &[PackageDependency],
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
) -> Result<Vec<ResolvedPackage>, PackageConfigError> {
    let mut selected = BTreeMap::<String, ResolvedPackage>::new();
    let mut violations = Vec::new();
    for dependency in dependencies {
        let Some(manifest) =
            available.get(&package_manifest_key(&dependency.id, &dependency.version))
        else {
            violations.push(format!(
                "package dependency {} version {} has no matching package.yml",
                dependency.id, dependency.version
            ));
            continue;
        };
        if let Some(selected_package) = selected.get(manifest.id.as_str()) {
            if selected_package.manifest.version != dependency.version {
                violations.push(format!(
                    "package dependency {} version {} conflicts with selected package.yml version {}",
                    dependency.id, dependency.version, selected_package.manifest.version
                ));
            }
            continue;
        }
        if manifest.version != dependency.version {
            violations.push(format!(
                "package dependency {} version {} does not match package.yml version {}",
                dependency.id, dependency.version, manifest.version
            ));
            continue;
        }
        selected.insert(
            manifest.id.to_string(),
            resolve_package_with_config(manifest, dependency.config.clone()),
        );
    }
    for import in imports {
        if import.first().map(String::as_str) == Some("ext") {
            violations.push("ext root has been removed".to_string());
            continue;
        }
        if import.first().map(String::as_str) == Some("std") {
            if !is_allowed_standard_library_import(import) {
                let import_path = import.join(".");
                let legacy_values_import = ["std", "values"].join(".");
                if import_path == legacy_values_import {
                    violations.push(format!(
                        "import {legacy_values_import} is invalid: use config.require<T>(path) or config.optional<T>(path)"
                    ));
                } else {
                    violations.push(format!("import {import_path} is invalid: import std"));
                }
            } else if let Some(manifest) =
                standard_library_manifest_for_import(import, dependencies, available)
            {
                selected
                    .entry(manifest.id.to_string())
                    .or_insert_with(|| resolve_package(manifest));
            }
            continue;
        }

        for package_key in package_keys_for_import(import, dependencies, available, &mut violations)
        {
            if let Some(manifest) = available.get(&package_key) {
                if let Some(selected_package) = selected.get(manifest.id.as_str()) {
                    if selected_package.manifest.version != manifest.version {
                        violations.push(format!(
                            "import {} selects package {} version {} but version {} is already selected",
                            import.join("."),
                            manifest.id,
                            manifest.version,
                            selected_package.manifest.version
                        ));
                    }
                    continue;
                }
                selected
                    .entry(manifest.id.to_string())
                    .or_insert_with(|| resolve_package(manifest));
            }
        }
    }

    if !violations.is_empty() {
        return Err(PackageConfigError::Validation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }

    super::transitive_dependencies::add_transitive_package_dependencies(
        &mut selected,
        available,
        dependencies,
    )?;

    Ok(selected.into_values().collect())
}

pub(super) fn resolve_package(manifest: &PackageManifest) -> ResolvedPackage {
    resolve_package_with_config(manifest, super::empty_dependency_config())
}

pub(super) fn resolve_package_with_config(
    manifest: &PackageManifest,
    config: serde_json::Value,
) -> ResolvedPackage {
    ResolvedPackage {
        manifest: manifest.clone(),
        config,
    }
}

fn is_allowed_standard_library_import(import: &[String]) -> bool {
    matches!(import, [root] if root == "std")
}

fn standard_library_manifest_for_import<'a>(
    _import: &[String],
    _dependencies: &[PackageDependency],
    available: &'a BTreeMap<PackageManifestKey, PackageManifest>,
) -> Option<&'a PackageManifest> {
    available
        .iter()
        .find(|((id, _version), _manifest)| is_standard_package_id(id))
        .map(|(_key, manifest)| manifest)
}

fn package_keys_for_import(
    import: &[String],
    dependencies: &[PackageDependency],
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
    violations: &mut Vec<String>,
) -> Vec<PackageManifestKey> {
    let full = import.join(".");
    let resolved =
        PackageExportResolver::dependency_ids_for_import(import, dependencies, available);

    if resolved.package_keys.is_empty() && !resolved.blocked_complex_package_ids.is_empty() {
        violations.push(format!(
            "import {full} matches package {} but requires packages alias for {}",
            resolved.blocked_complex_package_ids.join(", "),
            resolved.blocked_complex_package_ids.join(", ")
        ));
    }

    resolved.package_keys
}
