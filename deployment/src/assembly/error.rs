use thiserror::Error;

use skiff_artifact_identity::ArtifactIdentityError;
use skiff_artifact_model::{
    ContractOperationId, GatewayEntryKey, PackageArtifactRef, PackageBuildId, PackageRequirement,
    PackageRequirementKey, ServiceContractRef, ServiceDeploymentRef, ServiceIngressKey,
    ServiceProtocolIdentity, ServiceRequirementKey,
};

/// A closed-world assembly resolution failure. Resolution never guesses a
/// provider or package build when the candidate set is ambiguous or incomplete.
#[derive(Debug, Error)]
pub enum AssemblyResolutionError {
    #[error(transparent)]
    Artifact(#[from] ArtifactIdentityError),

    #[error("root deployment is not present in the candidate set: {0:?}")]
    MissingRoot(ServiceDeploymentRef),

    #[error("service contract is not present in the candidate set: {0:?}")]
    MissingContract(ServiceContractRef),

    #[error(
        "service contract {service_id}@{contract_version} has protocol identities {available:?}, expected {expected}"
    )]
    ContractProtocolMismatch {
        service_id: String,
        contract_version: String,
        expected: ServiceProtocolIdentity,
        available: Vec<ServiceProtocolIdentity>,
    },

    #[error("no in-process deployment provider exists for {0:?}")]
    MissingServiceProvider(ServiceContractRef),

    #[error(
        "deployment providers for {service_id}@{contract_version} use protocol identities {available:?}, expected {expected}"
    )]
    ServiceProviderProtocolMismatch {
        service_id: String,
        contract_version: String,
        expected: ServiceProtocolIdentity,
        available: Vec<ServiceProtocolIdentity>,
    },

    #[error("multiple deployment providers match {contract:?}: {providers:?}")]
    AmbiguousServiceProvider {
        contract: ServiceContractRef,
        providers: Vec<ServiceDeploymentRef>,
    },

    #[error("package artifact is not present in the candidate set: {0:?}")]
    MissingPackageArtifact(PackageArtifactRef),

    #[error("validated package admissions do not exactly match the assembly candidate packages")]
    ValidatedPackageAdmissionMismatch,

    #[error("package build {expected:?} resolves to candidate {available:?}")]
    PackageReferenceMismatch {
        expected: Box<PackageArtifactRef>,
        available: Box<PackageArtifactRef>,
    },

    #[error(
        "candidate package build {build_id} has conflicting references {first:?} and {second:?}"
    )]
    ConflictingCandidatePackageBuild {
        build_id: PackageBuildId,
        first: Box<PackageArtifactRef>,
        second: Box<PackageArtifactRef>,
    },

    #[error(
        "activation {activation:?} resolves package {package_id} to multiple builds {first_build_id} and {second_build_id}"
    )]
    MultiplePackageBuildsForId {
        activation: Box<ServiceDeploymentRef>,
        package_id: String,
        first_build_id: PackageBuildId,
        second_build_id: PackageBuildId,
    },

    #[error("activation {activation:?} has no package binding for {key:?}")]
    MissingPackageBinding {
        activation: Box<ServiceDeploymentRef>,
        key: PackageRequirementKey,
    },

    #[error(
        "activation {activation:?} package binding {key:?} selects {selected:?}, which does not satisfy {requirement:?}"
    )]
    PackageRequirementMismatch {
        activation: Box<ServiceDeploymentRef>,
        key: PackageRequirementKey,
        requirement: Box<PackageRequirement>,
        selected: Box<PackageArtifactRef>,
    },

    #[error(
        "package edge {key:?} resolves activation-independently to both {first:?} and {second:?}"
    )]
    ConflictingPackageLink {
        key: PackageRequirementKey,
        first: Box<PackageArtifactRef>,
        second: Box<PackageArtifactRef>,
    },

    #[error("activation {activation:?} contains an unused package binding {key:?}")]
    UnexpectedPackageBinding {
        activation: Box<ServiceDeploymentRef>,
        key: PackageRequirementKey,
    },

    #[error("activation {activation:?} has no service selector for {key:?}")]
    MissingServiceSelector {
        activation: Box<ServiceDeploymentRef>,
        key: ServiceRequirementKey,
    },

    #[error(
        "activation {activation:?} service selector {key:?} is {selected:?}, expected {expected:?}"
    )]
    ServiceSelectorMismatch {
        activation: Box<ServiceDeploymentRef>,
        key: ServiceRequirementKey,
        expected: Box<ServiceContractRef>,
        selected: Box<ServiceContractRef>,
    },

    #[error("activation {activation:?} contains an unused service selector {key:?}")]
    UnexpectedServiceSelector {
        activation: Box<ServiceDeploymentRef>,
        key: ServiceRequirementKey,
    },

    #[error(
        "activation {activation:?} binding {key:?} uses operation {operation}, which is absent from {contract:?}"
    )]
    MissingServiceOperation {
        activation: Box<ServiceDeploymentRef>,
        key: ServiceRequirementKey,
        contract: Box<ServiceContractRef>,
        operation: ContractOperationId,
    },

    #[error("activation {activation:?} references missing gateway entry {gateway_entry_key}")]
    MissingGatewayEntry {
        activation: ServiceDeploymentRef,
        gateway_entry_key: GatewayEntryKey,
    },

    #[error("gateway ingress key {key:?} is declared by both {first:?} and {second:?}")]
    GatewayIngressCollision {
        key: Box<ServiceIngressKey>,
        first: Box<ServiceDeploymentRef>,
        second: Box<ServiceDeploymentRef>,
    },
}

pub type AssemblyResult<T> = std::result::Result<T, AssemblyResolutionError>;
