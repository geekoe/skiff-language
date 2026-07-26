use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use serde_json::Value;
use skiff_artifact_identity::{
    package_artifact_ref, package_schema_type_id, runtime_assembly_ref, service_contract_ref,
    service_deployment_ref, validate_file_ir_identity, validate_package_artifact_identities,
    validate_package_schema_index, validate_package_schema_records,
    validate_runtime_assembly_identity, validate_service_contract_identities,
    validate_service_deployment_ref, PackageArtifactRecordPath, PackageFileIrRecordPath,
    PackageResourceRecordPath, PackageSchemaIndexRecordPath, PackageSchemaTypeRecordPath,
    RuntimeAssemblyRecordPath, ServiceContractRecordPath, ServiceDeploymentRecordPath,
};
use skiff_artifact_model::{
    package_schema_descriptor_refs, FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef,
    PackageSchemaIndex, PackageSchemaIndexRef, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageSchemaTypeRecordRef, PublicationResourceRef, RuntimeAssembly, RuntimeAssemblyRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};

use super::{
    error::{EcosystemStorageError, StorageResult},
    io::{canonical_bytes, strict_value, typed_from_value, CanonicalArtifactStore},
};

#[derive(Debug, Clone)]
pub struct ResolvedPackageSchema {
    pub index: Arc<PackageSchemaIndex>,
    pub records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

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

    pub fn write_package_schema_index(&self, index: &PackageSchemaIndex) -> StorageResult<PathBuf> {
        validate_package_schema_index(index)?;
        let reference = PackageSchemaIndexRef {
            package_id: index.package_id.clone(),
            package_schema_index_identity: index.package_schema_index_identity.clone(),
        };
        let path = PackageSchemaIndexRecordPath::new(&reference)?;
        self.write_immutable(path.as_relative_path(), &canonical_bytes(index)?)
    }

    pub fn read_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> StorageResult<Arc<PackageSchemaIndex>> {
        let path = PackageSchemaIndexRecordPath::new(reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_string(&host_path, &value, &["packageId"], &reference.package_id)?;
        raw_string(
            &host_path,
            &value,
            &["packageSchemaIndexIdentity"],
            reference.package_schema_index_identity.as_str(),
        )?;
        let index = typed_from_value::<PackageSchemaIndex>(&host_path, value)?;
        validate_package_schema_index(&index)?;
        if index.package_id != reference.package_id
            || index.package_schema_index_identity != reference.package_schema_index_identity
        {
            return invalid(
                &host_path,
                "typed PackageSchemaIndex does not match exact reference",
            );
        }
        ensure_canonical(&host_path, &bytes, &index)?;
        Ok(Arc::new(index))
    }

    pub fn write_package_schema_type_record(
        &self,
        record: &PackageSchemaTypeRecord,
    ) -> StorageResult<PathBuf> {
        validate_package_schema_type_record(&self.root().to_path_buf(), record)?;
        let reference = PackageSchemaTypeRecordRef {
            package_id: record.package_id.clone(),
            package_schema_type_id: record.package_schema_type_id.clone(),
        };
        let path = PackageSchemaTypeRecordPath::new(&reference)?;
        self.write_immutable(path.as_relative_path(), &canonical_bytes(record)?)
    }

    pub fn read_package_schema_type_record(
        &self,
        reference: &PackageSchemaTypeRecordRef,
    ) -> StorageResult<Arc<PackageSchemaTypeRecord>> {
        let path = PackageSchemaTypeRecordPath::new(reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_string(&host_path, &value, &["packageId"], &reference.package_id)?;
        raw_string(
            &host_path,
            &value,
            &["packageSchemaTypeId"],
            reference.package_schema_type_id.as_str(),
        )?;
        let record = typed_from_value::<PackageSchemaTypeRecord>(&host_path, value)?;
        validate_package_schema_type_record(&host_path, &record)?;
        if record.package_id != reference.package_id
            || record.package_schema_type_id != reference.package_schema_type_id
        {
            return invalid(
                &host_path,
                "typed PackageSchemaTypeRecord does not match exact reference",
            );
        }
        ensure_canonical(&host_path, &bytes, &record)?;
        Ok(Arc::new(record))
    }

    /// Resolves the exact schema record closure declared by a PackageArtifact.
    ///
    /// This validates storage identity and artifact/index/ref agreement only;
    /// compiler and runtime semantics deliberately remain outside the store.
    pub fn resolve_package_artifact_schema(
        &self,
        artifact: &PackageArtifact,
    ) -> StorageResult<ResolvedPackageSchema> {
        validate_package_artifact_identities(artifact)?;
        let index = self.read_package_schema_index(&artifact.package_schema_index)?;
        if index.package_id != artifact.package_id {
            return invalid(
                self.root(),
                "PackageSchemaIndex owner does not match PackageArtifact",
            );
        }

        let index_type_ids = index
            .types
            .values()
            .map(|entry| entry.package_schema_type_id.clone())
            .collect::<BTreeSet<_>>();
        let artifact_type_ids = artifact
            .package_schema_type_records
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if index_type_ids != artifact_type_ids || index.types.len() != index_type_ids.len() {
            return invalid(
                self.root(),
                "PackageArtifact schema record refs must exactly match PackageSchemaIndex entries",
            );
        }

        let mut records = BTreeMap::new();
        for (stable_schema_key, entry) in &index.types {
            let reference = artifact
                .package_schema_type_records
                .get(&entry.package_schema_type_id)
                .ok_or_else(|| EcosystemStorageError::InvalidRecord {
                    path: self.root().to_path_buf(),
                    message: format!(
                        "PackageSchemaIndex entry {stable_schema_key} has no artifact record ref"
                    ),
                })?;
            if reference.package_id != artifact.package_id
                || reference.package_schema_type_id != entry.package_schema_type_id
            {
                return invalid(
                    self.root(),
                    format!(
                        "PackageSchemaIndex entry {stable_schema_key} does not match artifact record ref"
                    ),
                );
            }
            let record = self.read_package_schema_type_record(reference)?;
            if record.package_id != artifact.package_id
                || record.stable_schema_key != *stable_schema_key
                || record.package_schema_type_id != entry.package_schema_type_id
            {
                return invalid(
                    self.root(),
                    format!(
                        "PackageSchemaIndex entry {stable_schema_key} does not match resolved type record"
                    ),
                );
            }
            records.insert(entry.package_schema_type_id.clone(), record);
        }

        let mut pending = records.keys().cloned().collect::<Vec<_>>();
        while let Some(type_id) = pending.pop() {
            let record = records
                .get(&type_id)
                .expect("pending schema record must already be resolved");
            for reference in package_schema_descriptor_refs(&record.canonical_descriptor.descriptor)
            {
                if records.contains_key(&reference.package_schema_type_id) {
                    continue;
                }
                let child = self.read_package_schema_type_record(&PackageSchemaTypeRecordRef {
                    package_id: reference.package_id.clone(),
                    package_schema_type_id: reference.package_schema_type_id.clone(),
                })?;
                if child.package_id != reference.package_id
                    || child.stable_schema_key != reference.stable_schema_key
                    || child.package_schema_type_id != reference.package_schema_type_id
                {
                    return invalid(
                        self.root(),
                        format!(
                            "Package schema child {} does not match referenced owner {} and stable key {}",
                            reference.package_schema_type_id,
                            reference.package_id,
                            reference.stable_schema_key
                        ),
                    );
                }
                pending.push(child.package_schema_type_id.clone());
                records.insert(child.package_schema_type_id.clone(), child);
            }
        }

        let resolved_records = records
            .iter()
            .map(|(type_id, record)| (type_id.clone(), record.as_ref().clone()))
            .collect();
        validate_package_schema_records(&resolved_records)?;
        Ok(ResolvedPackageSchema { index, records })
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

fn validate_package_schema_type_record(
    path: &std::path::Path,
    record: &PackageSchemaTypeRecord,
) -> StorageResult<()> {
    let expected = package_schema_type_id(
        &record.package_id,
        &record.stable_schema_key,
        &record.canonical_descriptor,
    )?;
    if record.package_schema_type_id != expected {
        return invalid(
            path,
            format!(
                "PackageSchemaTypeRecord {} has identity {}, expected {expected}",
                record.stable_schema_key, record.package_schema_type_id
            ),
        );
    }
    Ok(())
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

#[cfg(test)]
mod package_schema_tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use skiff_artifact_identity::{
        assign_package_artifact_identities, package_schema_index_identity, package_schema_type_id,
    };
    use skiff_artifact_model::{
        ContractTypeDescriptor, ContractTypeNameability, PackageBuildId,
        PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
        PackageRuntimeRequirements, PackageSchemaCanonicalDescriptor, PackageSchemaIndexEntry,
        PACKAGE_ARTIFACT_SCHEMA_VERSION,
    };

    use super::*;

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestStore {
        root: PathBuf,
        store: CanonicalArtifactStore,
    }

    impl TestStore {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "skiff-package-schema-store-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let store = CanonicalArtifactStore::create(&root).unwrap();
            Self { root, store }
        }
    }

    impl Drop for TestStore {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn record(package_id: &str, stable_key: &str) -> PackageSchemaTypeRecord {
        let canonical_descriptor = PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    skiff_artifact_model::ContractTypeRef::builtin("string"),
                )]),
            },
        };
        PackageSchemaTypeRecord {
            package_id: package_id.to_string(),
            stable_schema_key: stable_key.to_string(),
            package_schema_type_id: package_schema_type_id(
                package_id,
                stable_key,
                &canonical_descriptor,
            )
            .unwrap(),
            canonical_descriptor,
        }
    }

    fn record_with_child(
        package_id: &str,
        stable_key: &str,
        child: &PackageSchemaTypeRecord,
    ) -> PackageSchemaTypeRecord {
        let canonical_descriptor = PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "child".to_string(),
                    skiff_artifact_model::ContractTypeRef::package_schema(
                        &child.package_id,
                        &child.stable_schema_key,
                        child.package_schema_type_id.clone(),
                    ),
                )]),
            },
        };
        PackageSchemaTypeRecord {
            package_id: package_id.to_string(),
            stable_schema_key: stable_key.to_string(),
            package_schema_type_id: package_schema_type_id(
                package_id,
                stable_key,
                &canonical_descriptor,
            )
            .unwrap(),
            canonical_descriptor,
        }
    }

    fn index(record: &PackageSchemaTypeRecord) -> PackageSchemaIndex {
        let types = BTreeMap::from([(
            record.stable_schema_key.clone(),
            PackageSchemaIndexEntry {
                package_schema_type_id: record.package_schema_type_id.clone(),
                public_path: Some(record.stable_schema_key.clone()),
                nameability: ContractTypeNameability::PublicNameable,
            },
        )]);
        PackageSchemaIndex {
            package_id: record.package_id.clone(),
            package_schema_index_identity: package_schema_index_identity(
                &record.package_id,
                &types,
            )
            .unwrap(),
            types,
        }
    }

    fn artifact(
        package_version: &str,
        index: &PackageSchemaIndex,
        record_refs: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecordRef>,
    ) -> PackageArtifact {
        let mut artifact = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: index.package_id.clone(),
            package_version: package_version.to_string(),
            package_build_id: PackageBuildId::new("unassigned"),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: index.package_id.clone(),
                package_schema_index_identity: index.package_schema_index_identity.clone(),
            },
            package_schema_type_records: record_refs,
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_roots: Vec::new(),
            service_call_refs: Vec::new(),
        };
        assign_package_artifact_identities(&mut artifact).unwrap();
        artifact
    }

    fn record_ref(record: &PackageSchemaTypeRecord) -> PackageSchemaTypeRecordRef {
        PackageSchemaTypeRecordRef {
            package_id: record.package_id.clone(),
            package_schema_type_id: record.package_schema_type_id.clone(),
        }
    }

    #[test]
    fn schema_records_round_trip_idempotently_and_conflicting_bytes_are_rejected() {
        let test = TestStore::new();
        let record = record("example.com/shared", "User");
        let index = index(&record);

        let first_record_path = test
            .store
            .write_package_schema_type_record(&record)
            .unwrap();
        assert_eq!(
            first_record_path,
            test.store
                .write_package_schema_type_record(&record)
                .unwrap()
        );
        let first_index_path = test.store.write_package_schema_index(&index).unwrap();
        assert_eq!(
            first_index_path,
            test.store.write_package_schema_index(&index).unwrap()
        );
        assert_eq!(
            test.store
                .read_package_schema_type_record(&record_ref(&record))
                .unwrap()
                .as_ref(),
            &record
        );
        assert_eq!(
            test.store
                .read_package_schema_index(&PackageSchemaIndexRef {
                    package_id: index.package_id.clone(),
                    package_schema_index_identity: index.package_schema_index_identity.clone(),
                })
                .unwrap()
                .as_ref(),
            &index
        );

        fs::write(&first_record_path, b"conflicting payload").unwrap();
        assert!(matches!(
            test.store.write_package_schema_type_record(&record),
            Err(EcosystemStorageError::ImmutableConflict { .. })
        ));
    }

    #[test]
    fn schema_reads_reject_wrong_path_owner_stable_key_and_descriptor_hash() {
        let test = TestStore::new();
        let user_record = record("example.com/shared", "User");
        let path = test
            .store
            .write_package_schema_type_record(&user_record)
            .unwrap();

        let wrong_owner = PackageSchemaTypeRecord {
            package_id: "example.com/other".to_string(),
            ..user_record.clone()
        };
        assert!(test
            .store
            .write_package_schema_type_record(&wrong_owner)
            .is_err());

        let other = record("example.com/shared", "Other");
        let other_path = PackageSchemaTypeRecordPath::new(&record_ref(&other)).unwrap();
        let other_host_path = test.root.join(other_path.as_relative_path().as_path());
        fs::create_dir_all(other_host_path.parent().unwrap()).unwrap();
        fs::copy(&path, &other_host_path).unwrap();
        assert!(test
            .store
            .read_package_schema_type_record(&record_ref(&other))
            .is_err());

        let mut tampered = user_record.clone();
        tampered.stable_schema_key = "Tampered".to_string();
        fs::write(&path, canonical_bytes(&tampered).unwrap()).unwrap();
        assert!(test
            .store
            .read_package_schema_type_record(&record_ref(&user_record))
            .is_err());

        tampered.stable_schema_key = user_record.stable_schema_key.clone();
        tampered.canonical_descriptor = PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Enumeration {
                variants: vec!["changed".to_string()],
            },
        };
        fs::write(&path, canonical_bytes(&tampered).unwrap()).unwrap();
        assert!(test
            .store
            .read_package_schema_type_record(&record_ref(&user_record))
            .is_err());
    }

    #[test]
    fn package_artifact_schema_resolution_is_exact_and_deduplicated() {
        let test = TestStore::new();
        let record = record("example.com/shared", "User");
        let index = index(&record);
        let reference = record_ref(&record);
        test.store.write_package_schema_index(&index).unwrap();
        let stored_path = test
            .store
            .write_package_schema_type_record(&record)
            .unwrap();
        let refs = BTreeMap::from([(record.package_schema_type_id.clone(), reference.clone())]);
        let first = artifact("1.0.0", &index, refs.clone());
        let second = artifact("2.0.0", &index, refs);

        let first_resolved = test.store.resolve_package_artifact_schema(&first).unwrap();
        let second_resolved = test.store.resolve_package_artifact_schema(&second).unwrap();
        assert_eq!(first_resolved.records.len(), 1);
        assert_eq!(first_resolved.records, second_resolved.records);
        assert_eq!(
            stored_path,
            test.store
                .write_package_schema_type_record(&record)
                .unwrap()
        );
    }

    #[test]
    fn package_artifact_schema_resolution_loads_two_level_cross_package_closure() {
        let test = TestStore::new();
        let leaf = record("example.com/llm-api", "LlmApiFormat");
        let provider = record_with_child("example.com/llm-providers", "Provider", &leaf);
        let relay = record_with_child("example.com/relay", "RelayRequest", &provider);
        let relay_index = index(&relay);

        for record in [&leaf, &provider, &relay] {
            test.store.write_package_schema_type_record(record).unwrap();
        }
        test.store.write_package_schema_index(&relay_index).unwrap();
        let relay_artifact = artifact(
            "1.0.0",
            &relay_index,
            BTreeMap::from([(relay.package_schema_type_id.clone(), record_ref(&relay))]),
        );

        let resolved = test
            .store
            .resolve_package_artifact_schema(&relay_artifact)
            .unwrap();
        assert_eq!(
            resolved.records.keys().cloned().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                relay.package_schema_type_id,
                provider.package_schema_type_id,
                leaf.package_schema_type_id,
            ])
        );
    }

    #[test]
    fn cross_package_schema_resolution_fails_when_a_transitive_child_is_missing() {
        let test = TestStore::new();
        let leaf = record("example.com/llm-api", "LlmApiFormat");
        let provider = record_with_child("example.com/llm-providers", "Provider", &leaf);
        let provider_index = index(&provider);
        test.store
            .write_package_schema_type_record(&provider)
            .unwrap();
        test.store
            .write_package_schema_index(&provider_index)
            .unwrap();
        let provider_artifact = artifact(
            "1.0.0",
            &provider_index,
            BTreeMap::from([(
                provider.package_schema_type_id.clone(),
                record_ref(&provider),
            )]),
        );

        assert!(test
            .store
            .resolve_package_artifact_schema(&provider_artifact)
            .is_err());
    }

    #[test]
    fn package_artifact_schema_resolution_fails_closed_on_missing_or_mismatched_refs() {
        let test = TestStore::new();
        let user_record = record("example.com/shared", "User");
        let index = index(&user_record);
        let reference = record_ref(&user_record);
        let valid_refs = BTreeMap::from([(
            user_record.package_schema_type_id.clone(),
            reference.clone(),
        )]);
        let package_artifact = artifact("1.0.0", &index, valid_refs.clone());

        assert!(test
            .store
            .resolve_package_artifact_schema(&package_artifact)
            .is_err());
        test.store.write_package_schema_index(&index).unwrap();
        assert!(test
            .store
            .resolve_package_artifact_schema(&package_artifact)
            .is_err());
        test.store
            .write_package_schema_type_record(&user_record)
            .unwrap();

        let missing_ref = artifact("1.0.1", &index, BTreeMap::new());
        assert!(test
            .store
            .resolve_package_artifact_schema(&missing_ref)
            .is_err());

        let extra = record("example.com/shared", "Extra");
        let mut extra_refs = valid_refs;
        extra_refs.insert(extra.package_schema_type_id.clone(), record_ref(&extra));
        let extra_ref = artifact("1.0.2", &index, extra_refs);
        assert!(test
            .store
            .resolve_package_artifact_schema(&extra_ref)
            .is_err());

        let mut mismatched_index = index.clone();
        let entry = mismatched_index.types.remove("User").unwrap();
        mismatched_index.types.insert("WrongKey".to_string(), entry);
        mismatched_index.package_schema_index_identity =
            package_schema_index_identity(&mismatched_index.package_id, &mismatched_index.types)
                .unwrap();
        assert!(test
            .store
            .write_package_schema_index(&mismatched_index)
            .is_err());
    }
}
