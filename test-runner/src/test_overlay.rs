//! Test-only source overlay compilation.
//!
//! Production source is compiled first and retained verbatim. Test declarations
//! are converted to ordinary private functions in a second package build; the
//! canonical production package reference is never overwritten or retagged.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skiff_artifact_identity::package_artifact_ref;
use skiff_artifact_model::{PackageArtifactRef, PackageCallableId, PackageLocalAbiSymbol};
use skiff_compiler::{PackageCompileError, PackageSourceInput, PublishedPackageArtifact};
use skiff_compiler_input::source_tree::SourceTreeFile;
use skiff_compiler_input::{
    package_config::{
        discover_package_manifests_with_dependency_dirs, package_alias_bindings,
        read_user_package_manifest, PackageConfigError, PackageManifest, PackageResolutionDirs,
        PACKAGE_CONFIG_FILE,
    },
    package_sources::{read_official_package_sources, read_package_sources},
    read_publication_resources, InputAssemblyError, ManifestOwner, PublicationApiEntry,
};
use skiff_compiler_source::{
    source_graph::{CompilerSourceFile, PublicationSourceGraph},
    SourceCompileError,
};
use skiff_syntax::ast::{Block, FunctionDecl, SourceFile, TypeRef};
use thiserror::Error;

use crate::{
    canonical_fixture::PackageTestCase,
    canonical_package::{compile_package_artifact, CanonicalPackageProject},
};

#[derive(Debug, Clone)]
pub struct PackageTestOverlayBinding {
    pub case: PackageTestCase,
    pub public_path: String,
    pub callable_id: PackageCallableId,
}

#[derive(Debug, Clone)]
pub struct PublishedPackageTestOverlay {
    pub production: PackageArtifactRef,
    pub overlay: PublishedPackageArtifact,
    pub bindings: Vec<PackageTestOverlayBinding>,
}

#[derive(Debug, Error)]
pub enum PackageTestOverlayError {
    #[error(transparent)]
    PackageConfig(#[from] PackageConfigError),
    #[error(transparent)]
    Input(#[from] InputAssemblyError),
    #[error(transparent)]
    Source(#[from] SourceCompileError),
    #[error(transparent)]
    Compile(#[from] PackageCompileError),
    #[error("invalid package-test overlay: {0}")]
    Invalid(String),
}

pub fn compile_package_test_overlay(
    package_root: &Path,
    project: &CanonicalPackageProject,
    cases: &[PackageTestCase],
    package_dirs: &[PathBuf],
) -> Result<PublishedPackageTestOverlay, PackageTestOverlayError> {
    if cases.is_empty() {
        return Err(PackageTestOverlayError::Invalid(
            "at least one package test case is required".to_string(),
        ));
    }
    let production = package_artifact_ref(&project.package.artifact)
        .map_err(|error| PackageTestOverlayError::Invalid(error.to_string()))?;
    let (source, manifest) = build_overlay_source(package_root, cases)?;
    let overlay =
        compile_overlay_artifact(package_root, project, package_dirs, &manifest, &source)?;
    let bindings = overlay_bindings(cases, &overlay)?;
    if package_artifact_ref(&project.package.artifact)
        .map_err(|error| PackageTestOverlayError::Invalid(error.to_string()))?
        != production
    {
        return Err(PackageTestOverlayError::Invalid(
            "test overlay rewrote production PackageArtifact identity".to_string(),
        ));
    }
    Ok(PublishedPackageTestOverlay {
        production,
        overlay,
        bindings,
    })
}

fn build_overlay_source(
    package_root: &Path,
    cases: &[PackageTestCase],
) -> Result<(PackageSourceInput, PackageManifest), PackageTestOverlayError> {
    let manifest = read_user_package_manifest(&package_root.join(PACKAGE_CONFIG_FILE))?;
    let raw_sources = match manifest.provenance.owner {
        ManifestOwner::CompilerStandardPackage => {
            read_official_package_sources(&manifest, package_root)?
        }
        ManifestOwner::UserOrBuiltinPackage => read_package_sources(&manifest, package_root)?,
    };
    let mut source_tree = raw_sources.source_tree();
    let raw_graph = raw_sources.into_source_graph();
    let parsed_graph = PublicationSourceGraph::parse_raw_publication_sources(&raw_graph)?;
    let mut compiler_sources = parsed_graph.files().to_vec();
    let mut overlay_manifest = manifest.publication.clone();
    let mut grouped = BTreeMap::<PathBuf, Vec<&PackageTestCase>>::new();
    for case in cases {
        grouped
            .entry(case.relative_path.clone())
            .or_default()
            .push(case);
    }

    for (relative_path, selected) in grouped {
        let module_path = overlay_module_path(
            overlay_manifest.id.as_str(),
            &relative_path,
            &source_tree.sources,
        )?;
        let transformed = package_test_ast_for_cases(
            &selected[0].source_ast,
            selected
                .iter()
                .map(|case| (case.test_index, case.function_name.as_str())),
        );
        compiler_sources.push(CompilerSourceFile::from_parsed_ast(
            relative_path.clone(),
            module_path.clone(),
            true,
            false,
            selected[0].source_text.clone(),
            transformed,
        ));
        source_tree.sources.push(SourceTreeFile {
            module_path: module_path.clone(),
            file_path: relative_path,
            is_test_file: false,
            byte_len: selected[0].source_text.len() as u64,
        });
        overlay_manifest
            .api
            .entries
            .extend(selected.iter().map(|case| {
                PublicationApiEntry::for_source(
                    public_path(case),
                    module_path.clone(),
                    case.function_name.clone(),
                )
            }));
    }
    source_tree.sources.sort_by(|left, right| {
        left.module_path
            .cmp(&right.module_path)
            .then_with(|| left.file_path.cmp(&right.file_path))
    });
    let resources = read_publication_resources(package_root, &overlay_manifest.resources)?;
    let source = PackageSourceInput::new(
        overlay_manifest,
        source_tree,
        PublicationSourceGraph::from_compiler_sources(compiler_sources),
        resources,
    );
    Ok((source, manifest))
}

fn compile_overlay_artifact(
    package_root: &Path,
    project: &CanonicalPackageProject,
    package_dirs: &[PathBuf],
    manifest: &PackageManifest,
    source: &PackageSourceInput,
) -> Result<PublishedPackageArtifact, PackageTestOverlayError> {
    let mut resolution_dirs = package_dirs.to_vec();
    let local_store = package_root.join(".skiff-packages");
    if local_store.is_dir() && !resolution_dirs.contains(&local_store) {
        resolution_dirs.push(local_store);
    }
    let manifests = discover_package_manifests_with_dependency_dirs(
        package_root,
        &PackageResolutionDirs {
            package_dirs: resolution_dirs,
        },
        &manifest.dependencies,
    )?;
    let aliases = package_alias_bindings(&manifest.dependencies, &manifests);
    let dependencies = manifest
        .dependencies
        .iter()
        .map(|dependency| {
            project
                .artifact(&dependency.id, &dependency.version)
                .map(|package| package.artifact.clone())
                .ok_or_else(|| {
                    PackageTestOverlayError::Invalid(format!(
                        "canonical dependency {}@{} is absent from compiled project",
                        dependency.id, dependency.version
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let available = project
        .packages()
        .map(|package| package.artifact.clone())
        .collect::<Vec<_>>();
    Ok(compile_package_artifact(
        source,
        &aliases,
        &dependencies,
        &available,
        &[],
    )?)
}

fn overlay_bindings(
    cases: &[PackageTestCase],
    overlay: &PublishedPackageArtifact,
) -> Result<Vec<PackageTestOverlayBinding>, PackageTestOverlayError> {
    cases
        .iter()
        .map(|case| {
            let public_path = public_path(case);
            let symbol = overlay
                .artifact
                .package_local_abi
                .public_symbols
                .get(&public_path)
                .ok_or_else(|| {
                    PackageTestOverlayError::Invalid(format!(
                        "overlay public path {public_path} was not emitted"
                    ))
                })?;
            let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
                return Err(PackageTestOverlayError::Invalid(format!(
                    "overlay public path {public_path} is not callable"
                )));
            };
            Ok(PackageTestOverlayBinding {
                case: case.clone(),
                public_path,
                callable_id: callable_id.clone(),
            })
        })
        .collect()
}

fn public_path(case: &PackageTestCase) -> String {
    format!(
        "testCases.case{}",
        case.function_name.trim_start_matches("skiffTestCase")
    )
}

fn overlay_module_path(
    package_id: &str,
    test_path: &Path,
    production_sources: &[SourceTreeFile],
) -> Result<String, PackageTestOverlayError> {
    let name = test_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            PackageTestOverlayError::Invalid(format!(
                "test path {} is not valid UTF-8",
                test_path.display()
            ))
        })?;
    let production_name = name.strip_suffix(".test.skiff").ok_or_else(|| {
        PackageTestOverlayError::Invalid(format!(
            "test path {} must end with .test.skiff",
            test_path.display()
        ))
    })?;
    let production_path = test_path.with_file_name(format!("{production_name}.skiff"));
    let base = production_sources
        .iter()
        .find(|source| source.file_path == production_path)
        .map(|source| source.module_path.clone())
        .unwrap_or_else(|| {
            let mut relative = test_path.to_path_buf();
            relative.set_file_name(production_name);
            let path = relative
                .components()
                .filter_map(|part| part.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join(".");
            if package_id == "skiff.run/std" {
                format!("std.{path}")
            } else {
                path
            }
        });
    Ok(format!("{base}.__test"))
}

fn package_test_ast_for_cases<'a>(
    ast: &SourceFile,
    tests: impl IntoIterator<Item = (usize, &'a str)>,
) -> SourceFile {
    let functions = tests
        .into_iter()
        .map(|(test_index, function_name)| {
            let test = ast
                .tests
                .get(test_index)
                .expect("discovered package test case belongs to this AST");
            (
                ast.source_spans.tests.get(test_index).cloned(),
                FunctionDecl {
                    exported: false,
                    name: function_name.to_string(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: TypeRef {
                        name: "void".to_string(),
                    },
                    body: Block {
                        statements: test.body.statements.clone(),
                    },
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                    span: test.span,
                },
            )
        })
        .collect::<Vec<_>>();
    let mut overlay = ast.clone();
    overlay.tests.clear();
    overlay.test_default_run = None;
    overlay.source_spans.tests.clear();
    for (spans, function) in functions {
        if let Some(spans) = spans {
            overlay.source_spans.functions.push(spans);
        }
        overlay.functions.push(function);
    }
    overlay
}
