use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_core::path_safety::{
    is_safe_publication_artifact_id_component, is_safe_publication_artifact_path_segment,
};

use super::model::PublishedPackageArtifacts;
use crate::error::{EmissionError, Result};
use crate::projection::context::{dependency_config_is_empty, ProjectedPackageDependency};

/// A package dependency entry in a package/service assembly. `config` carries
/// the user-provided dependency config (an open schema) and is omitted when
/// empty. Field order matches the former `json!`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDependencyEntry {
    id: String,
    version: String,
    alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<Value>,
    assembly_identity: String,
    assembly_path: String,
}

pub fn package_dependency_entries(
    dependencies: &[ProjectedPackageDependency],
    package_artifacts: &BTreeMap<String, PublishedPackageArtifacts>,
) -> Result<Vec<PackageDependencyEntry>> {
    dependencies
        .iter()
        .map(|dependency| {
            let Some(artifact) = package_artifacts.get(&dependency.id) else {
                return Err(EmissionError::ContractValidation {
                    message: format!(
                        "package dependency {} has no published package assembly",
                        dependency.id
                    ),
                });
            };
            let config = (!dependency_config_is_empty(&dependency.config))
                .then(|| dependency.config.clone());
            Ok(PackageDependencyEntry {
                id: dependency.id.clone(),
                version: dependency.version.clone(),
                alias: dependency.effective_alias().to_string(),
                config,
                assembly_identity: artifact.assembly.identity.clone(),
                assembly_path: artifact.assembly.path.clone(),
            })
        })
        .collect()
}

pub fn package_artifact_assembly_path(package_id: &str, hash: &str) -> String {
    let package_path = package_artifact_path(package_id);
    format!("assemblies/packages/{package_path}/{hash}.json")
}

pub fn package_version_index_path(package_id: &str, version: &str) -> String {
    let package_path = package_artifact_path(package_id);
    assert_safe_package_artifact_path_segment(version, "package version");
    format!("indexes/packages/{package_path}/versions/{version}.json")
}

fn package_artifact_path(package_id: &str) -> String {
    assert!(
        is_safe_publication_artifact_id_component(package_id),
        "package id `{package_id}` must be safe for package artifact paths"
    );
    PublicationId::parse(package_id)
        .expect("package id was validated before artifact projection")
        .artifact_path()
}

fn assert_safe_package_artifact_path_segment(segment: &str, label: &str) {
    assert!(
        is_safe_publication_artifact_path_segment(segment),
        "{label} `{segment}` must be safe for package artifact paths"
    );
}
