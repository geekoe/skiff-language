use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use skiff_compiler_core::id::{SKIFF_STD_PUBLICATION_ID, STD_SOURCE_ALIAS};
use skiff_compiler_input_model::{
    CompilerRawSourceFile, CompilerSourceRole, RawPublicationSourceGraph, RawSourceFileMeta,
    RawSourceOrigin,
};

use crate::{
    error::InputAssemblyError,
    package_config::{PackageApiEntry, PackageManifest, PACKAGE_SEARCH_DIR},
    package_source_helpers::{
        is_official_aggregate_package, package_module_path_for_api_entry,
        source_path_for_api_source_module,
    },
    source_tree::{SourceTree, SourceTreeFile},
    ResolvedPackage,
};

pub fn read_package_sources(
    manifest: &PackageManifest,
    package_root: &Path,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    read_package_sources_with_module_path(
        manifest,
        package_root,
        |entry| entry.source_module_hint().to_string(),
        module_path_for_package_source,
    )
}

pub fn read_official_package_sources(
    manifest: &PackageManifest,
    package_root: &Path,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    let mut extra_sources = Vec::new();
    if manifest.id.as_str() == SKIFF_STD_PUBLICATION_ID {
        extra_sources.push(PackageSourcePath {
            root: default_prelude_dir(),
            relative_path: PathBuf::from("error.skiff"),
        });
    }
    read_package_sources_with_module_path_and_extra_sources(
        manifest,
        package_root,
        &extra_sources,
        |entry| package_module_path_for_api_entry(manifest, entry),
        |relative_path| {
            let module_path = module_path_for_package_source(relative_path);
            official_package_source_module_path(manifest.id.as_str(), &module_path)
        },
    )
}

pub fn read_resolved_package_sources(
    package: &ResolvedPackage,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    let package_root = package
        .manifest
        .provenance
        .path
        .parent()
        .expect("package manifest has parent directory");
    if is_official_aggregate_package(package.manifest.id.as_str()) {
        read_official_package_sources(&package.manifest, package_root)
    } else {
        read_package_sources(&package.manifest, package_root)
    }
}

fn read_package_sources_with_module_path(
    manifest: &PackageManifest,
    package_root: &Path,
    module_path_for_api_source: impl Fn(&PackageApiEntry) -> String,
    module_path_for_private_source: impl Fn(&Path) -> String,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    read_package_sources_with_module_path_and_extra_sources(
        manifest,
        package_root,
        &[],
        module_path_for_api_source,
        module_path_for_private_source,
    )
}

fn read_package_sources_with_module_path_and_extra_sources(
    manifest: &PackageManifest,
    package_root: &Path,
    extra_sources: &[PackageSourcePath],
    module_path_for_api_source: impl Fn(&PackageApiEntry) -> String,
    module_path_for_private_source: impl Fn(&Path) -> String,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    let mut paths = Vec::new();
    collect_package_source_paths(package_root, package_root, &mut paths)?;
    let mut source_paths = paths
        .into_iter()
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
            let full_path = source_path.root.join(&source_path.relative_path);
            let text =
                fs::read_to_string(&full_path).map_err(|source| InputAssemblyError::Read {
                    path: full_path.display().to_string(),
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
                module_path_for_private_source(&source_path.relative_path)
            };
            Ok(CompilerRawSourceFile {
                meta: RawSourceFileMeta {
                    relative_path: source_path.relative_path,
                    module_path,
                    is_test_file: false,
                    is_generated: false,
                    origin: RawSourceOrigin::Package {
                        package_id: manifest.id.to_string(),
                    },
                },
                text,
                role: CompilerSourceRole::Package,
            })
        })
        .collect::<Result<Vec<_>, InputAssemblyError>>()?;

    RawPackagePublicationSources::with_visibility(
        package_root.to_path_buf(),
        manifest.id.to_string(),
        files,
        visibility_by_path,
    )
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
    package_id: String,
    files: Vec<CompilerRawSourceFile>,
    visibility_by_path: BTreeMap<PathBuf, PackageSourceVisibility>,
}

impl RawPackagePublicationSources {
    pub fn with_visibility(
        root: PathBuf,
        package_id: impl Into<String>,
        files: Vec<CompilerRawSourceFile>,
        visibility_by_path: BTreeMap<PathBuf, PackageSourceVisibility>,
    ) -> Result<Self, InputAssemblyError> {
        let package_id = package_id.into();
        validate_package_publication_sources(&package_id, &files, &visibility_by_path)?;
        Ok(Self {
            root,
            package_id,
            files,
            visibility_by_path,
        })
    }

    pub fn into_source_graph(self) -> RawPublicationSourceGraph {
        let Self {
            root,
            package_id: _package_id,
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
    package_id: &str,
    files: &[CompilerRawSourceFile],
    visibility_by_path: &BTreeMap<PathBuf, PackageSourceVisibility>,
) -> Result<(), InputAssemblyError> {
    let file_paths = files
        .iter()
        .map(|source| source.meta.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let mut violations = Vec::new();

    for source in files {
        match &source.meta.origin {
            RawSourceOrigin::Package {
                package_id: source_package_id,
            } if source_package_id == package_id => {}
            RawSourceOrigin::Package {
                package_id: source_package_id,
            } => violations.push(format!(
                "{} belongs to package {source_package_id}, expected {package_id}",
                source.meta.relative_path.display()
            )),
            RawSourceOrigin::Service => violations.push(format!(
                "{} is not a package source",
                source.meta.relative_path.display()
            )),
        }
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
                collect_package_source_paths(package_root, &path, paths)?;
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "skiff")
            && !is_test_skiff_file(&path)
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

fn default_prelude_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("prelude")
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
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;
    use crate::{
        package_config::{PackageApi, PackageManifest},
        ManifestOwner, ManifestProvenance, PublicationApiEntry, PublicationManifest,
    };
    use skiff_compiler_core::id::PublicationId;

    #[test]
    fn official_package_sources_use_std_namespace_and_explicit_private_namespace() {
        let temp = TestDir::new("skiff-package-sources", "official-private-namespace");
        fs::write(
            temp.path().join("http.skiff"),
            r#"
                function request() -> string { return "ok" }
            "#,
        )
        .unwrap();
        fs::write(
            temp.path().join("helper.skiff"),
            r#"
                type HelperState { value: string }
                function helper() -> HelperState {
                  return { value: "internal" }
                }
            "#,
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("__private")).unwrap();
        fs::write(
            temp.path().join("__private").join("secret.skiff"),
            r#"
                function secret() -> string {
                  return "internal"
                }
            "#,
        )
        .unwrap();

        let manifest = official_std_manifest(temp.path().join("package.yml"));
        let sources = read_official_package_sources(&manifest, temp.path()).unwrap();
        let module_paths = sources
            .files()
            .iter()
            .map(|source| source.meta.module_path.as_str())
            .collect::<Vec<_>>();

        assert!(module_paths.contains(&"std.http"));
        assert!(module_paths.contains(&"std.helper"));
        assert!(module_paths.contains(&"std.__private.secret"));
        assert!(
            module_paths
                .iter()
                .all(|module_path| !module_path.contains('/')),
            "official package module paths must be dotted identities: {module_paths:?}"
        );
        assert!(
            !module_paths
                .iter()
                .any(|module_path| module_path.starts_with("skiff.run/std")),
            "canonical package id must not leak into module paths: {module_paths:?}"
        );
    }

    #[test]
    fn official_package_source_module_path_normalizes_std_prefixes() {
        assert_eq!(
            official_package_source_module_path("skiff.run/std", "helper"),
            "std.helper"
        );
        assert_eq!(
            official_package_source_module_path("skiff.run/std", "std.helper"),
            "std.helper"
        );
        assert_eq!(
            official_package_source_module_path("skiff.run/std", "__private.helper"),
            "std.__private.helper"
        );
        assert_eq!(
            official_package_source_module_path("skiff.run/std", "std.__private.helper"),
            "std.__private.helper"
        );
    }

    fn official_std_manifest(path: PathBuf) -> PackageManifest {
        PackageManifest::new(PublicationManifest::new(
            PublicationId::parse("skiff.run/std").unwrap(),
            "1.0.0".to_string(),
            PackageApi::from_entries(vec![PublicationApiEntry::for_source(
                "http.request",
                "http",
                "request",
            )]),
            Vec::new(),
            ManifestProvenance::file(path, ManifestOwner::CompilerStandardPackage),
        ))
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str, name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{prefix}-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("test dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
