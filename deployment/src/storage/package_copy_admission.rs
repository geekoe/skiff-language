use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use skiff_artifact_identity::{
    validate_package_artifact_identities, ArtifactRelativePath, PackageArtifactRecordPath,
    PackageFileIrRecordPath, PackageResourceRecordPath, PackageSchemaIndexRecordPath,
    PackageSchemaTypeRecordPath,
};
use skiff_artifact_model::{
    FileIrUnit, PackageArtifact, PackageArtifactRef, PackageSchemaIndexRef,
    PackageSchemaTypeRecordRef,
};

use super::{
    error::{EcosystemStorageError, StorageResult},
    io::{strict_value, typed_from_value, CanonicalArtifactStore},
    records::{ensure_canonical, raw_package_ref, validate_file_ref, validate_resource},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidatedPackageRecordKind {
    FileIr,
    StaticResource,
    SchemaType,
    SchemaIndex,
    PackageArtifact,
}

#[derive(Debug)]
struct ValidatedPackageRecord {
    kind: ValidatedPackageRecordKind,
    path: ArtifactRelativePath,
    bytes: Arc<[u8]>,
    byte_len: u64,
}

/// Opaque, process-local proof that one exact package and its schema closure
/// passed canonical store admission from one canonical source root.
///
/// Fields deliberately remain private and the type has no serialization
/// surface, so callers can reuse but cannot manufacture or persist admission.
#[derive(Debug)]
pub struct ValidatedPackageCopyRecords {
    source_root: PathBuf,
    reference: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    records: Vec<ValidatedPackageRecord>,
}

#[derive(Debug)]
struct ValidatedPackageArtifactRecord {
    source_root: PathBuf,
    reference: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    record: ValidatedPackageRecord,
}

/// Raw bytes of one exact package closure, read without canonical
/// re-validation.
///
/// This is only safe when the source store was produced and owned by the same
/// trusted workflow (for example the canonical isolated service test fixture).
/// The bytes are still content-addressed by their record paths, so the target
/// store's immutable writer rejects any conflicting content.
#[derive(Debug)]
pub struct RawPackageCopyRecords {
    records: Vec<(ArtifactRelativePath, Vec<u8>)>,
}

impl ValidatedPackageCopyRecords {
    pub fn artifact(&self) -> &PackageArtifact {
        &self.artifact
    }

    pub fn reference(&self) -> &PackageArtifactRef {
        &self.reference
    }
}

#[derive(Debug, Default)]
pub struct PackageArtifactAdmissionCache {
    records: BTreeMap<(PathBuf, PackageArtifactRef), ValidatedPackageCopyRecords>,
    admission_count: usize,
}

impl PackageArtifactAdmissionCache {
    pub fn admit<'a>(
        &'a mut self,
        source: &CanonicalArtifactStore,
        reference: &PackageArtifactRef,
    ) -> StorageResult<&'a ValidatedPackageCopyRecords> {
        let key = (source.root().to_path_buf(), reference.clone());
        match self.records.entry(key) {
            Entry::Occupied(entry) => {
                source.verify_validated_package_copy_records(entry.get())?;
                Ok(entry.into_mut())
            }
            Entry::Vacant(entry) => {
                let admitted = source.read_validated_package_copy_records(reference)?;
                self.admission_count = self.admission_count.checked_add(1).ok_or_else(|| {
                    EcosystemStorageError::InvalidRecord {
                        path: source.root().to_path_buf(),
                        message: "package admission count overflow".to_string(),
                    }
                })?;
                Ok(entry.insert(admitted))
            }
        }
    }

    #[cfg(test)]
    pub(super) fn admission_count(&self) -> usize {
        self.admission_count
    }
}

impl CanonicalArtifactStore {
    pub fn read_package_copy_records_raw(
        &self,
        reference: &PackageArtifactRef,
    ) -> StorageResult<RawPackageCopyRecords> {
        let package_path = PackageArtifactRecordPath::new(reference)?;
        let package_bytes = self.read_bytes(package_path.as_relative_path())?;
        let host_path = self.root().join(package_path.as_relative_path().as_path());
        let value = strict_value(&host_path, &package_bytes)?;
        let package = typed_from_value::<PackageArtifact>(&host_path, value)?;
        let mut records = Vec::with_capacity(
            package.files.len()
                + package.static_resources.len()
                + package.package_schema_type_records.len()
                + 2,
        );
        records.push((package_path.as_relative_path().clone(), package_bytes));
        for file_reference in &package.files {
            let path = PackageFileIrRecordPath::new(reference, file_reference)?;
            let bytes = self.read_bytes(path.as_relative_path())?;
            records.push((path.as_relative_path().clone(), bytes));
        }
        for resource_reference in &package.static_resources {
            let path = PackageResourceRecordPath::new(reference, resource_reference)?;
            let bytes = self.read_bytes(path.as_relative_path())?;
            records.push((path.as_relative_path().clone(), bytes));
        }
        for record in package.package_schema_type_records.values() {
            let record_reference = PackageSchemaTypeRecordRef {
                package_id: record.package_id.clone(),
                package_schema_type_id: record.package_schema_type_id.clone(),
            };
            let path = PackageSchemaTypeRecordPath::new(&record_reference)?;
            let bytes = self.read_bytes(path.as_relative_path())?;
            records.push((path.as_relative_path().clone(), bytes));
        }
        let index_path = PackageSchemaIndexRecordPath::new(&PackageSchemaIndexRef {
            package_id: package.package_schema_index.package_id.clone(),
            package_schema_index_identity: package
                .package_schema_index
                .package_schema_index_identity
                .clone(),
        })?;
        let index_bytes = self.read_bytes(index_path.as_relative_path())?;
        records.push((index_path.as_relative_path().clone(), index_bytes));
        Ok(RawPackageCopyRecords { records })
    }

    pub fn write_package_copy_records_raw(
        &self,
        records: &RawPackageCopyRecords,
    ) -> StorageResult<Vec<PathBuf>> {
        records
            .records
            .iter()
            .map(|(path, bytes)| self.write_immutable(path, bytes))
            .collect()
    }

    pub(super) fn read_validated_package_artifact_record(
        &self,
        reference: &PackageArtifactRef,
    ) -> StorageResult<Arc<PackageArtifact>> {
        Ok(self.admit_package_artifact_record(reference)?.artifact)
    }

    fn admit_package_artifact_record(
        &self,
        reference: &PackageArtifactRef,
    ) -> StorageResult<ValidatedPackageArtifactRecord> {
        let path = PackageArtifactRecordPath::new(reference)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let value = strict_value(&host_path, &bytes)?;
        raw_package_ref(&host_path, &value, reference)?;
        let artifact = typed_from_value::<PackageArtifact>(&host_path, value)?;
        validate_package_artifact_identities(&artifact)?;
        if &declared_package_artifact_ref(&artifact) != reference {
            return invalid(
                &host_path,
                "typed PackageArtifact does not match exact reference",
            );
        }
        ensure_canonical(&host_path, &bytes, &artifact)?;
        Ok(ValidatedPackageArtifactRecord {
            source_root: self.root().to_path_buf(),
            reference: reference.clone(),
            artifact: Arc::new(artifact),
            record: validated_package_record(
                ValidatedPackageRecordKind::PackageArtifact,
                path.as_relative_path().clone(),
                bytes,
            )?,
        })
    }

    pub fn read_validated_package_copy_records(
        &self,
        reference: &PackageArtifactRef,
    ) -> StorageResult<ValidatedPackageCopyRecords> {
        let package = self.admit_package_artifact_record(reference)?;
        let schema = self.resolve_package_artifact_schema_after_validation(&package.artifact)?;
        let mut records = Vec::with_capacity(
            package.artifact.files.len()
                + package.artifact.static_resources.len()
                + schema.records.len()
                + 2,
        );
        for file_reference in &package.artifact.files {
            let path = PackageFileIrRecordPath::new(reference, file_reference)?;
            let bytes = self.read_bytes(path.as_relative_path())?;
            let host_path = self.root().join(path.as_relative_path().as_path());
            let value = strict_value(&host_path, &bytes)?;
            let file = typed_from_value::<FileIrUnit>(&host_path, value)?;
            validate_file_ref(&host_path, reference, file_reference, &file)?;
            ensure_canonical(&host_path, &bytes, &file)?;
            records.push(validated_package_record(
                ValidatedPackageRecordKind::FileIr,
                path.as_relative_path().clone(),
                bytes,
            )?);
        }
        for resource_reference in &package.artifact.static_resources {
            let path = PackageResourceRecordPath::new(reference, resource_reference)?;
            let bytes = self.read_bytes(path.as_relative_path())?;
            validate_resource(
                &self.root().join(path.as_relative_path().as_path()),
                resource_reference,
                &bytes,
            )?;
            records.push(validated_package_record(
                ValidatedPackageRecordKind::StaticResource,
                path.as_relative_path().clone(),
                bytes,
            )?);
        }
        for record in schema.records.values() {
            let record_reference = PackageSchemaTypeRecordRef {
                package_id: record.package_id.clone(),
                package_schema_type_id: record.package_schema_type_id.clone(),
            };
            let path = PackageSchemaTypeRecordPath::new(&record_reference)?;
            let bytes = self.read_bytes(path.as_relative_path())?;
            ensure_canonical(
                &self.root().join(path.as_relative_path().as_path()),
                &bytes,
                record.as_ref(),
            )?;
            records.push(validated_package_record(
                ValidatedPackageRecordKind::SchemaType,
                path.as_relative_path().clone(),
                bytes,
            )?);
        }
        let index_path = PackageSchemaIndexRecordPath::new(&PackageSchemaIndexRef {
            package_id: schema.index.package_id.clone(),
            package_schema_index_identity: schema.index.package_schema_index_identity.clone(),
        })?;
        let index_bytes = self.read_bytes(index_path.as_relative_path())?;
        ensure_canonical(
            &self.root().join(index_path.as_relative_path().as_path()),
            &index_bytes,
            schema.index.as_ref(),
        )?;
        records.push(validated_package_record(
            ValidatedPackageRecordKind::SchemaIndex,
            index_path.as_relative_path().clone(),
            index_bytes,
        )?);
        records.push(package.record);
        Ok(ValidatedPackageCopyRecords {
            source_root: package.source_root,
            reference: package.reference,
            artifact: package.artifact,
            records,
        })
    }

    pub fn verify_validated_package_copy_records(
        &self,
        admitted: &ValidatedPackageCopyRecords,
    ) -> StorageResult<()> {
        validate_package_copy_token(admitted)?;
        if admitted.source_root != self.root() {
            return invalid(
                self.root(),
                format!(
                    "validated package source root {} does not match store root {}",
                    admitted.source_root.display(),
                    self.root().display()
                ),
            );
        }
        for record in &admitted.records {
            let bytes = self.read_bytes(&record.path)?;
            validate_record_fingerprint(self.root(), record, &bytes)?;
        }
        Ok(())
    }

    pub fn write_validated_package_copy_records(
        &self,
        admitted: &ValidatedPackageCopyRecords,
    ) -> StorageResult<Vec<PathBuf>> {
        validate_package_copy_token(admitted)?;
        admitted
            .records
            .iter()
            .map(|record| {
                validate_record_fingerprint(self.root(), record, &record.bytes)?;
                self.write_immutable(&record.path, &record.bytes)
            })
            .collect()
    }
}

pub(super) fn declared_package_artifact_ref(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}

fn validated_package_record(
    kind: ValidatedPackageRecordKind,
    path: ArtifactRelativePath,
    bytes: Vec<u8>,
) -> StorageResult<ValidatedPackageRecord> {
    let byte_len =
        u64::try_from(bytes.len()).map_err(|_| EcosystemStorageError::InvalidRecord {
            path: path.as_path().to_path_buf(),
            message: format!("{kind:?} byte length does not fit u64"),
        })?;
    Ok(ValidatedPackageRecord {
        kind,
        path,
        bytes: Arc::from(bytes),
        byte_len,
    })
}

fn validate_package_copy_token(admitted: &ValidatedPackageCopyRecords) -> StorageResult<()> {
    if declared_package_artifact_ref(&admitted.artifact) != admitted.reference {
        return invalid(
            &admitted.source_root,
            "validated PackageArtifact declaration no longer matches its exact reference",
        );
    }
    let expected_package_path = PackageArtifactRecordPath::new(&admitted.reference)?;
    let expected_index_path =
        PackageSchemaIndexRecordPath::new(&admitted.artifact.package_schema_index)?;
    let mut expected_file_paths = admitted
        .artifact
        .files
        .iter()
        .map(|reference| {
            Ok(
                PackageFileIrRecordPath::new(&admitted.reference, reference)?
                    .as_relative_path()
                    .as_str()
                    .to_string(),
            )
        })
        .collect::<StorageResult<BTreeSet<_>>>()?;
    let mut expected_resource_paths = admitted
        .artifact
        .static_resources
        .iter()
        .map(|reference| {
            Ok(
                PackageResourceRecordPath::new(&admitted.reference, reference)?
                    .as_relative_path()
                    .as_str()
                    .to_string(),
            )
        })
        .collect::<StorageResult<BTreeSet<_>>>()?;
    let mut paths = BTreeSet::new();
    let mut package_records = 0usize;
    let mut index_records = 0usize;
    for record in &admitted.records {
        if !paths.insert(record.path.as_str().to_string()) {
            return invalid(
                &admitted.source_root,
                format!("validated package copy token duplicates {}", record.path),
            );
        }
        match record.kind {
            ValidatedPackageRecordKind::FileIr => {
                if !expected_file_paths.remove(record.path.as_str()) {
                    return invalid(
                        &admitted.source_root,
                        "validated File IR record path is not declared by the PackageArtifact",
                    );
                }
            }
            ValidatedPackageRecordKind::StaticResource => {
                if !expected_resource_paths.remove(record.path.as_str()) {
                    return invalid(
                        &admitted.source_root,
                        "validated static resource path is not declared by the PackageArtifact",
                    );
                }
            }
            ValidatedPackageRecordKind::PackageArtifact => {
                package_records += 1;
                if record.path != *expected_package_path.as_relative_path() {
                    return invalid(
                        &admitted.source_root,
                        "validated PackageArtifact record path does not match its exact reference",
                    );
                }
            }
            ValidatedPackageRecordKind::SchemaIndex => {
                index_records += 1;
                if record.path != *expected_index_path.as_relative_path() {
                    return invalid(
                        &admitted.source_root,
                        "validated PackageSchemaIndex path does not match the PackageArtifact",
                    );
                }
            }
            ValidatedPackageRecordKind::SchemaType => {}
        }
    }
    if package_records != 1 || index_records != 1 {
        return invalid(
            &admitted.source_root,
            "validated package copy token must contain exactly one artifact and schema index",
        );
    }
    if !expected_file_paths.is_empty() || !expected_resource_paths.is_empty() {
        return invalid(
            &admitted.source_root,
            "validated package copy token is missing a declared File IR or static resource",
        );
    }
    Ok(())
}

fn validate_record_fingerprint(
    root: &std::path::Path,
    record: &ValidatedPackageRecord,
    bytes: &[u8],
) -> StorageResult<()> {
    let byte_len =
        u64::try_from(bytes.len()).map_err(|_| EcosystemStorageError::InvalidRecord {
            path: root.join(record.path.as_path()),
            message: format!("{:?} byte length does not fit u64", record.kind),
        })?;
    if byte_len != record.byte_len || bytes != record.bytes.as_ref() {
        return invalid(
            &root.join(record.path.as_path()),
            format!(
                "{:?} bytes no longer match the validated package copy token",
                record.kind
            ),
        );
    }
    Ok(())
}

fn invalid<T>(path: &std::path::Path, message: impl Into<String>) -> StorageResult<T> {
    Err(EcosystemStorageError::InvalidRecord {
        path: path.to_path_buf(),
        message: message.into(),
    })
}
