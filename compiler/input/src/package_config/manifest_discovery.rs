use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use skiff_compiler_core::id::PublicationId;

use crate::registry_helpers;

use super::{
    manifest_io::{read_package_manifest, read_std_registry},
    manifest_key,
    manifest_validation::PackageManifestOwner,
    PackageConfigError, PackageDependency, PackageManifest, PackageManifestKey,
    PackageResolutionDirs, PACKAGE_CONFIG_FILE,
};

pub(super) fn discover_package_manifests(
    root: &Path,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    discover_package_manifest_baseline(root)
}

pub(super) fn discover_package_manifests_with_dirs(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    discover_package_manifests_with_dependency_dirs(root, package_dirs, &[])
}

pub(super) fn discover_package_manifests_with_dependency_dirs(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
    dependencies: &[PackageDependency],
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    let mut manifests = discover_package_manifest_baseline(root)?;
    discover_package_dependencies_in_stores(package_dirs, dependencies, &mut manifests)?;
    Ok(manifests)
}

fn discover_package_manifest_baseline(
    root: &Path,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    let mut manifests = discover_builtin_std_package_manifests()?;
    let root_manifest_path = root.join(PACKAGE_CONFIG_FILE);
    if root_manifest_path.is_file()
        && !manifests
            .values()
            .any(|manifest| same_manifest_path(&manifest.provenance.path, &root_manifest_path))
    {
        let manifest = read_package_manifest(
            &root_manifest_path,
            PackageManifestOwner::UserOrBuiltinPackage,
        )?;
        insert_manifest(&mut manifests, manifest)?;
    }
    Ok(manifests)
}

pub(super) fn discover_builtin_std_registry_manifests(
    std_dir: &Path,
    registry_path: &Path,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    let registry = read_std_registry(registry_path)?;
    if registry.schema_version.as_deref() != Some("skiff-std-registry-v1") {
        return Err(PackageConfigError::Validation {
            message: format!(
                "{}: schemaVersion must be skiff-std-registry-v1",
                registry_path.display()
            ),
        });
    }

    let mut manifests = BTreeMap::new();
    for package in registry.packages {
        registry_helpers::validate_std_registry_package_id(registry_path, &package.id)
            .map_err(|message| PackageConfigError::Validation { message })?;
        let package_dir = registry_helpers::official_registry_package_dir(
            std_dir,
            registry_path,
            &package.id,
            &package.path,
        )
        .map_err(|message| PackageConfigError::Validation { message })?;
        let manifest_path = package_dir.join(PACKAGE_CONFIG_FILE);
        let manifest = read_package_manifest(
            &manifest_path,
            PackageManifestOwner::CompilerStandardPackage,
        )?;
        if manifest.id.as_str() != package.id {
            return Err(PackageConfigError::Validation {
                message: format!(
                    "{}: std registry maps {} to {}, but package.yml declares id {}",
                    registry_path.display(),
                    package.id,
                    package.path,
                    manifest.id
                ),
            });
        }
        if !manifest.api.entries().any(|entry| {
            builtin_package_api_entry_is_canonical(
                manifest.id.as_str(),
                &entry.public_path_string(),
            )
        }) {
            return Err(PackageConfigError::Validation {
                message: format!(
                    "{}: std package {} must export its canonical module {} or a child module",
                    manifest_path.display(),
                    manifest.id,
                    manifest.id
                ),
            });
        }
        insert_manifest(&mut manifests, manifest)?;
    }
    Ok(manifests)
}

fn discover_builtin_std_package_manifests(
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    let std_dir = default_std_dir();
    let registry_path = std_dir.join("registry.yml");
    if registry_path.is_file() {
        return discover_builtin_std_registry_manifests(&std_dir, &registry_path);
    }
    Ok(BTreeMap::new())
}

fn default_std_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("std")
}

fn builtin_package_api_entry_is_canonical(package_id: &str, path: &str) -> bool {
    let public_path = crate::export_config::package_public_path(package_id, path);
    public_path == package_id || public_path.starts_with(&format!("{package_id}."))
}

fn discover_package_dependencies_in_stores(
    package_dirs: &PackageResolutionDirs,
    dependencies: &[PackageDependency],
    manifests: &mut BTreeMap<PackageManifestKey, PackageManifest>,
) -> Result<(), PackageConfigError> {
    let mut stack = dependencies
        .iter()
        .map(|dependency| (dependency.id.clone(), dependency.version.clone()))
        .collect::<Vec<_>>();
    while let Some((id, version)) = stack.pop() {
        let key = (id.clone(), version.clone());
        if manifests.contains_key(&key) {
            continue;
        }
        let mut discovered = false;
        for package_dir in &package_dirs.package_dirs {
            let root = package_store_dependency_root(package_dir, &id, &version);
            let manifest_path = root.join(PACKAGE_CONFIG_FILE);
            if manifest_path.is_file() {
                discover_package_dependency_at_root(&root, &id, &version, manifests)?;
                discovered = true;
                break;
            }
        }
        if !discovered {
            continue;
        }
        if let Some(manifest) = manifests.get(&key) {
            stack.extend(
                manifest
                    .dependencies
                    .iter()
                    .map(|dependency| (dependency.id.clone(), dependency.version.clone())),
            );
        }
    }
    Ok(())
}

fn discover_package_dependency_at_root(
    root: &Path,
    id: &str,
    version: &str,
    manifests: &mut BTreeMap<PackageManifestKey, PackageManifest>,
) -> Result<(), PackageConfigError> {
    let manifest_path = root.join(PACKAGE_CONFIG_FILE);
    let manifest =
        read_package_manifest(&manifest_path, PackageManifestOwner::UserOrBuiltinPackage)?;
    if manifest.id.as_str() != id || manifest.version != version {
        return Err(PackageConfigError::Validation {
            message: format!(
                "{}: package store path selected {} version {}, but package.yml declares {} version {}",
                manifest_path.display(),
                id,
                version,
                manifest.id,
                manifest.version
            ),
        });
    }
    insert_manifest(manifests, manifest)
}

fn package_store_dependency_root(dir: &Path, id: &str, version: &str) -> std::path::PathBuf {
    let id_dir = PublicationId::parse(id)
        .expect("package dependency id was validated before package store discovery")
        .artifact_path();
    dir.join(id_dir).join(version)
}

fn insert_manifest(
    manifests: &mut BTreeMap<PackageManifestKey, PackageManifest>,
    manifest: PackageManifest,
) -> Result<(), PackageConfigError> {
    let key = manifest_key(&manifest);
    if let Some(existing) = manifests.get(&key) {
        if same_manifest_path(&existing.provenance.path, &manifest.provenance.path) {
            return Ok(());
        }
        return Err(PackageConfigError::Validation {
            message: format!(
                "duplicate package id {} version {}",
                manifest.id, manifest.version
            ),
        });
    }
    manifests.insert(key, manifest);
    Ok(())
}

fn same_manifest_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    fs::canonicalize(left)
        .ok()
        .zip(fs::canonicalize(right).ok())
        .is_some_and(|(left, right)| left == right)
}
