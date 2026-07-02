pub(crate) mod r#compile_input;
pub(crate) mod package_job;
pub(crate) mod package_path;
pub(crate) mod service_dependencies;
pub(crate) mod service_job;
pub(crate) mod service_packages;
pub(crate) mod source_graph;
pub(crate) mod test_rules;

pub(crate) use package_job::PackagePublicationJob;
pub(crate) use service_job::{build_service_job, ServiceJobSeeds, ServicePublicationJob};
pub use skiff_compiler_input::api_spec::{PublicationApiEntry, PublicationApiSource};
#[cfg(test)]
pub(crate) use skiff_compiler_input::empty_dependency_config;
pub use skiff_compiler_input::package_config::{
    discover_package_manifests, discover_package_manifests_with_dependency_dirs,
    discover_package_manifests_with_dirs, is_complex_package_id, package_alias_bindings,
    read_user_package_manifest, resolve_package_imports, PackageApi, PackageApiEntry,
    PackageConfigError, PackageManifest, PackageManifestKey, PackageResolutionDirs,
    PACKAGE_CONFIG_FILE,
};
pub use skiff_compiler_input::service_config::{
    ServiceAccessConfig, ServiceConfig, ServiceOrganizationRole, ServiceVisibility, TimeoutConfig,
};
pub(crate) use skiff_compiler_input::service_ingress::ServiceIngressSeed;
pub use skiff_compiler_input::source_tree::{SourceTree, SourceTreeFile};
pub use skiff_compiler_input::{
    ManifestOwner, ManifestProvenance, PackageDependency, PublicationCompilePolicy,
    PublicationManifest, ServiceDependency,
};
pub(crate) use skiff_compiler_input::{
    ResolvedPackage, ResolvedPackageGraph, ResolvedServiceDependencies,
};
pub(crate) use source_graph::Publication;

#[cfg(test)]
pub(crate) fn assemble_publication(
    manifest: PublicationManifest,
    source_tree: SourceTree,
    source_graph: source_graph::PublicationSourceGraph,
    package_graph: ResolvedPackageGraph,
) -> Publication {
    source_graph::assemble_publication(manifest, source_tree, source_graph, package_graph)
}
