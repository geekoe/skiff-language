mod error;
mod io;
mod package_copy_admission;
mod records;
mod strict_json;

pub use error::{EcosystemStorageError, StorageResult};
pub use io::CanonicalArtifactStore;
pub use package_copy_admission::{PackageArtifactAdmissionCache, ValidatedPackageCopyRecords};
pub use records::ResolvedPackageSchema;

mod pointers;

pub use pointers::{
    PackageArtifactPointer, ReleasePointer, ServiceContractPointer, ServiceDeploymentPointer,
};

#[cfg(test)]
mod tests;
