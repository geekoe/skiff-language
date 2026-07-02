pub mod api_spec;
pub mod api_yml;
pub mod compile_policy;
pub mod dependencies;
pub mod error;
pub mod export_config;
pub mod input;
pub mod manifest;
pub mod package_config;
pub mod package_job;
pub mod package_source_helpers;
pub mod package_sources;
pub mod publication;
pub mod raw_sources;
pub mod registry_helpers;
pub mod service_config;
pub mod service_dependencies;
pub mod service_ingress;
pub mod service_job;
pub mod service_packages;
pub mod source_tree;
pub mod test_rules;

pub use api_spec::{
    PublicationApiEntry, PublicationApiPublicInstanceEntry, PublicationApiSource,
    PublicationApiSpec, PublicationApiSpecEntry, SourceSymbolSelector,
};
pub use compile_policy::PublicationCompilePolicy;
pub use dependencies::{
    canonical_publication_dependency_id, collect_package_dependency_violations,
    dependency_config_is_empty, empty_dependency_config, is_complex_package_dependency_id,
    is_publication_dependency_id, is_reserved_source_import_alias,
    is_safe_publication_artifact_id_component, is_safe_publication_artifact_path_segment,
    is_standard_package_id, is_valid_source_import_alias, PackageDependency, ResolvedPackage,
    ResolvedPackageGraph, ResolvedServiceDependencies, ServiceDependency,
    ServiceDependencyLockEntry, ServiceDependencyRemoteBoxProvenance,
};
pub use error::InputAssemblyError;
pub use input::{
    classify_publication_root, PublicationInputError, PublicationInputKind, PublicationRootManifest,
};
pub use manifest::{
    parse_publication_id_field, validate_publication_version_field, ManifestOwner,
    ManifestProvenance, PublicationManifest,
};
pub use package_job::{build_package_jobs, RawPackagePublicationJob};
pub use publication::{assemble_publication, RawPublication};
pub use raw_sources::read_publication_sources;
pub use service_ingress::{
    ServiceHttpIngressSeed, ServiceHttpRouteIngressSeed, ServiceIngressSeed,
    ServiceWebSocketIngressSeed,
};
pub use service_job::{build_service_job, RawServicePublicationJob, ServiceJobSeeds};
pub use service_packages::{
    PackageManifestDiscoveryResult, ResolvedServicePackages, ServiceSourcePackageFacts,
};
pub use skiff_compiler_input_model::{
    CompilerRawSourceFile, RawPublicationSourceGraph, RawSourceFileMeta, RawSourceOrigin,
    RawSourceTree, RawSourceTreeFile,
};

pub use api_yml::read_publication_api_yml;
