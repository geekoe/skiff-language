use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use skiff_compiler_core::{
    id::SKIFF_STD_PUBLICATION_ID,
    registry_helpers::{validate_official_registry_package_path, validate_std_registry_package_id},
};
use thiserror::Error;

use crate::{package_config::PackageManifest, ManifestOwner};

const STD_REGISTRY_SCHEMA_VERSION: &str = "skiff-std-registry-v1";

/// Runtime-validated authority for every compiler-owned platform source.
///
/// The root is supplied by a trusted launcher. Construction deliberately has
/// no default and performs no cwd, environment, or executable-path discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerPlatformSources {
    root: PathBuf,
    std_dir: PathBuf,
    prelude_dir: PathBuf,
    registry_path: PathBuf,
    prelude_error_path: PathBuf,
    packages: BTreeMap<String, PlatformPackageSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformPackageSource {
    pub(crate) root: PathBuf,
    pub(crate) manifest_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum CompilerPlatformSourcesError {
    #[error("compiler platform source root must be absolute: {root}")]
    RootNotAbsolute { root: PathBuf },
    #[error("failed to inspect compiler platform source path {path}: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse compiler platform registry {path}: {message}")]
    RegistryParse { path: PathBuf, message: String },
    #[error("invalid compiler platform source layout: {message}")]
    InvalidLayout { message: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlatformRegistry {
    schema_version: String,
    packages: Vec<PlatformRegistryPackage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformRegistryPackage {
    id: String,
    path: String,
}

impl CompilerPlatformSources {
    pub fn new(root: &Path) -> Result<Self, CompilerPlatformSourcesError> {
        if !root.is_absolute() {
            return Err(CompilerPlatformSourcesError::RootNotAbsolute {
                root: root.to_path_buf(),
            });
        }

        let root = canonical_directory(root, "platform source root")?;
        let std_dir = canonical_contained_directory(&root, &root.join("std"), "std directory")?;
        let prelude_dir =
            canonical_contained_directory(&root, &root.join("prelude"), "prelude directory")?;
        let registry_path =
            canonical_contained_file(&std_dir, &std_dir.join("registry.yml"), "std registry")?;
        let prelude_error_path = canonical_contained_file(
            &prelude_dir,
            &prelude_dir.join("error.skiff"),
            "prelude error source",
        )?;

        let packages = load_platform_packages(&root, &std_dir, &registry_path)?;

        Ok(Self {
            root,
            std_dir,
            prelude_dir,
            registry_path,
            prelude_error_path,
            packages,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn std_dir(&self) -> &Path {
        &self.std_dir
    }

    pub fn prelude_dir(&self) -> &Path {
        &self.prelude_dir
    }

    pub fn registry_path(&self) -> &Path {
        &self.registry_path
    }

    pub fn prelude_error_path(&self) -> &Path {
        &self.prelude_error_path
    }

    pub fn official_package_roots(
        &self,
    ) -> Result<impl Iterator<Item = (&str, &Path)>, CompilerPlatformSourcesError> {
        self.revalidate()?;
        Ok(self
            .packages
            .iter()
            .map(|(package_id, source)| (package_id.as_str(), source.root.as_path())))
    }

    pub fn revalidate(&self) -> Result<(), CompilerPlatformSourcesError> {
        let current = Self::new(&self.root)?;
        if current != *self {
            return Err(invalid_layout(format!(
                "platform source provenance changed after validation for {}",
                self.root.display()
            )));
        }
        Ok(())
    }

    /// Reads the canonical prelude content used by compiler identity framing.
    pub fn read_prelude_sources(
        &self,
    ) -> Result<Vec<(PathBuf, String)>, CompilerPlatformSourcesError> {
        self.revalidate()?;
        let mut sources = Vec::new();
        let mut visited_dirs = BTreeSet::new();
        collect_prelude_sources(
            &self.prelude_dir,
            &self.prelude_dir,
            &mut visited_dirs,
            &mut sources,
        )?;
        sources.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(sources)
    }

    pub(crate) fn packages(&self) -> &BTreeMap<String, PlatformPackageSource> {
        &self.packages
    }

    pub(crate) fn authorize_manifest(
        &self,
        manifest: &PackageManifest,
    ) -> Result<&PlatformPackageSource, CompilerPlatformSourcesError> {
        self.revalidate()?;
        if manifest.provenance.owner != ManifestOwner::CompilerStandardPackage
            || manifest.provenance.synthetic
        {
            return Err(invalid_layout(format!(
                "package {} is not a compiler-authorized platform manifest",
                manifest.id
            )));
        }
        let package = self.packages.get(manifest.id.as_str()).ok_or_else(|| {
            invalid_layout(format!(
                "package {} is not declared by {}",
                manifest.id,
                self.registry_path.display()
            ))
        })?;
        let manifest_path = canonical_file(&manifest.provenance.path, "manifest provenance")?;
        if manifest_path != package.manifest_path {
            return Err(invalid_layout(format!(
                "package {} manifest provenance {} does not match authorized path {}",
                manifest.id,
                manifest_path.display(),
                package.manifest_path.display()
            )));
        }
        Ok(package)
    }
}

fn canonical_directory(
    path: &Path,
    description: &str,
) -> Result<PathBuf, CompilerPlatformSourcesError> {
    let canonical = canonicalize(path)?;
    if !canonical.is_dir() {
        return Err(invalid_layout(format!(
            "{description} {} is not a directory",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn load_platform_packages(
    root: &Path,
    std_dir: &Path,
    registry_path: &Path,
) -> Result<BTreeMap<String, PlatformPackageSource>, CompilerPlatformSourcesError> {
    let registry_text = fs::read_to_string(registry_path).map_err(|source| {
        CompilerPlatformSourcesError::Inspect {
            path: registry_path.to_path_buf(),
            source,
        }
    })?;
    let registry = serde_yaml::from_str::<PlatformRegistry>(&registry_text).map_err(|source| {
        CompilerPlatformSourcesError::RegistryParse {
            path: registry_path.to_path_buf(),
            message: source.to_string(),
        }
    })?;
    if registry.schema_version != STD_REGISTRY_SCHEMA_VERSION {
        return Err(invalid_layout(format!(
            "{}: schemaVersion must be {STD_REGISTRY_SCHEMA_VERSION}",
            registry_path.display()
        )));
    }

    let mut packages = BTreeMap::new();
    for package in registry.packages {
        validate_std_registry_package_id(&package.id)
            .map_err(|error| invalid_layout(format!("{}: {error}", registry_path.display())))?;
        validate_official_registry_package_path(&package.id, &package.path)
            .map_err(|error| invalid_layout(format!("{}: {error}", registry_path.display())))?;
        let package_root = canonical_contained_directory(
            root,
            &std_dir.join(&package.path),
            &format!("registry package {}", package.id),
        )?;
        if package.id == SKIFF_STD_PUBLICATION_ID && package_root != std_dir {
            return Err(invalid_layout(format!(
                "{}: {} must resolve to canonical std directory {}, got {}",
                registry_path.display(),
                SKIFF_STD_PUBLICATION_ID,
                std_dir.display(),
                package_root.display()
            )));
        }
        let manifest_path = canonical_contained_file(
            &package_root,
            &package_root.join("package.yml"),
            &format!("registry package {} manifest", package.id),
        )?;
        match packages.entry(package.id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(PlatformPackageSource {
                    root: package_root,
                    manifest_path,
                });
            }
            Entry::Occupied(_) => {
                return Err(invalid_layout(format!(
                    "{}: duplicate registry package id {}",
                    registry_path.display(),
                    package.id
                )));
            }
        }
    }
    if !packages.contains_key(SKIFF_STD_PUBLICATION_ID) {
        return Err(invalid_layout(format!(
            "{}: missing registry package {SKIFF_STD_PUBLICATION_ID}",
            registry_path.display()
        )));
    }
    Ok(packages)
}

fn canonical_file(path: &Path, description: &str) -> Result<PathBuf, CompilerPlatformSourcesError> {
    let canonical = canonicalize(path)?;
    if !canonical.is_file() {
        return Err(invalid_layout(format!(
            "{description} {} is not a file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn canonical_contained_directory(
    root: &Path,
    path: &Path,
    description: &str,
) -> Result<PathBuf, CompilerPlatformSourcesError> {
    let canonical = canonical_directory(path, description)?;
    require_containment(root, &canonical, description)?;
    Ok(canonical)
}

fn canonical_contained_file(
    root: &Path,
    path: &Path,
    description: &str,
) -> Result<PathBuf, CompilerPlatformSourcesError> {
    let canonical = canonical_file(path, description)?;
    require_containment(root, &canonical, description)?;
    Ok(canonical)
}

fn canonicalize(path: &Path) -> Result<PathBuf, CompilerPlatformSourcesError> {
    path.canonicalize()
        .map_err(|source| CompilerPlatformSourcesError::Inspect {
            path: path.to_path_buf(),
            source,
        })
}

fn require_containment(
    root: &Path,
    path: &Path,
    description: &str,
) -> Result<(), CompilerPlatformSourcesError> {
    if path.starts_with(root) {
        return Ok(());
    }
    Err(invalid_layout(format!(
        "{description} {} escapes canonical platform root {}",
        path.display(),
        root.display()
    )))
}

fn invalid_layout(message: String) -> CompilerPlatformSourcesError {
    CompilerPlatformSourcesError::InvalidLayout { message }
}

fn collect_prelude_sources(
    root: &Path,
    dir: &Path,
    visited_dirs: &mut BTreeSet<PathBuf>,
    sources: &mut Vec<(PathBuf, String)>,
) -> Result<(), CompilerPlatformSourcesError> {
    let canonical_dir = canonical_contained_directory(root, dir, "prelude source directory")?;
    if !visited_dirs.insert(canonical_dir.clone()) {
        return Ok(());
    }
    let entries =
        fs::read_dir(&canonical_dir).map_err(|source| CompilerPlatformSourcesError::Inspect {
            path: canonical_dir.clone(),
            source,
        })?;
    for entry in entries {
        let entry = entry.map_err(|source| CompilerPlatformSourcesError::Inspect {
            path: canonical_dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_prelude_sources(root, &path, visited_dirs, sources)?;
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("skiff") {
            continue;
        }
        let canonical = canonical_contained_file(root, &path, "prelude source")?;
        let relative = canonical.strip_prefix(root).map_err(|_| {
            invalid_layout(format!(
                "prelude source {} escaped canonical prelude root {}",
                canonical.display(),
                root.display()
            ))
        })?;
        let text = fs::read_to_string(&canonical).map_err(|source| {
            CompilerPlatformSourcesError::Inspect {
                path: canonical.clone(),
                source,
            }
        })?;
        sources.push((relative.to_path_buf(), text));
    }
    Ok(())
}

#[cfg(test)]
#[path = "platform_sources/tests.rs"]
mod tests;
