use std::{fs, path::Path, sync::Arc};

use anyhow::Context;
use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_artifact_identity::{
    ordered_package_units_from_artifact_refs, ordered_package_units_from_artifact_root,
    PackageUnitArtifactRef,
};
use skiff_artifact_model::{
    FileIrRef, FileIrUnit, PackageUnit, PublicationResourceRef, ServiceUnit,
};
use skiff_runtime_linked_program::{
    LoadedPublicationResource, PublicationResourcePath, PublicationResourceTable,
};

use super::{
    cache::ArtifactGraphCache,
    graph::{ArtifactGraph, ArtifactGraphIdentities},
    validation::{
        deserialize_file_ir_unit, deserialize_package_unit, deserialize_service_unit,
        validate_loaded_file_ref,
    },
};
use crate::{
    paths::{resolve_index_artifact_path, ArtifactRootRelativePath},
    types::{ArtifactIndexPointer, PackageUnitArtifactPointer},
    utils::read_json_file,
};

#[derive(Debug, Clone, Copy)]
pub struct ArtifactGraphLoader<'a> {
    pub(super) artifact_root: &'a Path,
    cache: ArtifactGraphCache<'a>,
}

impl<'a> ArtifactGraphLoader<'a> {
    pub fn new(artifact_root: &'a Path, cache: ArtifactGraphCache<'a>) -> Self {
        Self {
            artifact_root,
            cache,
        }
    }

    pub fn load_pointer_artifact_graph(
        &self,
        pointer: &ArtifactIndexPointer,
    ) -> anyhow::Result<ArtifactGraph> {
        let service_unit_path = self
            .resolve_service_unit_pointer(pointer)
            .with_context(|| {
                format!(
                    "failed to resolve service unit for service {} build {}",
                    pointer.service_id, pointer.build_id
                )
            })?;
        let service = Arc::new(self.load_service_unit_at_artifact_path(&service_unit_path)?);
        self.load_service_artifact_graph_for_pointer(service, pointer)
    }

    pub fn load_service_artifact_graph(
        &self,
        service: Arc<ServiceUnit>,
    ) -> anyhow::Result<ArtifactGraph> {
        let service_files = self.load_file_refs(&service.files, "service unit files")?;
        let service_resources =
            self.load_resource_refs(&service.resources, "service unit resources")?;
        let package_units = self.resolve_service_packages(&service)?;
        self.build_artifact_graph(service, service_files, service_resources, package_units)
    }

    fn load_service_artifact_graph_for_pointer(
        &self,
        service: Arc<ServiceUnit>,
        pointer: &ArtifactIndexPointer,
    ) -> anyhow::Result<ArtifactGraph> {
        let service_files = self.load_file_refs(&service.files, "service unit files")?;
        let service_resources =
            self.load_resource_refs(&service.resources, "service unit resources")?;
        let package_units = match &pointer.package_units {
            Some(package_units) => self.resolve_pinned_service_packages(&service, package_units)?,
            None => self.resolve_service_packages(&service)?,
        };
        self.build_artifact_graph(service, service_files, service_resources, package_units)
    }

    fn build_artifact_graph(
        &self,
        service: Arc<ServiceUnit>,
        service_files: Vec<Arc<FileIrUnit>>,
        service_resources: PublicationResourceTable,
        package_units: Vec<Arc<PackageUnit>>,
    ) -> anyhow::Result<ArtifactGraph> {
        let package_files = package_units
            .iter()
            .map(|package| {
                self.load_file_refs(
                    &package.files,
                    &format!(
                        "package unit {}@{} files",
                        package.package_id, package.version
                    ),
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let package_resources = package_units
            .iter()
            .map(|package| {
                self.load_resource_refs(
                    &package.resources,
                    &format!(
                        "package unit {}@{} resources",
                        package.package_id, package.version
                    ),
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let identities = ArtifactGraphIdentities::from_loaded_units(
            &service_files,
            &package_units,
            &package_files,
        );
        Ok(ArtifactGraph {
            service_unit: service,
            service_files,
            package_units,
            package_files,
            service_resources,
            package_resources,
            identities,
        })
    }

    pub fn load_service_unit_at_path(&self, relative_path: &Path) -> anyhow::Result<ServiceUnit> {
        let relative_path = ArtifactRootRelativePath::new(relative_path, "service unit")?;
        self.load_service_unit_at_artifact_path(&relative_path)
    }

    pub(super) fn load_service_unit_at_artifact_path(
        &self,
        relative_path: &ArtifactRootRelativePath,
    ) -> anyhow::Result<ServiceUnit> {
        let value = self.read_artifact_json(relative_path, "service unit")?;
        deserialize_service_unit(value, relative_path)
    }

    pub fn load_file_ir_at_path(&self, relative_path: &Path) -> anyhow::Result<Arc<FileIrUnit>> {
        let relative_path = ArtifactRootRelativePath::new(relative_path, "File IR unit")?;
        self.load_file_ir_at_artifact_path(&relative_path)
    }

    fn load_file_ir_at_artifact_path(
        &self,
        relative_path: &ArtifactRootRelativePath,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        let value = self.read_artifact_json(relative_path, "File IR unit")?;
        let unit = deserialize_file_ir_unit(value, relative_path)?;
        Ok(self.cache.insert_file(unit))
    }

    pub fn load_package_unit_at_path(
        &self,
        relative_path: &Path,
    ) -> anyhow::Result<Arc<PackageUnit>> {
        let relative_path = ArtifactRootRelativePath::new(relative_path, "package unit")?;
        self.load_package_unit_at_artifact_path(&relative_path)
    }

    fn load_package_unit_at_artifact_path(
        &self,
        relative_path: &ArtifactRootRelativePath,
    ) -> anyhow::Result<Arc<PackageUnit>> {
        let value = self.read_artifact_json(relative_path, "package unit")?;
        let unit = deserialize_package_unit(value, relative_path)?;
        Ok(self.cache.insert_package(unit))
    }

    pub fn load_file_refs(
        &self,
        refs: &[FileIrRef],
        label: &str,
    ) -> anyhow::Result<Vec<Arc<FileIrUnit>>> {
        refs.iter()
            .map(|file_ref| self.load_file_ref(file_ref, label))
            .collect()
    }

    fn load_file_ref(&self, file_ref: &FileIrRef, label: &str) -> anyhow::Result<Arc<FileIrUnit>> {
        let relative_path = self
            .file_ir_path(file_ref)
            .with_context(|| format!("{label} {}", file_ref.file_ir_identity))?;
        let unit = self.load_file_ir_at_artifact_path(&relative_path)?;
        validate_loaded_file_ref(unit.as_ref(), file_ref, &relative_path, label)?;
        Ok(unit)
    }

    pub fn load_resource_refs(
        &self,
        refs: &[PublicationResourceRef],
        label: &str,
    ) -> anyhow::Result<PublicationResourceTable> {
        let mut table = PublicationResourceTable::default();
        for resource_ref in refs {
            let logical_path =
                PublicationResourcePath::parse(&resource_ref.path).map_err(|error| {
                    anyhow::anyhow!(
                        "{label} resource path {} is invalid: {}",
                        resource_ref.path,
                        error.message()
                    )
                })?;
            let relative_path = self
                .resource_artifact_path(resource_ref, label)
                .with_context(|| format!("{label} resource {}", logical_path.as_str()))?;
            let bytes = self.read_artifact_bytes(
                &relative_path,
                &format!("{label} resource {}", logical_path.as_str()),
            )?;
            validate_loaded_resource(resource_ref, &bytes, &relative_path, label)?;
            let resource = LoadedPublicationResource {
                meta: resource_ref.clone(),
                bytes: Arc::from(bytes.into_boxed_slice()),
            };
            if table
                .insert(logical_path.as_str().to_string(), resource)
                .is_some()
            {
                anyhow::bail!(
                    "{label} contains duplicate resource path {}",
                    logical_path.as_str()
                );
            }
        }
        Ok(table)
    }

    fn file_ir_path(&self, file_ref: &FileIrRef) -> anyhow::Result<ArtifactRootRelativePath> {
        if let Some(path) = &file_ref.artifact_path {
            return ArtifactRootRelativePath::parse(
                path,
                &format!("File IR unit {} artifactPath", file_ref.file_ir_identity),
            );
        }
        anyhow::bail!(
            "File IR unit {} requires artifactPath when loading from artifact root",
            file_ref.file_ir_identity
        )
    }

    fn resource_artifact_path(
        &self,
        resource_ref: &PublicationResourceRef,
        label: &str,
    ) -> anyhow::Result<ArtifactRootRelativePath> {
        let Some(path) = &resource_ref.artifact_path else {
            anyhow::bail!(
                "{label} resource {} requires artifactPath when loading from artifact root",
                resource_ref.path
            );
        };
        if path.contains('\\') {
            anyhow::bail!(
                "{label} resource {} artifactPath {} must use / separators",
                resource_ref.path,
                path
            );
        }
        ArtifactRootRelativePath::parse(
            path,
            &format!("{label} resource {} artifactPath", resource_ref.path),
        )
    }

    fn resolve_service_packages(
        &self,
        service: &ServiceUnit,
    ) -> anyhow::Result<Vec<Arc<PackageUnit>>> {
        let packages = ordered_package_units_from_artifact_root(self.artifact_root, service)
            .context("failed to resolve service package dependencies")?;
        Ok(packages
            .into_iter()
            .map(|package| self.cache.insert_package(package))
            .collect())
    }

    fn resolve_pinned_service_packages(
        &self,
        service: &ServiceUnit,
        package_refs: &[PackageUnitArtifactPointer],
    ) -> anyhow::Result<Vec<Arc<PackageUnit>>> {
        let package_refs = package_refs
            .iter()
            .map(package_unit_artifact_ref)
            .collect::<Vec<_>>();
        let packages =
            ordered_package_units_from_artifact_refs(self.artifact_root, service, &package_refs)
                .context("failed to resolve pinned service package dependencies")?;
        Ok(packages
            .into_iter()
            .map(|package| self.cache.insert_package(package))
            .collect())
    }

    pub(super) fn read_artifact_json(
        &self,
        relative_path: &ArtifactRootRelativePath,
        label: &str,
    ) -> anyhow::Result<Value> {
        let path = resolve_index_artifact_path(self.artifact_root, relative_path, label)?;
        read_json_file(&path, label)
    }

    fn read_artifact_bytes(
        &self,
        relative_path: &ArtifactRootRelativePath,
        label: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let path = resolve_index_artifact_path(self.artifact_root, relative_path, label)?;
        fs::read(&path)
            .map_err(|error| anyhow::anyhow!("failed to read {label} {}: {error}", path.display()))
    }
}

fn validate_loaded_resource(
    resource_ref: &PublicationResourceRef,
    bytes: &[u8],
    relative_path: &ArtifactRootRelativePath,
    label: &str,
) -> anyhow::Result<()> {
    let actual_len = u64::try_from(bytes.len()).map_err(|_| {
        anyhow::anyhow!(
            "{label} resource {} loaded from {} is too large to validate",
            resource_ref.path,
            relative_path.display()
        )
    })?;
    if actual_len != resource_ref.byte_len {
        anyhow::bail!(
            "{label} resource {} loaded from {} byte_len mismatch: expected {}, got {}",
            resource_ref.path,
            relative_path.display(),
            resource_ref.byte_len,
            actual_len
        );
    }
    let actual_sha256 = hex::encode(Sha256::digest(bytes));
    if actual_sha256 != resource_ref.sha256 {
        anyhow::bail!(
            "{label} resource {} loaded from {} sha256 mismatch: expected {}, got {}",
            resource_ref.path,
            relative_path.display(),
            resource_ref.sha256,
            actual_sha256
        );
    }
    Ok(())
}

fn package_unit_artifact_ref(pointer: &PackageUnitArtifactPointer) -> PackageUnitArtifactRef {
    PackageUnitArtifactRef {
        package_id: pointer.package_id.clone(),
        version: pointer.version.clone(),
        build_identity: pointer.build_identity.clone(),
        abi_identity: pointer.abi_identity.clone(),
        unit_hash: pointer.unit_hash.clone(),
        unit_path: pointer.unit_path.clone(),
    }
}
