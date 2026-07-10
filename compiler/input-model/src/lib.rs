pub mod compile_input;
pub mod compile_policy;
pub mod dependencies;
pub mod manifest;
pub mod raw_source;
pub mod resources;
pub mod service_ingress;

pub use compile_input::{
    PackagePublicationInput, PublicationInput, PublicationInputCore, PublicationInputMetadata,
    ServicePublicationInput,
};
pub use compile_policy::PublicationCompilePolicy;
pub use dependencies::{
    canonical_publication_dependency_id, collect_package_dependency_violations,
    dependency_config_is_empty, empty_dependency_config, is_complex_package_dependency_id,
    is_publication_dependency_id, is_reserved_source_import_alias,
    is_safe_publication_artifact_id_component, is_safe_publication_artifact_path_segment,
    is_standard_package_id, is_valid_source_import_alias, PackageDependency,
    ResolvedServiceDependencies, ServiceDependency, ServiceDependencyLockEntry,
    ServiceDependencyRemoteBoxProvenance,
};
pub use manifest::{
    parse_publication_id_field, validate_publication_version_field, ManifestOwner,
    ManifestProvenance, PublicationManifest,
};
pub use raw_source::{
    CompilerRawSourceFile, RawPublicationSourceGraph, RawSourceFileMeta, RawSourceOrigin,
    RawSourceTree, RawSourceTreeFile,
};
pub use resources::{PublicationResourceInput, PublicationResourceSpec};
pub use service_ingress::{
    ServiceHttpIngressSeed, ServiceHttpRouteIngressSeed, ServiceIngressSeed,
    ServiceWebSocketIngressSeed,
};

pub use skiff_compiler_core::api_spec::{
    PublicationApiEntry, PublicationApiPublicInstanceEntry, PublicationApiSource,
    PublicationApiSpec, PublicationApiSpecEntry, SourceSymbolSelector,
};
pub use skiff_compiler_core::export_config;
pub use skiff_compiler_core::id::{PublicationId, PublicationIdError};
pub use skiff_compiler_core::source_role::PublicationSourceRole as CompilerSourceRole;
