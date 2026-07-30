use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use skiff_compiler_core::id::{SKIFF_STD_PUBLICATION_ID, STD_SOURCE_ALIAS};
use skiff_compiler_input_model::{
    CompilerRawSourceFile, CompilerSourceRole, RawPublicationSourceGraph, RawSourceFileMeta,
};

use crate::{
    error::InputAssemblyError,
    package_config::{PackageApiEntry, PackageManifest, PACKAGE_SEARCH_DIR},
    package_source_helpers::{
        is_official_aggregate_package, package_module_path_for_api_entry,
        source_path_for_api_source_module,
    },
    source_tree::{SourceTree, SourceTreeFile},
    test_rules::module_relative_path_for_test_file,
    CompilerPlatformSources, ManifestOwner, ResolvedPackage,
};

pub fn read_package_sources(
    manifest: &PackageManifest,
    package_root: &Path,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    read_user_package_sources(manifest, package_root, false)
}

/// Reads the explicit source surface for a `service.yml kind: test` compile.
///
/// Ordinary package/service authoring must use [`read_package_sources`], which
/// excludes `*.test.skiff` before reading or parsing it.
pub fn read_test_service_package_sources(
    manifest: &PackageManifest,
    package_root: &Path,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    read_user_package_sources(manifest, package_root, true)
}

fn read_user_package_sources(
    manifest: &PackageManifest,
    package_root: &Path,
    include_test_sources: bool,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    if manifest.provenance.owner == ManifestOwner::CompilerStandardPackage
        || is_official_aggregate_package(manifest.id.as_str())
    {
        return Err(validation_error(vec![format!(
            "package {} requires CompilerPlatformSources authorization",
            manifest.id
        )]));
    }
    read_package_sources_with_module_path(
        manifest,
        package_root,
        |entry| entry.source_module_hint().to_string(),
        module_path_for_package_source,
        include_test_sources,
    )
}

pub fn read_official_package_sources(
    platform_sources: &CompilerPlatformSources,
    manifest: &PackageManifest,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    let package_root = platform_sources
        .authorize_manifest(manifest)
        .map_err(platform_validation_error)?
        .root
        .clone();
    let mut extra_sources = Vec::new();
    if manifest.id.as_str() == SKIFF_STD_PUBLICATION_ID {
        extra_sources.push(PackageSourcePath {
            root: platform_sources.prelude_dir().to_path_buf(),
            relative_path: PathBuf::from("error.skiff"),
        });
    }
    read_package_sources_with_module_path_and_extra_sources(
        manifest,
        &package_root,
        &extra_sources,
        Some(&package_root),
        |entry| package_module_path_for_api_entry(manifest, entry),
        |relative_path| {
            let module_path = module_path_for_package_source(relative_path);
            official_package_source_module_path(manifest.id.as_str(), &module_path)
        },
        false,
    )
}

pub fn read_resolved_package_sources(
    platform_sources: &CompilerPlatformSources,
    package: &ResolvedPackage,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    let package_root = package
        .manifest
        .provenance
        .path
        .parent()
        .expect("package manifest has parent directory");
    if is_official_aggregate_package(package.manifest.id.as_str()) {
        read_official_package_sources(platform_sources, &package.manifest)
    } else {
        read_package_sources(&package.manifest, package_root)
    }
}

fn read_package_sources_with_module_path(
    manifest: &PackageManifest,
    package_root: &Path,
    module_path_for_api_source: impl Fn(&PackageApiEntry) -> String,
    module_path_for_private_source: impl Fn(&Path) -> String,
    include_test_sources: bool,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    read_package_sources_with_module_path_and_extra_sources(
        manifest,
        package_root,
        &[],
        None,
        module_path_for_api_source,
        module_path_for_private_source,
        include_test_sources,
    )
}

fn read_package_sources_with_module_path_and_extra_sources(
    manifest: &PackageManifest,
    package_root: &Path,
    extra_sources: &[PackageSourcePath],
    trusted_source_root: Option<&Path>,
    module_path_for_api_source: impl Fn(&PackageApiEntry) -> String,
    module_path_for_private_source: impl Fn(&Path) -> String,
    include_test_sources: bool,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    let mut paths = Vec::new();
    collect_package_source_paths_with_trust(
        package_root,
        package_root,
        &mut paths,
        trusted_source_root,
    )?;
    let mut source_paths = paths
        .into_iter()
        .filter(|relative_path| include_test_sources || !is_test_skiff_file(relative_path))
        .map(|relative_path| PackageSourcePath {
            root: package_root.to_path_buf(),
            relative_path,
        })
        .collect::<Vec<_>>();
    source_paths.extend(extra_sources.iter().cloned());
    source_paths.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.root.cmp(&right.root))
    });

    let mut missing_api_sources = Vec::new();
    let api_sources = manifest
        .api
        .entries()
        .filter_map(|entry| {
            let relative_path = source_path_for_api_source_module_in_roots(
                manifest.id.as_str(),
                package_root,
                extra_sources,
                entry.source_module_hint(),
            );
            if relative_path.is_none() {
                missing_api_sources.push(format!(
                    "package {} api {} source module {} not found at {}",
                    manifest.id,
                    public_api_label(&entry.public_path_string()),
                    entry.source_module_hint(),
                    package_root
                        .join(source_path_for_module_hint(entry.source_module_hint()))
                        .display()
                ));
            }
            relative_path.map(|source_path| {
                (
                    source_path.relative_path,
                    (
                        module_path_for_api_source(&entry),
                        package_module_path_for_api_entry(manifest, &entry),
                    ),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    if !missing_api_sources.is_empty() {
        return Err(validation_error(missing_api_sources));
    }

    let mut visibility_by_path = BTreeMap::new();
    let files = source_paths
        .into_iter()
        .map(|source_path| {
            let is_test_file = is_test_skiff_file(&source_path.relative_path);
            if !is_test_file {
                reject_reserved_test_module_segment(&source_path.relative_path)?;
            }
            let full_path = source_path.root.join(&source_path.relative_path);
            let read_path = if trusted_source_root.is_some() {
                canonical_contained_source_path(&source_path.root, &full_path)?
            } else {
                full_path.clone()
            };
            let text =
                fs::read_to_string(&read_path).map_err(|source| InputAssemblyError::Read {
                    path: read_path.display().to_string(),
                    source,
                })?;
            let module_path = if let Some((module_path, public_module_path)) =
                api_sources.get(&source_path.relative_path)
            {
                visibility_by_path.insert(
                    source_path.relative_path.clone(),
                    PackageSourceVisibility::Export {
                        public_module_path: public_module_path.clone(),
                    },
                );
                module_path.clone()
            } else {
                visibility_by_path.insert(
                    source_path.relative_path.clone(),
                    PackageSourceVisibility::Private,
                );
                let module_relative_path = if is_test_file {
                    module_relative_path_for_test_file(&source_path.relative_path)
                } else {
                    source_path.relative_path.clone()
                };
                module_path_for_private_source(&module_relative_path)
            };
            Ok(CompilerRawSourceFile {
                meta: RawSourceFileMeta {
                    relative_path: source_path.relative_path,
                    module_path,
                    is_test_file,
                    is_generated: false,
                },
                text,
                role: CompilerSourceRole::Package,
            })
        })
        .collect::<Result<Vec<_>, InputAssemblyError>>()?;

    RawPackagePublicationSources::with_visibility(
        package_root.to_path_buf(),
        files,
        visibility_by_path,
    )
}

fn reject_reserved_test_module_segment(relative_path: &Path) -> Result<(), InputAssemblyError> {
    let implementation_module_path = module_path_for_package_source(relative_path);
    if implementation_module_path
        .split('.')
        .any(|segment| segment == "__test")
    {
        return Err(validation_error(vec![format!(
            "production source {} uses reserved compiler test module segment __test",
            relative_path.display()
        )]));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PackageSourcePath {
    root: PathBuf,
    relative_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSourceVisibility {
    Export { public_module_path: String },
    Private,
}

#[derive(Debug, Clone)]
pub struct RawPackagePublicationSources {
    root: PathBuf,
    files: Vec<CompilerRawSourceFile>,
    visibility_by_path: BTreeMap<PathBuf, PackageSourceVisibility>,
}

impl RawPackagePublicationSources {
    pub fn with_visibility(
        root: PathBuf,
        files: Vec<CompilerRawSourceFile>,
        visibility_by_path: BTreeMap<PathBuf, PackageSourceVisibility>,
    ) -> Result<Self, InputAssemblyError> {
        validate_package_publication_sources(&files, &visibility_by_path)?;
        Ok(Self {
            root,
            files,
            visibility_by_path,
        })
    }

    pub fn into_source_graph(self) -> RawPublicationSourceGraph {
        let Self {
            root,
            files,
            visibility_by_path: _visibility_by_path,
        } = self;
        RawPublicationSourceGraph { root, files }
    }

    pub fn source_tree(&self) -> SourceTree {
        SourceTree {
            root: self.root.clone(),
            sources: self
                .files
                .iter()
                .map(|source| SourceTreeFile {
                    module_path: source.meta.module_path.clone(),
                    file_path: source.meta.relative_path.clone(),
                    is_test_file: source.meta.is_test_file,
                    byte_len: source.text.len() as u64,
                })
                .collect(),
        }
    }

    pub fn files(&self) -> &[CompilerRawSourceFile] {
        &self.files
    }
}

fn validate_package_publication_sources(
    files: &[CompilerRawSourceFile],
    visibility_by_path: &BTreeMap<PathBuf, PackageSourceVisibility>,
) -> Result<(), InputAssemblyError> {
    let file_paths = files
        .iter()
        .map(|source| source.meta.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let mut violations = Vec::new();

    for source in files {
        if !visibility_by_path.contains_key(&source.meta.relative_path) {
            violations.push(format!(
                "{} has no package visibility",
                source.meta.relative_path.display()
            ));
        }
    }

    for (path, visibility) in visibility_by_path {
        if !file_paths.contains(path) {
            violations.push(format!(
                "{} has package visibility but no raw source",
                path.display()
            ));
        }
        if let PackageSourceVisibility::Export { public_module_path } = visibility {
            if public_module_path.is_empty() {
                violations.push(format!("{} has empty public module path", path.display()));
            }
        }
    }

    if violations.is_empty() {
        return Ok(());
    }

    Err(validation_error(violations))
}

fn source_path_for_api_source_module_in_roots(
    package_id: &str,
    package_root: &Path,
    extra_sources: &[PackageSourcePath],
    module: &str,
) -> Option<PackageSourcePath> {
    source_path_for_api_source_module(package_id, package_root, module)
        .map(|relative_path| PackageSourcePath {
            root: package_root.to_path_buf(),
            relative_path,
        })
        .or_else(|| {
            let expected = source_path_for_module_hint(module);
            extra_sources.iter().find_map(|source| {
                (source.relative_path == expected
                    && source.root.join(&source.relative_path).is_file())
                .then(|| source.clone())
            })
        })
}

pub fn collect_package_source_paths(
    package_root: &Path,
    current: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), InputAssemblyError> {
    collect_package_source_paths_with_trust(package_root, current, paths, None)
}

fn collect_package_source_paths_with_trust(
    package_root: &Path,
    current: &Path,
    paths: &mut Vec<PathBuf>,
    trusted_source_root: Option<&Path>,
) -> Result<(), InputAssemblyError> {
    if let Some(trusted_root) = trusted_source_root {
        canonical_contained_source_path(trusted_root, current)?;
    }
    let entries = fs::read_dir(current).map_err(|source| InputAssemblyError::Read {
        path: current.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| InputAssemblyError::Read {
            path: current.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            if !should_skip_package_source_dir(&path) {
                collect_package_source_paths_with_trust(
                    package_root,
                    &path,
                    paths,
                    trusted_source_root,
                )?;
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "skiff")
        {
            paths.push(
                path.strip_prefix(package_root)
                    .expect("package source is below package root")
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn canonical_contained_source_path(
    trusted_root: &Path,
    path: &Path,
) -> Result<PathBuf, InputAssemblyError> {
    let canonical = path
        .canonicalize()
        .map_err(|source| InputAssemblyError::Read {
            path: path.display().to_string(),
            source,
        })?;
    if canonical.starts_with(trusted_root) {
        return Ok(canonical);
    }
    Err(validation_error(vec![format!(
        "platform source {} escapes canonical root {}",
        canonical.display(),
        trusted_root.display()
    )]))
}

pub fn is_test_skiff_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".test.skiff"))
}

pub fn module_path_for_package_source(relative_path: &Path) -> String {
    let mut without_extension = relative_path.to_path_buf();
    without_extension.set_extension("");
    without_extension
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn source_path_for_module_hint(module: &str) -> PathBuf {
    PathBuf::from(module.replace('.', "/")).with_extension("skiff")
}

fn public_api_label(public_path: &str) -> &str {
    if public_path.is_empty() {
        "<root>"
    } else {
        public_path
    }
}

const OFFICIAL_PACKAGE_PRIVATE_MODULE_SEGMENT: &str = "__private";

pub fn official_package_source_module_path(package_id: &str, module_path: &str) -> String {
    debug_assert!(is_official_aggregate_package(package_id));

    let private_prefix = format!("{STD_SOURCE_ALIAS}.{OFFICIAL_PACKAGE_PRIVATE_MODULE_SEGMENT}");
    if module_path == private_prefix || module_path.starts_with(&format!("{private_prefix}.")) {
        module_path.to_string()
    } else if let Some(module_path) =
        module_path.strip_prefix(&format!("{OFFICIAL_PACKAGE_PRIVATE_MODULE_SEGMENT}."))
    {
        format!("{private_prefix}.{module_path}")
    } else if module_path == OFFICIAL_PACKAGE_PRIVATE_MODULE_SEGMENT {
        private_prefix
    } else if module_path.starts_with(&format!("{STD_SOURCE_ALIAS}.")) {
        module_path.to_string()
    } else {
        format!("{STD_SOURCE_ALIAS}.{module_path}")
    }
}

pub fn should_skip_package_source_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == PACKAGE_SEARCH_DIR
        || name == "target"
        || name == "node_modules"
        || name.starts_with('.')
}

fn platform_validation_error(error: crate::CompilerPlatformSourcesError) -> InputAssemblyError {
    validation_error(vec![error.to_string()])
}

fn validation_error(violations: Vec<String>) -> InputAssemblyError {
    InputAssemblyError::Validation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
#[path = "package_sources/tests.rs"]
mod tests;
