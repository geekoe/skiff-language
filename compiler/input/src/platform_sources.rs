use std::{
    collections::{btree_map::Entry, BTreeMap},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use skiff_compiler_core::{
    id::SKIFF_STD_PUBLICATION_ID,
    registry_helpers::{validate_official_registry_package_path, validate_std_registry_package_id},
};
use skiff_trusted_registry_contract::TRUSTED_REGISTRY_PACKAGE_ID;
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

/// Immutable compiler-owned source input for one prelude registry build.
///
/// Logical paths are relative to the canonical platform root. The snapshot is
/// fully contained and read before the source crate parses any entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerPlatformSourceSnapshot {
    sources: Box<[(PathBuf, String)]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerPlatformPackageAuthority {
    package_id: &'static str,
    platform_root: PathBuf,
    package_root: PathBuf,
}

impl CompilerPlatformPackageAuthority {
    pub fn package_id(&self) -> &'static str {
        self.package_id
    }

    pub(crate) fn platform_root(&self) -> &Path {
        &self.platform_root
    }

    pub fn package_root(&self) -> &Path {
        &self.package_root
    }
}

impl CompilerPlatformSourceSnapshot {
    pub fn sources(&self) -> &[(PathBuf, String)] {
        &self.sources
    }
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

    pub fn trusted_registry_package_authority(
        &self,
    ) -> Result<CompilerPlatformPackageAuthority, CompilerPlatformSourcesError> {
        self.revalidate()?;
        let package = self
            .packages
            .get(TRUSTED_REGISTRY_PACKAGE_ID)
            .ok_or_else(|| {
                invalid_layout(format!(
                    "{}: missing registry package {TRUSTED_REGISTRY_PACKAGE_ID}",
                    self.registry_path.display()
                ))
            })?;
        Ok(CompilerPlatformPackageAuthority {
            package_id: TRUSTED_REGISTRY_PACKAGE_ID,
            platform_root: self.root.clone(),
            package_root: package.root.clone(),
        })
    }

    /// Captures every official `.skiff` source consumed by PreludeRegistry.
    pub fn prelude_registry_snapshot(
        &self,
    ) -> Result<CompilerPlatformSourceSnapshot, CompilerPlatformSourcesError> {
        self.revalidate()?;
        let mut sources = Vec::new();
        collect_contained_sources(
            &self.prelude_dir,
            &self.prelude_dir,
            Path::new("prelude"),
            false,
            &mut sources,
        )?;
        for (package_id, package) in &self.packages {
            if package_id == TRUSTED_REGISTRY_PACKAGE_ID {
                continue;
            }
            if package_id != SKIFF_STD_PUBLICATION_ID {
                return Err(invalid_layout(format!(
                    "official package {package_id} has no prelude registry logical root"
                )));
            }
            collect_contained_sources(
                &package.root,
                &package.root,
                Path::new("std"),
                true,
                &mut sources,
            )?;
        }
        sources.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(CompilerPlatformSourceSnapshot {
            sources: sources.into_boxed_slice(),
        })
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
        if package.id != TRUSTED_REGISTRY_PACKAGE_ID {
            validate_std_registry_package_id(&package.id)
                .map_err(|error| invalid_layout(format!("{}: {error}", registry_path.display())))?;
        }
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
    if root.join("registry").is_dir() {
        let trusted_registry_root = canonical_contained_directory(
            root,
            &root.join("registry"),
            "trusted registry package",
        )?;
        let trusted_registry_manifest = canonical_contained_file(
            &trusted_registry_root,
            &trusted_registry_root.join("package.yml"),
            "trusted registry package manifest",
        )?;
        packages.insert(
            TRUSTED_REGISTRY_PACKAGE_ID.to_string(),
            PlatformPackageSource {
                root: trusted_registry_root,
                manifest_path: trusted_registry_manifest,
            },
        );
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

fn collect_contained_sources(
    root: &Path,
    dir: &Path,
    logical_root: &Path,
    skip_test_sources: bool,
    sources: &mut Vec<(PathBuf, String)>,
) -> Result<(), CompilerPlatformSourcesError> {
    let canonical_dir = canonical_contained_directory(root, dir, "platform source directory")?;
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
        if path.extension().and_then(|extension| extension.to_str()) != Some("skiff") {
            continue;
        }
        let Some(file_stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            return Err(invalid_layout(format!(
                "platform source {} has a non-UTF-8 file name",
                path.display()
            )));
        };
        if skip_test_sources && (file_stem.ends_with(".test") || file_stem.ends_with("_test")) {
            continue;
        }
        let canonical = canonical_contained_file(root, &path, "platform source")?;
        let file_name = path.file_name().ok_or_else(|| {
            invalid_layout(format!(
                "platform source {} has no logical file name",
                path.display()
            ))
        })?;
        let text = fs::read_to_string(&canonical).map_err(|source| {
            CompilerPlatformSourcesError::Inspect {
                path: canonical.clone(),
                source,
            }
        })?;
        sources.push((logical_root.join(file_name), text));
    }
    Ok(())
}

#[cfg(test)]
#[path = "platform_sources/tests.rs"]
mod tests;
