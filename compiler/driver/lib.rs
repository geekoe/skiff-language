pub mod authoring;
#[doc(hidden)]
pub mod ecosystem_store;
pub(crate) mod input;
pub(crate) mod pipeline;
pub(crate) mod shared;
pub(crate) mod source_compile;

pub use input::{
    ManifestOwner, ManifestProvenance, PackageCompileInput, PackageContractCompileDependency,
    PackageDependency, PackageSourceInput, PublicationManifest, PublicationResourceInput,
    PublicationSourceGraph, SourceTree, SourceTreeFile,
};
pub use pipeline::{compile_contract, compile_package};
pub use shared::package_compile_error::PackageCompileError;

pub use skiff_artifact_model::{
    ContractRequirement, FileIrUnit, PackageArtifact, PackageRequirement, ServiceCallRef,
    ServiceContract, ServiceRequirement,
};
pub use skiff_compiler_contract::{
    definition_contract_operation_id, definition_contract_type_id, definition_contract_type_ref,
    ContractDefinitionError, ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};
pub use skiff_compiler_emission::package_artifact::PublishedPackageArtifact;
