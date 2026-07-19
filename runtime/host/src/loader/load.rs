use std::path::{Path, PathBuf};

use skiff_runtime_loader::{
    load_dev_reload_pointers_from_roots, load_service_version_build_pointers_from_roots,
};

use crate::{config::DEFAULT_HTTP_RESPONSE_MAX_BYTES, host::RuntimeServiceConfig};

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
