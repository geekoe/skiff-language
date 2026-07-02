use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use skiff_artifact_model::ServiceUnit;
use skiff_runtime_linked_program::{
    ArtifactFileIrUnit as FileIrUnit, FileIrRef, LinkedProgramImage, PackageUnit,
    RuntimeProgramIdentity,
};
use skiff_runtime_loader::{
    select_runtime_program_pointer_from_roots, ArtifactGraph, ArtifactGraphCache,
    ArtifactGraphLoader, ArtifactIndexPointer, RuntimeProgramArtifactSelection,
};

use crate::artifact_cache::{LinkedProgramImageCache, RuntimeArtifactCaches};
use skiff_runtime_activation::{
    build_runtime_activation_for_image, RuntimeActivation, RuntimeActivationCache,
};

#[cfg(any(test, feature = "test-support"))]
use crate::program::RuntimeProgramLayers;

use super::link_runtime_program_image;

pub struct LoadOptions<'a> {
    pub roots: &'a [PathBuf],
    pub caches: Option<&'a RuntimeArtifactCaches>,
}

pub fn load_runtime_program_parts(
    selection: RuntimeProgramArtifactSelection,
    opts: LoadOptions<'_>,
) -> anyhow::Result<LoadedRuntimeProgramParts> {
    let local_caches;
    let caches = if let Some(caches) = opts.caches {
        caches
    } else {
        local_caches = RuntimeArtifactCaches::new();
        &local_caches
    };
    let pointer = select_runtime_program_pointer_from_roots(opts.roots, &selection)?;
    let loaded = RuntimeProgramPartsLoader::new(&pointer.artifact_root, caches)
        .load_pointer_parts_with_service_unit(&pointer.entry)?;
    caches.evict_lru_to_budget();
    Ok(loaded.parts)
}

#[cfg(any(test, feature = "test-support"))]
pub fn load_runtime_program_layers(
    selection: RuntimeProgramArtifactSelection,
    opts: LoadOptions<'_>,
) -> anyhow::Result<Arc<RuntimeProgramLayers>> {
    let parts = load_runtime_program_parts(selection, opts)?;
    Ok(runtime_program_layers_from_parts(parts))
}

pub(super) struct LoadedRuntimeProgramPartsArtifact {
    pub(super) service_unit: Arc<ServiceUnit>,
    pub(super) parts: LoadedRuntimeProgramParts,
}

pub struct LoadedRuntimeProgramParts {
    pub(super) identity: RuntimeProgramIdentity,
    pub(super) image: Arc<LinkedProgramImage>,
    pub(super) activation: Arc<RuntimeActivation>,
}

#[cfg(any(test, feature = "test-support"))]
fn runtime_program_layers_from_parts(
    parts: LoadedRuntimeProgramParts,
) -> Arc<RuntimeProgramLayers> {
    Arc::new(RuntimeProgramLayers::new(
        parts.identity,
        parts.image,
        parts.activation,
    ))
}

pub(super) struct RuntimeProgramPartsLoader<'a> {
    graph_loader: ArtifactGraphLoader<'a>,
    image_cache: &'a LinkedProgramImageCache,
    activation_cache: &'a RuntimeActivationCache,
}

impl<'a> RuntimeProgramPartsLoader<'a> {
    pub(super) fn new(artifact_root: &'a Path, caches: &'a RuntimeArtifactCaches) -> Self {
        Self {
            graph_loader: ArtifactGraphLoader::new(
                artifact_root,
                ArtifactGraphCache::new(&caches.files, &caches.packages),
            ),
            image_cache: &caches.images,
            activation_cache: &caches.activation_cache,
        }
    }

    pub(super) fn load_pointer_parts_with_service_unit(
        &self,
        pointer: &ArtifactIndexPointer,
    ) -> anyhow::Result<LoadedRuntimeProgramPartsArtifact> {
        let graph = self.load_pointer_artifact_graph(pointer)?;
        let service_unit = graph.service_unit.clone();
        let parts = self.link_loaded_artifact_graph_parts(graph)?;
        Ok(LoadedRuntimeProgramPartsArtifact {
            service_unit,
            parts,
        })
    }

    pub(super) fn load_pointer_artifact_graph(
        &self,
        pointer: &ArtifactIndexPointer,
    ) -> anyhow::Result<ArtifactGraph> {
        self.graph_loader.load_pointer_artifact_graph(pointer)
    }

    pub fn load_service_artifact_graph(
        &self,
        service: Arc<ServiceUnit>,
    ) -> anyhow::Result<ArtifactGraph> {
        self.graph_loader.load_service_artifact_graph(service)
    }

    pub(super) fn link_loaded_artifact_graph_parts(
        &self,
        graph: ArtifactGraph,
    ) -> anyhow::Result<LoadedRuntimeProgramParts> {
        let image_build = link_runtime_program_image(graph)
            .map_err(|error| anyhow::anyhow!("failed to link runtime program: {error}"))?;
        let identity = image_build.identity;
        let image = self
            .image_cache
            .insert(identity.linked_image_identity.clone(), image_build.image);
        let activation = Arc::new(
            build_runtime_activation_for_image(image.as_ref(), image_build.activation_facts)
                .map_err(|error| anyhow::anyhow!("failed to build runtime activation: {error}"))?,
        );
        let activation_entry = self
            .activation_cache
            .insert_arc(identity.clone(), activation);
        if activation_entry.linked_image_identity() != identity.linked_image_identity {
            anyhow::bail!(
                "cached runtime activation linked image identity mismatch for dynamic build id {}: cached {}, current {}",
                identity.dynamic_build_id,
                activation_entry.linked_image_identity(),
                identity.linked_image_identity
            );
        }
        Ok(LoadedRuntimeProgramParts {
            identity: activation_entry.identity().clone(),
            image,
            activation: activation_entry.activation(),
        })
    }

    pub fn load_service_unit_at_path(&self, relative_path: &Path) -> anyhow::Result<ServiceUnit> {
        self.graph_loader.load_service_unit_at_path(relative_path)
    }

    pub fn load_file_ir_at_path(&self, relative_path: &Path) -> anyhow::Result<Arc<FileIrUnit>> {
        self.graph_loader.load_file_ir_at_path(relative_path)
    }

    pub fn load_package_unit_at_path(
        &self,
        relative_path: &Path,
    ) -> anyhow::Result<Arc<PackageUnit>> {
        self.graph_loader.load_package_unit_at_path(relative_path)
    }

    pub(crate) fn load_file_refs(
        &self,
        refs: &[FileIrRef],
        label: &str,
    ) -> anyhow::Result<Vec<Arc<FileIrUnit>>> {
        self.graph_loader.load_file_refs(refs, label)
    }
}

#[cfg(test)]
mod tests;
