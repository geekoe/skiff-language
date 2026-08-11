use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use skiff_artifact_identity::{
    PackageArtifactPointerPath, PackageArtifactRecordPath, ReleasePointerPath, ServiceContractPointerPath,
    ServiceContractRecordPath, ServiceDeploymentPointerPath, ServiceDeploymentRecordPath,
};
use skiff_artifact_model::{PackageArtifactRef, ServiceContractRef, ServiceDeploymentRef};

use super::{
    error::{io_error, EcosystemStorageError, StorageResult},
    io::{
        canonical_bytes, read_locked_bytes, strict_value, typed_from_value, CanonicalArtifactStore,
    },
};

const PACKAGE_POINTER_SCHEMA: &str = "skiff-package-artifact-pointer-v1";
const CONTRACT_POINTER_SCHEMA: &str = "skiff-service-contract-pointer-v1";
const DEPLOYMENT_POINTER_SCHEMA: &str = "skiff-service-deployment-pointer-v1";
const RELEASE_POINTER_SCHEMA: &str = "skiff-release-pointer-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageArtifactPointer {
    pub schema_version: String,
    pub artifact: PackageArtifactRef,
    pub record_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractPointer {
    pub schema_version: String,
    pub contract: ServiceContractRef,
    pub record_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentPointer {
    pub schema_version: String,
    pub deployment: ServiceDeploymentRef,
    pub record_path: String,
}
/// The release pointer table entry `(profile, serviceId, version) -> buildId`.
///
/// The value carries the complete `ServiceDeploymentRef` (whose
/// `deployment_artifact_identity` is the buildId consumed as the runtime
/// loading unit) so the runtime can locate the exact deployment record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleasePointer {
    pub schema_version: String,
    pub profile: String,
    pub deployment: ServiceDeploymentRef,
    pub record_path: String,
}

impl PackageArtifactPointer {
    pub fn new(artifact: PackageArtifactRef) -> StorageResult<Self> {
        let record_path = PackageArtifactRecordPath::new(&artifact)?.to_string();
        Ok(Self {
            schema_version: PACKAGE_POINTER_SCHEMA.to_string(),
            artifact,
            record_path,
        })
    }

    fn validate(&self, path: &Path) -> StorageResult<()> {
        if self.schema_version != PACKAGE_POINTER_SCHEMA
            || self.record_path != PackageArtifactRecordPath::new(&self.artifact)?.as_str()
        {
            return invalid(path, "package pointer schema or recordPath mismatch");
        }
        Ok(())
    }
}

impl ServiceContractPointer {
    pub fn new(contract: ServiceContractRef) -> StorageResult<Self> {
        let record_path = ServiceContractRecordPath::new(&contract)?.to_string();
        Ok(Self {
            schema_version: CONTRACT_POINTER_SCHEMA.to_string(),
            contract,
            record_path,
        })
    }

    fn validate(&self, path: &Path) -> StorageResult<()> {
        if self.schema_version != CONTRACT_POINTER_SCHEMA
            || self.record_path != ServiceContractRecordPath::new(&self.contract)?.as_str()
        {
            return invalid(path, "contract pointer schema or recordPath mismatch");
        }
        Ok(())
    }
}

impl ServiceDeploymentPointer {
    pub fn new(deployment: ServiceDeploymentRef) -> StorageResult<Self> {
        let record_path = ServiceDeploymentRecordPath::new(&deployment)?.to_string();
        Ok(Self {
            schema_version: DEPLOYMENT_POINTER_SCHEMA.to_string(),
            deployment,
            record_path,
        })
    }

    fn validate(&self, path: &Path) -> StorageResult<()> {
        if self.schema_version != DEPLOYMENT_POINTER_SCHEMA
            || self.record_path != ServiceDeploymentRecordPath::new(&self.deployment)?.as_str()
        {
            return invalid(path, "deployment pointer schema or recordPath mismatch");
        }
        Ok(())
    }
}
impl ReleasePointer {
    pub fn new(
        profile: impl Into<String>,
        deployment: ServiceDeploymentRef,
    ) -> StorageResult<Self> {
        let profile = profile.into();
        ReleasePointerPath::new(
            &profile,
            &deployment.service_id,
            &deployment.contract_version,
        )?;
        let record_path = ServiceDeploymentRecordPath::new(&deployment)?.to_string();
        Ok(Self {
            schema_version: RELEASE_POINTER_SCHEMA.to_string(),
            profile,
            deployment,
            record_path,
        })
    }

    fn validate(&self, path: &Path) -> StorageResult<()> {
        ReleasePointerPath::new(
            &self.profile,
            &self.deployment.service_id,
            &self.deployment.contract_version,
        )?;
        if self.schema_version != RELEASE_POINTER_SCHEMA
            || self.record_path != ServiceDeploymentRecordPath::new(&self.deployment)?.as_str()
        {
            return invalid(path, "release pointer schema or recordPath mismatch");
        }
        Ok(())
    }
}

impl CanonicalArtifactStore {
    pub fn read_package_artifact_pointer(
        &self,
        package_id: &str,
        package_version: &str,
    ) -> StorageResult<Option<PackageArtifactPointer>> {
        let path = PackageArtifactPointerPath::new(package_id, package_version)?;
        let pointer = read_pointer(
            self,
            path.as_relative_path(),
            PackageArtifactPointer::validate,
        )?;
        if let Some(pointer) = &pointer {
            self.read_package_artifact(&pointer.artifact)?;
        }
        Ok(pointer)
    }

    pub fn compare_and_swap_package_artifact_pointer(
        &self,
        expected: Option<&PackageArtifactPointer>,
        candidate: &PackageArtifactPointer,
    ) -> StorageResult<()> {
        candidate.validate(self.root())?;
        self.read_package_artifact(&candidate.artifact)?;
        let path = PackageArtifactPointerPath::new(
            &candidate.artifact.package_id,
            &candidate.artifact.package_version,
        )?;
        cas_pointer(
            self,
            path.as_relative_path(),
            expected,
            candidate,
            PackageArtifactPointer::validate,
        )
    }

    pub fn read_service_contract_pointer(
        &self,
        service_id: &str,
        contract_version: &str,
    ) -> StorageResult<Option<ServiceContractPointer>> {
        let path = ServiceContractPointerPath::new(service_id, contract_version)?;
        let pointer = read_pointer(
            self,
            path.as_relative_path(),
            ServiceContractPointer::validate,
        )?;
        if let Some(pointer) = &pointer {
            self.read_service_contract(&pointer.contract)?;
        }
        Ok(pointer)
    }

    pub fn compare_and_swap_service_contract_pointer(
        &self,
        expected: Option<&ServiceContractPointer>,
        candidate: &ServiceContractPointer,
    ) -> StorageResult<()> {
        candidate.validate(self.root())?;
        self.read_service_contract(&candidate.contract)?;
        let path = ServiceContractPointerPath::new(
            &candidate.contract.service_id,
            &candidate.contract.contract_version,
        )?;
        cas_pointer(
            self,
            path.as_relative_path(),
            expected,
            candidate,
            ServiceContractPointer::validate,
        )
    }

    pub fn read_service_deployment_pointer(
        &self,
        service_id: &str,
        contract_version: &str,
    ) -> StorageResult<Option<ServiceDeploymentPointer>> {
        let path = ServiceDeploymentPointerPath::new(service_id, contract_version)?;
        let pointer = read_pointer(
            self,
            path.as_relative_path(),
            ServiceDeploymentPointer::validate,
        )?;
        if let Some(pointer) = &pointer {
            self.read_service_deployment(&pointer.deployment)?;
        }
        Ok(pointer)
    }

    pub fn compare_and_swap_service_deployment_pointer(
        &self,
        expected: Option<&ServiceDeploymentPointer>,
        candidate: &ServiceDeploymentPointer,
    ) -> StorageResult<()> {
        candidate.validate(self.root())?;
        self.read_service_deployment(&candidate.deployment)?;
        let path = ServiceDeploymentPointerPath::new(
            &candidate.deployment.service_id,
            &candidate.deployment.contract_version,
        )?;
        cas_pointer(
            self,
            path.as_relative_path(),
            expected,
            candidate,
            ServiceDeploymentPointer::validate,
        )
    }

    pub fn read_release_pointer(
        &self,
        profile: &str,
        service_id: &str,
        version: &str,
    ) -> StorageResult<Option<ReleasePointer>> {
        let path = ReleasePointerPath::new(profile, service_id, version)?;
        let pointer = read_pointer(self, path.as_relative_path(), ReleasePointer::validate)?;
        if let Some(pointer) = &pointer {
            self.read_service_deployment(&pointer.deployment)?;
        }
        Ok(pointer)
    }

    /// Atomically replaces the release pointer without any expectation check.
    /// The candidate's target deployment record must already exist.
    pub fn write_release_pointer(&self, candidate: &ReleasePointer) -> StorageResult<()> {
        candidate.validate(self.root())?;
        self.read_service_deployment(&candidate.deployment)?;
        let path = ReleasePointerPath::new(
            &candidate.profile,
            &candidate.deployment.service_id,
            &candidate.deployment.contract_version,
        )?;
        self.with_exclusive_pointer_lock(path.as_relative_path(), |destination| {
            self.replace_locked(destination, &canonical_bytes(candidate)?)
        })
    }

    pub fn compare_and_swap_release_pointer(
        &self,
        expected: Option<&ReleasePointer>,
        candidate: &ReleasePointer,
    ) -> StorageResult<()> {
        candidate.validate(self.root())?;
        self.read_service_deployment(&candidate.deployment)?;
        let path = ReleasePointerPath::new(
            &candidate.profile,
            &candidate.deployment.service_id,
            &candidate.deployment.contract_version,
        )?;
        cas_pointer(
            self,
            path.as_relative_path(),
            expected,
            candidate,
            ReleasePointer::validate,
        )
    }

    /// Removes the release pointer under the exclusive lock. `expected` is an
    /// optional CAS guard on the current value. Removing an absent pointer is
    /// idempotent and returns `None`.
    pub fn unset_release_pointer(
        &self,
        profile: &str,
        service_id: &str,
        version: &str,
        expected: Option<&ReleasePointer>,
    ) -> StorageResult<Option<ReleasePointer>> {
        let path = ReleasePointerPath::new(profile, service_id, version)?;
        self.with_exclusive_pointer_lock(path.as_relative_path(), |destination| {
            let current = read_locked_bytes(destination)?
                .map(|bytes| parse_pointer(destination, &bytes, ReleasePointer::validate))
                .transpose()?;
            if let Some(expected) = expected {
                if current.as_ref() != Some(expected) {
                    return Err(EcosystemStorageError::CasMismatch {
                        path: destination.to_path_buf(),
                        message: "current pointer does not equal expected pointer".to_string(),
                    });
                }
            }
            if current.is_some() {
                fs::remove_file(destination)
                    .map_err(|source| io_error("remove release pointer", destination, source))?;
                sync_directory(
                    destination
                        .parent()
                        .expect("pointer destinations always have a parent"),
                )?;
            }
            Ok(current)
        })
    }
}

fn read_pointer<T: DeserializeOwned + Serialize>(
    store: &CanonicalArtifactStore,
    path: &skiff_artifact_identity::ArtifactRelativePath,
    validate: fn(&T, &Path) -> StorageResult<()>,
) -> StorageResult<Option<T>> {
    let Some(bytes) = store.read_optional_bytes(path)? else {
        return Ok(None);
    };
    let host_path = store.root().join(path.as_path());
    parse_pointer(&host_path, &bytes, validate).map(Some)
}

fn cas_pointer<T>(
    store: &CanonicalArtifactStore,
    path: &skiff_artifact_identity::ArtifactRelativePath,
    expected: Option<&T>,
    candidate: &T,
    validate: fn(&T, &Path) -> StorageResult<()>,
) -> StorageResult<()>
where
    T: DeserializeOwned + Serialize + PartialEq,
{
    store.with_exclusive_pointer_lock(path, |destination| {
        let current = read_locked_bytes(destination)?
            .map(|bytes| parse_pointer(destination, &bytes, validate))
            .transpose()?;
        if current.as_ref() != expected {
            return Err(EcosystemStorageError::CasMismatch {
                path: destination.to_path_buf(),
                message: "current pointer does not equal expected pointer".to_string(),
            });
        }
        validate(candidate, destination)?;
        store.replace_locked(destination, &canonical_bytes(candidate)?)
    })
}

fn parse_pointer<T>(
    path: &Path,
    bytes: &[u8],
    validate: fn(&T, &Path) -> StorageResult<()>,
) -> StorageResult<T>
where
    T: DeserializeOwned + Serialize,
{
    let value = strict_value(path, bytes)?;
    let pointer = typed_from_value(path, value)?;
    validate(&pointer, path)?;
    if canonical_bytes(&pointer)? != bytes {
        return invalid(path, "pointer bytes are not canonical JSON");
    }
    Ok(pointer)
}

fn invalid<T>(path: &Path, message: impl Into<String>) -> StorageResult<T> {
    Err(EcosystemStorageError::InvalidRecord {
        path: PathBuf::from(path),
        message: message.into(),
    })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> StorageResult<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync pointer directory", path, source))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> StorageResult<()> {
    Ok(())
}
