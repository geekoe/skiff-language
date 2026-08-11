pub mod authoring;
mod generated_deployment;
mod http_gateway_projection;
pub(crate) mod input;
pub(crate) mod pipeline;
#[doc(hidden)]
pub mod platform_error_projection_codegen;
pub(crate) mod shared;
pub(crate) mod source_compile;
mod websocket_gateway_projection;

pub use generated_deployment::{
    generate_service_deployment, generate_service_deployment_with_validated_packages,
    GeneratedServiceDeploymentError, GeneratedServiceDeploymentInput,
    GeneratedServicePackageAdmissions,
};
pub use input::{
    CompilerPlatformSources, CompilerPlatformSourcesError, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageContractCompileDependency, PackageDependency, PackageSourceInput,
    PublicationManifest, PublicationResourceInput, PublicationSourceGraph, SourceTree,
    SourceTreeFile,
};
pub use pipeline::{
    compile_contract, compile_package, compile_service_package, CompiledServicePackage,
    PackageBytecodeLane, PackageCompileOutput, ServicePackageCompileError,
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
