use thiserror::Error;

use skiff_artifact_identity::ArtifactIdentityError;
use skiff_artifact_model::{
    ContractOperationId, GatewayEntryKey, PackageArtifactRef, PackageBuildId, PackageRequirement,
    PackageRequirementKey, ServiceContractRef, ServiceDeploymentRef, ServiceProtocolIdentity,
    ServiceRequirementKey,
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

    #[error("package build {expected:?} resolves to candidate {available:?}")]
    PackageReferenceMismatch {
        expected: PackageArtifactRef,
        available: PackageArtifactRef,
    },

    #[error(
        "candidate package build {build_id} has conflicting references {first:?} and {second:?}"
    )]
    ConflictingCandidatePackageBuild {
        build_id: PackageBuildId,
        first: PackageArtifactRef,
        second: PackageArtifactRef,
    },

    #[error("activation {activation:?} has no package binding for {key:?}")]
    MissingPackageBinding {
        activation: ServiceDeploymentRef,
        key: PackageRequirementKey,
    },

    #[error(
        "activation {activation:?} package binding {key:?} selects {selected:?}, which does not satisfy {requirement:?}"
    )]
    PackageRequirementMismatch {
        activation: ServiceDeploymentRef,
        key: PackageRequirementKey,
        requirement: PackageRequirement,
        selected: PackageArtifactRef,
    },

    #[error(
        "package edge {key:?} resolves activation-independently to both {first:?} and {second:?}"
    )]
    ConflictingPackageLink {
        key: PackageRequirementKey,
        first: PackageArtifactRef,
        second: PackageArtifactRef,
    },

    #[error("activation {activation:?} contains an unused package binding {key:?}")]
    UnexpectedPackageBinding {
        activation: ServiceDeploymentRef,
        key: PackageRequirementKey,
    },

    #[error("activation {activation:?} has no service selector for {key:?}")]
    MissingServiceSelector {
        activation: ServiceDeploymentRef,
        key: ServiceRequirementKey,
    },

    #[error(
        "activation {activation:?} service selector {key:?} is {selected:?}, expected {expected:?}"
    )]
    ServiceSelectorMismatch {
        activation: ServiceDeploymentRef,
        key: ServiceRequirementKey,
        expected: ServiceContractRef,
        selected: ServiceContractRef,
    },

    #[error("activation {activation:?} contains an unused service selector {key:?}")]
    UnexpectedServiceSelector {
        activation: ServiceDeploymentRef,
        key: ServiceRequirementKey,
    },

    #[error(
        "activation {activation:?} binding {key:?} uses operation {operation}, which is absent from {contract:?}"
    )]
    MissingServiceOperation {
        activation: ServiceDeploymentRef,
        key: ServiceRequirementKey,
        contract: ServiceContractRef,
        operation: ContractOperationId,
    },

    #[error(
        "activation {activation:?} has deployment gateway ingress {gateway_entry_keys:?}, but RuntimeAssembly gateway entries are not yet linked"
    )]
    GatewayIngressNotLinked {
        activation: ServiceDeploymentRef,
        gateway_entry_keys: Vec<GatewayEntryKey>,
    },
}

pub type AssemblyResult<T> = std::result::Result<T, AssemblyResolutionError>;
