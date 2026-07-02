use serde_json::Value;

pub use skiff_compiler_input_model::{
    canonical_publication_dependency_id, collect_package_dependency_violations,
    dependency_config_is_empty, empty_dependency_config, is_complex_package_dependency_id,
    is_publication_dependency_id, is_reserved_source_import_alias,
    is_safe_publication_artifact_id_component, is_safe_publication_artifact_path_segment,
    is_standard_package_id, is_valid_source_import_alias, ResolvedServiceDependencies,
    ServiceDependencyLockEntry, ServiceDependencyRemoteBoxProvenance,
};
pub use skiff_compiler_input_model::{PackageDependency, ServiceDependency};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    pub manifest: crate::package_config::PackageManifest,
    pub config: Value,
}

#[derive(Debug, Clone)]
pub struct ResolvedPackageGraph {
    dependencies: Vec<PackageDependency>,
}

impl ResolvedPackageGraph {
    pub fn declared_only(dependencies: Vec<PackageDependency>) -> Self {
        Self { dependencies }
    }

    pub fn declared_dependencies(&self) -> &[PackageDependency] {
        &self.dependencies
    }
}
