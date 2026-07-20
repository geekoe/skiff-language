use std::{path::PathBuf, sync::Arc};

use serde_json::Value;
use skiff_artifact_identity::{
    package_artifact_ref, runtime_assembly_ref, service_contract_ref, service_deployment_ref,
    validate_file_ir_identity, validate_package_artifact_identities,
    validate_runtime_assembly_identity, validate_service_contract_identities,
    validate_service_deployment_ref, PackageArtifactRecordPath, PackageFileIrRecordPath,
    PackageResourceRecordPath, RuntimeAssemblyRecordPath, ServiceContractRecordPath,
    ServiceDeploymentRecordPath,
};
use skiff_artifact_model::{
    FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef, PublicationResourceRef,
    RuntimeAssembly, RuntimeAssemblyRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef,
};

use super::{
    error::{EcosystemStorageError, StorageResult},
    io::{canonical_bytes, strict_value, typed_from_value, CanonicalArtifactStore},
};

impl CanonicalArtifactStore {
    pub fn write_package_artifact(&self, artifact: &PackageArtifact) -> StorageResult<PathBuf> {
        validate_package_artifact_identities(artifact)?;
        let reference = package_artifact_ref(artifact)?;
        let path = PackageArtifactRecordPath::new(&reference)?;
        self.write_immutable(path.as_relative_path(), &canonical_bytes(artifact)?)
    }

    pub fn read_package_artifact(
        &self,
        reference: &PackageArtifactRef,
    ) -> StorageResult<Arc<PackageArtifact>> {
        let path = PackageArtifactRecordPath::new(reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_package_ref(&host_path, &value, reference)?;
        let artifact = typed_from_value::<PackageArtifact>(&host_path, value)?;
        validate_package_artifact_identities(&artifact)?;
        if &package_artifact_ref(&artifact)? != reference {
            return invalid(
                &host_path,
                "typed PackageArtifact does not match exact reference",
            );
        }
        ensure_canonical(&host_path, &bytes, &artifact)?;
        Ok(Arc::new(artifact))
    }

    pub fn write_service_contract(&self, contract: &ServiceContract) -> StorageResult<PathBuf> {
        validate_service_contract_identities(contract)?;
        let reference = service_contract_ref(contract)?;
        let path = ServiceContractRecordPath::new(&reference)?;
        self.write_immutable(path.as_relative_path(), &canonical_bytes(contract)?)
    }

    pub fn read_service_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> StorageResult<Arc<ServiceContract>> {
        let path = ServiceContractRecordPath::new(reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_contract_ref(&host_path, &value, reference)?;
        let contract = typed_from_value::<ServiceContract>(&host_path, value)?;
        validate_service_contract_identities(&contract)?;
        if &service_contract_ref(&contract)? != reference {
            return invalid(
                &host_path,
                "typed ServiceContract does not match exact reference",
            );
        }
        ensure_canonical(&host_path, &bytes, &contract)?;
        Ok(Arc::new(contract))
    }

    pub fn write_service_deployment(
        &self,
        deployment: &ServiceDeployment,
    ) -> StorageResult<PathBuf> {
        let reference = service_deployment_ref(deployment);
        validate_service_deployment_ref(&reference, deployment)?;
        let path = ServiceDeploymentRecordPath::new(&reference)?;
        self.write_immutable(path.as_relative_path(), &canonical_bytes(deployment)?)
    }

    pub fn read_service_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> StorageResult<Arc<ServiceDeployment>> {
        let path = ServiceDeploymentRecordPath::new(reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_deployment_ref(&host_path, &value, reference)?;
        let deployment = typed_from_value::<ServiceDeployment>(&host_path, value)?;
        validate_service_deployment_ref(reference, &deployment)?;
        ensure_canonical(&host_path, &bytes, &deployment)?;
        Ok(Arc::new(deployment))
    }

    pub fn write_runtime_assembly(&self, assembly: &RuntimeAssembly) -> StorageResult<PathBuf> {
        validate_runtime_assembly_identity(assembly)?;
        let reference = runtime_assembly_ref(assembly)?;
        let path = RuntimeAssemblyRecordPath::new(&reference)?;
        self.write_immutable(path.as_relative_path(), &canonical_bytes(assembly)?)
    }

    pub fn read_runtime_assembly(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> StorageResult<Arc<RuntimeAssembly>> {
        let path = RuntimeAssemblyRecordPath::new(reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_string(
            &host_path,
            &value,
            &["assemblyIdentity"],
            reference.assembly_identity.as_str(),
        )?;
        let assembly = typed_from_value::<RuntimeAssembly>(&host_path, value)?;
        validate_runtime_assembly_identity(&assembly)?;
        if &runtime_assembly_ref(&assembly)? != reference {
            return invalid(
                &host_path,
                "typed RuntimeAssembly does not match exact reference",
            );
        }
        ensure_canonical(&host_path, &bytes, &assembly)?;
        Ok(Arc::new(assembly))
    }

    pub fn write_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
        file: &FileIrUnit,
    ) -> StorageResult<PathBuf> {
        validate_file_ir_identity(file)?;
        validate_file_ref(&self.root().to_path_buf(), package, reference, file)?;
        let path = PackageFileIrRecordPath::new(package, reference)?;
        self.write_immutable(path.as_relative_path(), &canonical_bytes(file)?)
    }

    pub fn read_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> StorageResult<Arc<FileIrUnit>> {
        let path = PackageFileIrRecordPath::new(package, reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_string(
            &host_path,
            &value,
            &["fileIrIdentity"],
            &reference.file_ir_identity,
        )?;
        raw_string(&host_path, &value, &["modulePath"], &reference.module_path)?;
        let file = typed_from_value::<FileIrUnit>(&host_path, value)?;
        validate_file_ref(&host_path, package, reference, &file)?;
        ensure_canonical(&host_path, &bytes, &file)?;
        Ok(Arc::new(file))
    }

    pub fn write_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
        bytes: &[u8],
    ) -> StorageResult<PathBuf> {
        validate_resource(&self.root().to_path_buf(), reference, bytes)?;
        let path = PackageResourceRecordPath::new(package, reference)?;
        self.write_immutable(path.as_relative_path(), bytes)
    }

    pub fn read_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> StorageResult<Arc<[u8]>> {
        let path = PackageResourceRecordPath::new(package, reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        validate_resource(&host_path, reference, &bytes)?;
        Ok(Arc::from(bytes))
    }
}

fn raw_package_ref(
    path: &std::path::Path,
    value: &Value,
    reference: &PackageArtifactRef,
) -> StorageResult<()> {
    raw_string(path, value, &["packageId"], &reference.package_id)?;
    raw_string(path, value, &["packageVersion"], &reference.package_version)?;
    raw_string(
        path,
        value,
        &["packageBuildId"],
        reference.package_build_id.as_str(),
    )?;
    raw_string(
        path,
        value,
        &["packageLocalAbi", "localAbiIdentity"],
        reference.package_local_abi_identity.as_str(),
    )
}

fn raw_contract_ref(
    path: &std::path::Path,
    value: &Value,
    reference: &ServiceContractRef,
) -> StorageResult<()> {
    raw_string(path, value, &["serviceId"], &reference.service_id)?;
    raw_string(
        path,
        value,
        &["contractVersion"],
        &reference.contract_version,
    )?;
    raw_string(
        path,
        value,
        &["serviceProtocolIdentity"],
        reference.service_protocol_identity.as_str(),
    )
}

fn raw_deployment_ref(
    path: &std::path::Path,
    value: &Value,
    reference: &ServiceDeploymentRef,
) -> StorageResult<()> {
    raw_string(
        path,
        value,
        &["contract", "serviceId"],
        &reference.service_id,
    )?;
    raw_string(
        path,
        value,
        &["contract", "contractVersion"],
        &reference.contract_version,
    )?;
    raw_string(
        path,
        value,
        &["deploymentRevision"],
        reference.deployment_revision.as_str(),
    )?;
    raw_string(
        path,
        value,
        &["deploymentArtifactIdentity"],
        reference.deployment_artifact_identity.as_str(),
    )
}

fn raw_string(
    path: &std::path::Path,
    value: &Value,
    fields: &[&str],
    expected: &str,
) -> StorageResult<()> {
    let mut current = value;
    for field in fields {
        current = current
            .get(*field)
            .ok_or_else(|| EcosystemStorageError::InvalidRecord {
                path: path.to_path_buf(),
                message: format!("raw JSON is missing {}", fields.join(".")),
            })?;
    }
    if current.as_str() != Some(expected) {
        return invalid(
            path,
            format!(
                "raw JSON {} does not match exact reference value {expected}",
                fields.join(".")
            ),
        );
    }
    Ok(())
}

fn validate_file_ref(
    path: &std::path::Path,
    package: &PackageArtifactRef,
    reference: &FileIrRef,
    file: &FileIrUnit,
) -> StorageResult<()> {
    validate_file_ir_identity(file)?;
    if file.file_ir_identity != reference.file_ir_identity
        || file.module_path != reference.module_path
        || reference
            .source_ast_hash
            .as_ref()
            .is_some_and(|hash| hash != &file.source_ast_hash)
    {
        return invalid(
            path,
            format!(
                "File IR content does not match package {} exact reference",
                package.package_build_id
            ),
        );
    }
    Ok(())
}

fn validate_resource(
    path: &std::path::Path,
    reference: &PublicationResourceRef,
    bytes: &[u8],
) -> StorageResult<()> {
    use sha2::{Digest, Sha256};
    let actual_len =
        u64::try_from(bytes.len()).map_err(|_| EcosystemStorageError::InvalidRecord {
            path: path.to_path_buf(),
            message: "resource byte length does not fit u64".to_string(),
        })?;
    let actual_hash = hex::encode(Sha256::digest(bytes));
    if actual_len != reference.byte_len || actual_hash != reference.sha256 {
        return invalid(path, "static resource hash or byte length mismatch");
    }
    Ok(())
}

fn invalid<T>(path: &std::path::Path, message: impl Into<String>) -> StorageResult<T> {
    Err(EcosystemStorageError::InvalidRecord {
        path: path.to_path_buf(),
        message: message.into(),
    })
}

fn ensure_canonical<T: serde::Serialize>(
    path: &std::path::Path,
    bytes: &[u8],
    value: &T,
) -> StorageResult<()> {
    if canonical_bytes(value)? != bytes {
        return invalid(path, "record bytes are not canonical JSON");
    }
    Ok(())
}
