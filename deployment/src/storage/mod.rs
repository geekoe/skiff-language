mod error;
mod io;
mod records;
mod strict_json;

pub use error::{EcosystemStorageError, StorageResult};
pub use io::CanonicalArtifactStore;

mod activation;
mod pointers;

pub use activation::{
    ActivationRecoveryAction, CommittedActivation, EnvironmentActivationState, PendingActivation,
    ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
};
pub use pointers::{
    PackageArtifactPointer, RuntimeAssemblyPointer, ServiceContractPointer,
    ServiceDeploymentPointer,
};

#[cfg(test)]
mod tests;
