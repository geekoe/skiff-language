use std::{
    collections::BTreeMap,
    ops::Deref,
    path::{Path, PathBuf},
};

use serde_json::Value;
use thiserror::Error;

use crate::{
    dependency_config_is_empty as publication_dependency_config_is_empty,
    empty_dependency_config as publication_empty_dependency_config,
    is_complex_package_dependency_id, is_reserved_source_import_alias,
    is_standard_package_id as publication_is_standard_package_id, PublicationApiSpec,
    PublicationApiSpecEntry, PublicationManifest, ResolvedPackage,
};

pub use crate::PackageDependency;

mod export_resolver;
mod import_resolution;
mod manifest_discovery;
mod manifest_io;
mod manifest_validation;
mod transitive_dependencies;

pub const PACKAGE_CONFIG_FILE: &str = "package.yml";
pub use skiff_artifact_identity::PACKAGE_ASSEMBLY_IDENTITY_PREFIX;
pub const PACKAGE_SEARCH_DIR: &str = ".skiff-packages";

pub use export_resolver::PackageExportResolver;

pub type PackageManifestKey = (String, String);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageResolutionDirs {
    pub package_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifest {
    pub publication: PublicationManifest,
}

impl PackageManifest {
    pub fn new(publication: PublicationManifest) -> Self {
        Self { publication }
    }

    pub fn into_publication(self) -> PublicationManifest {
        self.publication
    }
}

impl Deref for PackageManifest {
    type Target = PublicationManifest;

    fn deref(&self) -> &Self::Target {
        &self.publication
    }
}

pub type PackageApi = PublicationApiSpec;
pub type PackageApiEntry = PublicationApiSpecEntry;

pub fn package_manifest_key(id: &str, version: &str) -> PackageManifestKey {
    (id.to_string(), version.to_string())
}

pub fn manifest_key(manifest: &PackageManifest) -> PackageManifestKey {
    package_manifest_key(manifest.id.as_str(), &manifest.version)
}

pub fn empty_dependency_config() -> Value {
    publication_empty_dependency_config()
}

pub fn dependency_config_is_empty(value: &Value) -> bool {
    publication_dependency_config_is_empty(value)
}

#[derive(Debug, Error)]
pub enum PackageConfigError {
    #[error("failed to read package search directory {path}: {source}")]
    ReadPackageDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read package manifest {path}: {source}")]
    ReadPackageManifest {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse package manifest {path}: {message}")]
    ParsePackageManifest { path: String, message: String },
    #[error("package manifest validation failed:\n{message}")]
    Validation { message: String },
}

pub fn discover_package_manifests(
    root: &Path,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    manifest_discovery::discover_package_manifests(root)
}

pub fn discover_package_manifests_with_dirs(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    manifest_discovery::discover_package_manifests_with_dirs(root, package_dirs)
}

pub fn discover_package_manifests_with_dependency_dirs(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
    dependencies: &[PackageDependency],
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    manifest_discovery::discover_package_manifests_with_dependency_dirs(
        root,
        package_dirs,
        dependencies,
    )
}

#[cfg(test)]
fn discover_builtin_std_registry_manifests(
    std_dir: &Path,
    registry_path: &Path,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    manifest_discovery::discover_builtin_std_registry_manifests(std_dir, registry_path)
}

pub fn read_user_package_manifest(path: &Path) -> Result<PackageManifest, PackageConfigError> {
    manifest_io::read_user_package_manifest(path)
}

fn is_enabled_standard_package_id(id: &str) -> bool {
    is_standard_package_id(id)
}

pub fn is_standard_package_id(id: &str) -> bool {
    publication_is_standard_package_id(id)
}

pub fn is_reserved_package_alias(alias: &str) -> bool {
    is_reserved_source_import_alias(alias)
}

pub fn is_complex_package_id(package_id: &str) -> bool {
    is_complex_package_dependency_id(package_id)
}

pub fn package_alias_bindings(
    dependencies: &[PackageDependency],
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
) -> BTreeMap<String, Vec<String>> {
    PackageExportResolver::alias_bindings(dependencies, available)
}

pub fn resolve_package_imports(
    imports: &[Vec<String>],
    dependencies: &[PackageDependency],
    available: &BTreeMap<PackageManifestKey, PackageManifest>,
) -> Result<Vec<ResolvedPackage>, PackageConfigError> {
    import_resolution::resolve_package_imports(imports, dependencies, available)
}

#[cfg(test)]
mod tests;
