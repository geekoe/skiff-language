mod error;
mod io;
mod package_copy_admission;
mod records;
mod strict_json;

pub use error::{EcosystemStorageError, StorageResult};
pub use io::CanonicalArtifactStore;
pub use package_copy_admission::{PackageArtifactAdmissionCache, ValidatedPackageCopyRecords};
pub use records::ResolvedPackageSchema;

mod activation;
mod pointers;

pub use activation::{
    ActivationRecoveryAction, CommittedActivation, PendingActivation, ProfileActivationState,
    PROFILE_ACTIVATION_STATE_SCHEMA_VERSION,
};
pub use pointers::{
    PackageArtifactPointer, ReleasePointer, RuntimeAssemblyPointer, ServiceContractPointer,
    ServiceDeploymentPointer,
};

#[cfg(test)]
mod tests;
