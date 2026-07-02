use std::{collections::BTreeSet, path::Path};

use serde::Deserialize;
use serde_json::Value;
use serde_yaml::Value as YamlValue;

use super::{
    is_enabled_standard_package_id, is_reserved_package_alias, is_standard_package_id,
    PackageConfigError, PackageDependency, PackageManifest,
};
use crate::{
    canonical_publication_dependency_id, collect_package_dependency_violations,
    is_publication_dependency_id, is_safe_publication_artifact_id_component,
    is_safe_publication_artifact_path_segment, parse_publication_id_field,
    validate_publication_version_field, ManifestOwner, ManifestProvenance, PublicationApiSpec,
    PublicationManifest,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPackageManifest {
    id: Option<String>,
    version: Option<String>,
    api: Option<YamlValue>,
    exports: Option<Value>,
    #[serde(default)]
    packages: Vec<PackageDependency>,
    requires: Option<RawPackageRequires>,
    dependencies: Option<RawPackageDependencies>,
    #[serde(default)]
    #[serde(rename = "valuesRequirements")]
    values_requirements: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackageDependencies {
    packages: Option<Value>,
    services: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackageRequires {
    bindings: Option<Value>,
    services: Option<Value>,
}

pub(super) type PackageManifestOwner = ManifestOwner;

pub(super) fn validate_package_manifest(
    raw: RawPackageManifest,
    path: &Path,
    owner: PackageManifestOwner,
    api: PublicationApiSpec,
) -> Result<PackageManifest, PackageConfigError> {
    let mut violations = Vec::new();
    let id = parse_publication_id_field("id", raw.id, &mut violations);
    let version = validate_publication_version_field("version", raw.version, &mut violations);
    if raw.exports.is_some() {
        violations.push("exports has been removed; use top-level api".to_string());
    }
    if raw.api.is_some() {
        violations.push("api has been removed; declare public API in api.yml".to_string());
    }
    if raw
        .dependencies
        .as_ref()
        .is_some_and(|dependencies| dependencies.packages.is_some())
    {
        violations
            .push("dependencies.packages has been removed; use top-level packages".to_string());
    }
    if raw
        .dependencies
        .as_ref()
        .is_some_and(|dependencies| dependencies.services.is_some())
    {
        violations.push("dependencies.services has been removed; service dependencies are only valid in service.yml top-level services".to_string());
    }
    validate_removed_requires(raw.requires, &mut violations);
    if raw.values_requirements.is_some() {
        violations.push(
            "valuesRequirements has been removed; use config.require<T>(path) or config.optional<T>(path) in Skiff source".to_string(),
        );
    }

    let mut dependencies = raw.packages;
    for dependency in &mut dependencies {
        if let Some(canonical_id) = canonical_publication_dependency_id(&dependency.id) {
            dependency.id = canonical_id;
        }
    }
    dependencies.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.alias.cmp(&right.alias))
    });
    dependencies.dedup();
    let mut dependency_aliases = BTreeSet::new();
    for dependency in &dependencies {
        collect_package_dependency_violations(
            dependency,
            "packages",
            &mut dependency_aliases,
            &mut violations,
        );
    }

    if let Some(id) = id.as_ref() {
        let id = id.as_str();
        let is_compiler_standard_package = owner == ManifestOwner::CompilerStandardPackage;
        if is_compiler_standard_package && !is_enabled_standard_package_id(id) {
            violations.push(format!(
                "standard package id {id} is not enabled; allowed standard package is skiff.run/std"
            ));
        }
        if id == "ext" || id.starts_with("ext.") {
            violations.push("ext root has been removed".to_string());
        }
        if ((is_standard_package_id(id) || id.starts_with("std.")) && !is_compiler_standard_package)
            || id == "connect"
            || (!is_compiler_standard_package
                && !id.contains(['.', '/'])
                && is_reserved_package_alias(id)
                && id != "ext")
        {
            violations.push(format!("package id {id} is reserved"));
        }
        if !is_publication_dependency_id(id) {
            violations.push(format!("package id {id} must be a publication id"));
        } else if !is_safe_publication_artifact_id_component(id) {
            violations.push(format!(
                "package id {id} must be safe for package artifact paths"
            ));
        }
    }

    if let Some(version) = &version {
        if !is_safe_publication_artifact_path_segment(version) {
            violations.push(format!(
                "package version {version} must be safe for package artifact paths"
            ));
        }
    }

    if !violations.is_empty() {
        return Err(PackageConfigError::Validation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {}: {violation}", path.display()))
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }

    let mut publication = PublicationManifest::new(
        id.expect("validated package id"),
        version.expect("validated package version"),
        api,
        dependencies,
        ManifestProvenance::file(path, owner),
    );
    Ok(PackageManifest::new(publication))
}

fn validate_removed_requires(requires: Option<RawPackageRequires>, violations: &mut Vec<String>) {
    let Some(requires) = requires else {
        return;
    };
    if requires.services.is_some() {
        violations.push("requires.services has been removed; package source cannot declare service dependencies".to_string());
    }
    if requires.bindings.is_some() {
        violations.push(
            "requires.bindings has been removed; pass any interface values as package entry parameters"
                .to_string(),
        );
    }
}
