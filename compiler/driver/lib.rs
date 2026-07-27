pub mod authoring;
mod generated_deployment;
mod http_gateway_projection;
pub(crate) mod input;
pub(crate) mod pipeline;
pub(crate) mod shared;
pub(crate) mod source_compile;
mod websocket_gateway_projection;

pub use generated_deployment::{
    generate_service_deployment, GeneratedServiceDeploymentError, GeneratedServiceDeploymentInput,
};
pub use input::{
    CompilerPlatformSources, CompilerPlatformSourcesError, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageContractCompileDependency, PackageDependency, PackageSourceInput,
    PublicationManifest, PublicationResourceInput, PublicationSourceGraph, SourceTree,
    SourceTreeFile,
};
pub use pipeline::{
    compile_contract, compile_package, compile_service_package, CompiledServicePackage,
    ServicePackageCompileError,
};
pub use shared::package_compile_error::PackageCompileError;

pub use skiff_artifact_model::{
    ContractRequirement, FileIrUnit, PackageArtifact, PackageRequirement, ServiceCallRef,
    ServiceContract, ServiceRequirement,
};
pub use skiff_compiler_contract::{
    definition_contract_operation_id, ContractDefinitionError, ServiceApiFunction,
    ServiceApiFunctionStatus, ServiceApiProjection, ServiceApiVisibility,
    ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};
pub use skiff_compiler_emission::package_artifact::PublishedPackageArtifact;
pub use skiff_compiler_projection_input::{ResolvedPackageSchema, ResolvedPackageSchemaError};
