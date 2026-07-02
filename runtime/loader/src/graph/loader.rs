use std::{path::Path, sync::Arc};

use anyhow::Context;
use serde_json::Value;
use skiff_artifact_identity::ordered_package_units_from_artifact_root;
use skiff_artifact_model::{FileIrRef, FileIrUnit, PackageUnit, ServiceUnit};

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
    types::ArtifactIndexPointer,
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
        self.load_service_artifact_graph(service)
    }

    pub fn load_service_artifact_graph(
        &self,
        service: Arc<ServiceUnit>,
    ) -> anyhow::Result<ArtifactGraph> {
        let service_files = self.load_file_refs(&service.files, "service unit files")?;
        let package_units = self.resolve_service_packages(&service)?;
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

    pub(super) fn read_artifact_json(
        &self,
        relative_path: &ArtifactRootRelativePath,
        label: &str,
    ) -> anyhow::Result<Value> {
        let path = resolve_index_artifact_path(self.artifact_root, relative_path, label)?;
        read_json_file(&path, label)
    }
}
