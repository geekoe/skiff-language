use std::path::{Path, PathBuf};

use skiff_runtime_loader::{
    load_dev_reload_pointers_from_roots, load_service_version_build_pointers_from_roots,
};

use crate::{
    artifact_cache::RuntimeArtifactCaches, config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    host::RuntimeServiceConfig,
};

use super::{
    options::{ArtifactLoadOptions, ArtifactLoadSource},
    runtime_config::load_services_from_rooted_artifact_pointers,
};

#[allow(dead_code)]
pub(crate) async fn load_services_from_artifact_index(
    artifact_root: &Path,
    base_runtime_id: &str,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    load_services_from_artifact_index_with_default(
        artifact_root,
        base_runtime_id,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn load_services_from_artifact_index_with_default(
    artifact_root: &Path,
    base_runtime_id: &str,
    runtime_http_response_max_bytes: usize,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    load_services_from_artifact_root_with_default(
        artifact_root,
        base_runtime_id,
        runtime_http_response_max_bytes,
        &ArtifactLoadOptions::release(),
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn load_services_from_artifact_root(
    artifact_root: &Path,
    base_runtime_id: &str,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    load_services_from_artifact_root_with_default(
        artifact_root,
        base_runtime_id,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        &ArtifactLoadOptions::release(),
    )
    .await
}

pub(crate) async fn load_services_from_artifact_root_with_default(
    artifact_root: &Path,
    base_runtime_id: &str,
    runtime_http_response_max_bytes: usize,
    options: &ArtifactLoadOptions,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    let artifact_roots = vec![artifact_root.to_path_buf()];
    load_services_from_artifact_roots_with_default(
        &artifact_roots,
        base_runtime_id,
        runtime_http_response_max_bytes,
        options,
    )
    .await
}

pub(crate) async fn load_services_from_artifact_roots_with_default(
    artifact_roots: &[PathBuf],
    base_runtime_id: &str,
    runtime_http_response_max_bytes: usize,
    options: &ArtifactLoadOptions,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    if artifact_roots.is_empty() {
        anyhow::bail!("at least one artifacts root is required");
    }
    for artifact_root in artifact_roots {
        if !artifact_root.is_dir() {
            anyhow::bail!(
                "artifacts root {} is not a directory",
                artifact_root.display()
            );
        }
    }

    let pointer_files = match &options.source {
        ArtifactLoadSource::DevReload => load_dev_reload_pointers_from_roots(artifact_roots)?,
        ArtifactLoadSource::Release => {
            load_service_version_build_pointers_from_roots(artifact_roots)?
        }
    };

    let services = load_services_from_rooted_artifact_pointers(
        base_runtime_id,
        runtime_http_response_max_bytes,
        pointer_files,
    )
    .await?;

    Ok(services)
}

pub(crate) async fn load_service_build_from_artifact_roots_with_caches(
    artifact_roots: &[PathBuf],
    service_id: &str,
    build_id: &str,
    base_runtime_id: &str,
    runtime_http_response_max_bytes: usize,
    options: &ArtifactLoadOptions,
    artifact_caches: &RuntimeArtifactCaches,
    allow_missing_local_config: bool,
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    if artifact_roots.is_empty() {
        anyhow::bail!("at least one artifacts root is required");
    }
    for artifact_root in artifact_roots {
        if !artifact_root.is_dir() {
            anyhow::bail!(
                "artifacts root {} is not a directory",
                artifact_root.display()
            );
        }
    }

    let pointer_files = match &options.source {
        ArtifactLoadSource::DevReload => load_dev_reload_pointers_from_roots(artifact_roots)?,
        ArtifactLoadSource::Release => {
            load_service_version_build_pointers_from_roots(artifact_roots)?
        }
    };
    let mut direct_build_matches = Vec::new();
    let mut service_matches = Vec::new();
    for pointer in pointer_files {
        if pointer.entry.service_id != service_id {
            continue;
        }
        if pointer.entry.build_id == build_id {
            direct_build_matches.push(pointer);
        } else {
            service_matches.push(pointer);
        }
    }
    let candidates = if direct_build_matches.is_empty() {
        service_matches
    } else {
        direct_build_matches
    };
    if candidates.is_empty() {
        anyhow::bail!("no artifact pointer matched serviceId {service_id}");
    }

    let services = super::runtime_config::load_services_from_rooted_artifact_pointers_with_caches(
        base_runtime_id,
        runtime_http_response_max_bytes,
        candidates,
        artifact_caches,
        allow_missing_local_config,
    )
    .await?;
    let matches = services
        .into_iter()
        .filter(|service| service.runtime_program_identity.dynamic_build_id == build_id)
        .collect::<Vec<_>>();
    if matches.is_empty() {
        anyhow::bail!(
            "no runtime program matched serviceId {service_id} dynamic buildId {build_id}"
        );
    }
    Ok(matches)
}
