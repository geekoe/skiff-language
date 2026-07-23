use std::{collections::BTreeMap, fs, path::Path};

use skiff_compiler_core::id::PublicationId;

use crate::CompilerPlatformSources;

use super::{
    manifest_io::read_package_manifest, manifest_key, manifest_validation::PackageManifestOwner,
    PackageConfigError, PackageDependency, PackageManifest, PackageManifestKey,
    PackageResolutionDirs, PACKAGE_CONFIG_FILE,
};

pub(super) fn discover_package_manifests(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    discover_package_manifest_baseline(platform_sources, root)
}

pub(super) fn discover_package_manifests_with_dirs(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    package_dirs: &PackageResolutionDirs,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    discover_package_manifests_with_dependency_dirs(platform_sources, root, package_dirs, &[])
}

pub(super) fn discover_package_manifests_with_dependency_dirs(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    package_dirs: &PackageResolutionDirs,
    dependencies: &[PackageDependency],
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    let mut manifests = discover_package_manifest_baseline(platform_sources, root)?;
    discover_package_dependencies_in_stores(package_dirs, dependencies, &mut manifests)?;
    Ok(manifests)
}

fn discover_package_manifest_baseline(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    let mut manifests = discover_builtin_std_package_manifests(platform_sources)?;
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

fn discover_builtin_std_package_manifests(
    platform_sources: &CompilerPlatformSources,
) -> Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError> {
    platform_sources
        .revalidate()
        .map_err(platform_source_validation_error)?;
    let mut manifests = BTreeMap::new();
    for (package_id, package) in platform_sources.packages() {
        let manifest_path = &package.manifest_path;
        let mut manifest =
            read_package_manifest(manifest_path, PackageManifestOwner::CompilerStandardPackage)?;
        if package_id == skiff_trusted_registry_contract::TRUSTED_REGISTRY_PACKAGE_ID {
            manifest.publication.api = crate::trusted_registry_native_api();
        }
        if manifest.id.as_str() != package_id {
            return Err(PackageConfigError::Validation {
                message: format!(
                    "{}: platform registry grants {}, but package.yml declares id {}",
                    platform_sources.registry_path().display(),
                    package_id,
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
        platform_sources
            .authorize_manifest(&manifest)
            .map_err(platform_source_validation_error)?;
        insert_manifest(&mut manifests, manifest)?;
    }
    Ok(manifests)
}

fn platform_source_validation_error(
    error: crate::CompilerPlatformSourcesError,
) -> PackageConfigError {
    PackageConfigError::Validation {
        message: error.to_string(),
    }
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
