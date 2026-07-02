use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use skiff_runtime_loader::service_id_artifact_path;

use super::{identity::identity_hash_with_label, SERVICE_VERSION_POINTER_SCHEMA_VERSION};

pub(crate) fn artifact_roots_control_fingerprint(
    artifact_roots: &[PathBuf],
    dev_reload: Option<bool>,
) -> anyhow::Result<String> {
    if artifact_roots.is_empty() {
        anyhow::bail!("at least one artifacts root is required");
    }
    if dev_reload.unwrap_or(false) {
        return multi_root_fingerprint(artifact_roots, |artifact_root| {
            let current = artifact_root.join("dev").join("services");
            if !current.exists() {
                return Ok(None);
            }
            if !current.is_dir() {
                anyhow::bail!(
                    "artifact dev reload dir {} is not a directory",
                    current.display()
                );
            }
            artifact_files_fingerprint(artifact_root, &current).map(Some)
        });
    }
    multi_root_fingerprint(artifact_roots, |artifact_root| {
        let version_root = artifact_root.join("versions").join("services");
        if !version_root.exists() {
            return Ok(None);
        }
        if !version_root.is_dir() {
            anyhow::bail!(
                "artifact versions dir {} is not a directory",
                version_root.display()
            );
        }
        service_version_artifact_fingerprint(artifact_root).map(Some)
    })
}

fn multi_root_fingerprint<F>(
    artifact_roots: &[PathBuf],
    mut fingerprint: F,
) -> anyhow::Result<String>
where
    F: FnMut(&Path) -> anyhow::Result<Option<String>>,
{
    let mut hasher = Sha256::new();
    let mut found = false;
    for (index, artifact_root) in artifact_roots.iter().enumerate() {
        let Some(root_fingerprint) = fingerprint(artifact_root)? else {
            continue;
        };
        found = true;
        hasher.update(index.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(artifact_root.display().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(root_fingerprint.as_bytes());
        hasher.update(b"\0");
    }
    if !found {
        anyhow::bail!(
            "artifact roots {} have no artifact pointer JSON files",
            artifact_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(hex::encode(hasher.finalize()))
}

fn service_version_artifact_fingerprint(artifact_root: &Path) -> anyhow::Result<String> {
    let version_root = artifact_root.join("versions").join("services");
    if !version_root.is_dir() {
        anyhow::bail!(
            "artifact versions dir {} is not a directory",
            version_root.display()
        );
    }

    let mut version_paths = Vec::new();
    collect_json_files_recursive(&version_root, &mut version_paths)?;
    version_paths.sort();
    if version_paths.is_empty() {
        anyhow::bail!(
            "artifact versions dir {} has no service version pointer JSON files",
            version_root.display()
        );
    }

    let mut files = Vec::new();
    let mut seen_build_paths = HashSet::new();
    for version_path in version_paths {
        let version_bytes = fs::read(&version_path).map_err(|error| {
            anyhow::anyhow!("failed to read {}: {error}", version_path.display())
        })?;
        let version_value: serde_json::Value =
            serde_json::from_slice(&version_bytes).map_err(|error| {
                anyhow::anyhow!(
                    "failed to parse {} as service version pointer JSON: {error}",
                    version_path.display()
                )
            })?;
        let schema_version = required_string(&version_value, "schemaVersion", &version_path)?;
        if schema_version != SERVICE_VERSION_POINTER_SCHEMA_VERSION {
            anyhow::bail!(
                "{} schemaVersion must be {SERVICE_VERSION_POINTER_SCHEMA_VERSION}",
                version_path.display()
            );
        }
        let service_id = required_string(&version_value, "serviceId", &version_path)?;
        let _version = required_string(&version_value, "version", &version_path)?;
        let build_id = required_string(&version_value, "buildId", &version_path)?;
        files.push((
            relative_artifact_path(artifact_root, &version_path)?,
            version_bytes,
        ));

        let build_path = artifact_root
            .join("builds")
            .join("services")
            .join(service_id_artifact_path(service_id)?)
            .join(format!(
                "{}.json",
                identity_hash_with_label(build_id, "service version pointer buildId")?
            ));
        if seen_build_paths.insert(build_path.clone()) {
            let build_bytes = fs::read(&build_path).map_err(|error| {
                anyhow::anyhow!("failed to read {}: {error}", build_path.display())
            })?;
            files.push((
                relative_artifact_path(artifact_root, &build_path)?,
                build_bytes,
            ));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative_path, bytes) in files {
        hasher.update(relative_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(Sha256::digest(&bytes));
        hasher.update(b"\0");
    }
    Ok(hex::encode(hasher.finalize()))
}

fn artifact_files_fingerprint(root: &Path, current: &Path) -> anyhow::Result<String> {
    let mut files = Vec::new();
    collect_artifact_files(root, current, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative_path, path) in files {
        hasher.update(relative_path.as_bytes());
        hasher.update(b"\0");
        let bytes = fs::read(&path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
        hasher.update(Sha256::digest(&bytes));
        hasher.update(b"\0");
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect_json_files_recursive(dir: &Path, paths: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", dir.display()))?
    {
        let entry = entry
            .map_err(|error| anyhow::anyhow!("failed to read {} entry: {error}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| anyhow::anyhow!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_json_files_recursive(&path, paths)?;
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn collect_artifact_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(current)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| {
            anyhow::anyhow!(
                "failed to read directory entry under {}: {error}",
                current.display()
            )
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| anyhow::anyhow!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_artifact_files(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative_path = relative_artifact_path(root, &path)?;
        files.push((relative_path, path));
    }
    Ok(())
}

fn relative_artifact_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    Ok(path
        .strip_prefix(root)
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to relativize {} against {}: {error}",
                path.display(),
                root.display()
            )
        })?
        .display()
        .to_string())
}

fn required_string<'a>(
    value: &'a serde_json::Value,
    key: &str,
    path: &Path,
) -> anyhow::Result<&'a str> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{} {key} is required", path.display()))
}
