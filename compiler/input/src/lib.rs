pub mod api_spec;
pub mod api_yml;
pub mod compile_policy;
pub mod contract_dependencies;
pub mod dependencies;
pub mod error;
pub mod export_config;
pub mod manifest;
pub mod package_config;
pub mod package_source_helpers;
pub mod package_sources;
pub mod platform_sources;
mod registry_native_sources;
pub mod resources;
pub mod source_tree;
pub mod test_rules;

pub use api_spec::{
    PublicationApiEntry, PublicationApiPublicInstanceEntry, PublicationApiSource,
    PublicationApiSpec, PublicationApiSpecEntry, SourceSymbolSelector,
};
pub use compile_policy::PackageCompilePolicy;
pub use contract_dependencies::{
    read_contract_dependency, read_contract_dependency_json, ContractDependencyError,
    ContractDependencyIndex, ResolvedContractDependency,
};
pub use dependencies::{
    canonical_publication_dependency_id, collect_package_dependency_violations,
    dependency_config_is_empty, empty_dependency_config, is_complex_package_dependency_id,
    is_publication_dependency_id, is_reserved_source_import_alias,
    is_safe_publication_artifact_id_component, is_safe_publication_artifact_path_segment,
    is_standard_package_id, is_valid_source_import_alias, PackageDependency, ResolvedPackage,
    ResolvedPackageGraph,
};
pub use error::InputAssemblyError;
pub use manifest::{
    parse_publication_id_field, validate_publication_version_field, ManifestOwner,
    ManifestProvenance, PublicationManifest,
};
pub use platform_sources::{
    CompilerPlatformPackageAuthority, CompilerPlatformSources, CompilerPlatformSourcesError,
};
pub use registry_native_sources::trusted_registry_native_sources;
pub use resources::{
    collect_publication_resource_spec_violations, read_publication_resources,
    validate_publication_resource_logical_path, MAX_PUBLICATION_RESOURCES,
    MAX_PUBLICATION_RESOURCE_BYTE_LEN, MAX_PUBLICATION_RESOURCE_TOTAL_BYTE_LEN,
};
pub use skiff_compiler_input_model::{
    CompilerRawSourceFile, PublicationResourceInput, PublicationResourceSpec,
    RawPublicationSourceGraph, RawSourceFileMeta, RawSourceTree, RawSourceTreeFile,
};

pub use api_yml::read_publication_api_yml;
