use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    emission::artifact::PublishedFileIrArtifact,
    input::{
        compile_input::{PackagePublicationInput, PublicationInput},
        source_graph::{CompilerSourceFile, Publication, PublicationSourceGraph},
        ManifestOwner, ManifestProvenance, PackageConfigError, PackageDependency,
        PackageManifestKey, PackageResolutionDirs, PublicationApiEntry, PublicationCompilePolicy,
        PublicationManifest, ResolvedPackage, ResolvedPackageGraph, SourceTree, SourceTreeFile,
    },
    shared::publication_error::PublicationError,
    source_compile::{
        CompileParsedPublicationSourcesInput, PublicationTypeSymbolIndex,
        SourceConfigAndEffectMetadata, SourceConfigAndEffectMetadataBatchInput,
        SourceConfigAndEffectMetadataInput, SourceEffectMetadata, SourceSymbolKey,
    },
};
use skiff_compiler_compiled::{
    projection_input::{build_package_projection_inputs, build_projection_input},
    CompiledPublication, PackagePublication,
};
use skiff_compiler_core::artifact::{
    ConfigAndEffectMetadata, PackageDependencyConstraint, PackageUnit,
};
use skiff_compiler_projection_input::{
    ConfigRequirementAccessProjection, ConfigRequirementDependencyStepProjection,
    ConfigRequirementProjection, ConfigRequirementProvenanceProjection,
    ConfigRequirementPublicationProjection, ConfigRequirementScopeProjection,
    ConfigRequirementSetProjection, ConfigRequirementsSeed, ConfigSourcePositionProjection,
    ConfigSourceSpanProjection,
};
use skiff_syntax::ast::{Expr, SourceFile as AstSourceFile};
use skiff_syntax::error::CompileError;

pub mod package_test_artifacts;
mod package_units;
pub mod project_fixtures;
pub mod service_test_artifacts;

pub const PACKAGE_CONFIG_FILE: &str = crate::input::PACKAGE_CONFIG_FILE;

pub use package_test_artifacts::{
    list_package_test_assemblies, write_package_test_artifact_root, TestPackageTestArtifactInput,
    TestPackageTestArtifactOutput, TestPackageTestDependencyPackageInput,
    TestPackageTestEntrypointInput, TestPackageTestFileIrArtifact, TestPackageTestPointer,
};
pub use service_test_artifacts::{
    write_test_service_artifact_root, TestServiceArtifactError, TestServiceArtifactInput,
    TestServiceArtifactOutput, TestServiceFileIrArtifact,
};

pub fn compile_source_file_ir_artifact_for_test(
    source: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
) -> skiff_syntax::error::Result<PublishedFileIrArtifact> {
    let source_path = source_path.into();
    let module_path = module_path.into();
    let role = role.into();
    let unit = crate::pipeline::compile_source_file_ir_unit(
        source,
        source_path.clone(),
        module_path.clone(),
        role.clone(),
    )?;
    Ok(published_file_ir_artifact_for_test(
        &unit,
        source_path,
        module_path,
        role,
    ))
}

fn published_file_ir_artifact_for_test(
    unit: &skiff_compiler_lowering::file_ir::FileIrUnit,
    source_path: String,
    module_path: String,
    role: String,
) -> PublishedFileIrArtifact {
    crate::emission::file_ir_artifacts::published_file_ir_artifact_from_unit(
        unit,
        source_path,
        module_path,
        role,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestPackageManifest {
    pub id: String,
    pub version: String,
    pub api: Vec<TestPackageApiEntry>,
    pub dependencies: Vec<PackageDependency>,
    pub path: PathBuf,
    pub synthetic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestPackageApiEntry {
    pub path: String,
    pub module: String,
    pub symbol: String,
}

impl TestPackageApiEntry {
    pub fn module(path: impl Into<String>, module: impl Into<String>) -> Self {
        let path = path.into();
        let module = module.into();
        let symbol = path
            .rsplit('.')
            .find(|segment| !segment.is_empty())
            .or_else(|| module.rsplit('.').next())
            .unwrap_or("Api")
            .to_string();
        Self::source(path, module, symbol)
    }

    pub fn source(
        path: impl Into<String>,
        module: impl Into<String>,
        symbol: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            module: module.into(),
            symbol: symbol.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResolvedPackage {
    pub manifest: TestPackageManifest,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct TestPackageUnitArtifact {
    pub package_id: String,
    pub package_version: String,
    pub package_dependencies: Vec<PackageDependencyConstraint>,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub unit: PackageUnit,
}

#[derive(Debug, Clone)]
pub struct TestPackageCompiledArtifacts {
    pub file_ir_artifacts: Vec<PublishedFileIrArtifact>,
    pub config_and_effect_metadata: ConfigAndEffectMetadata,
    pub package_unit: Option<PackageUnit>,
}

#[derive(Debug, Clone)]
pub struct TestPackageCompiledArtifactsWithEntrypointMetadata {
    pub compiled: TestPackageCompiledArtifacts,
    pub entrypoint_config_and_effect_metadata: Vec<ConfigAndEffectMetadata>,
}

#[derive(Debug, Clone)]
pub struct TestPackageDependencyPublications {
    package_publications: Vec<PackagePublication>,
}

impl TestPackageDependencyPublications {
    pub fn empty() -> Self {
        Self {
            package_publications: Vec::new(),
        }
    }

    fn as_slice(&self) -> &[PackagePublication] {
        &self.package_publications
    }
}

impl Default for TestPackageDependencyPublications {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone)]
pub struct TestCompilerSourceFile {
    pub relative_path: PathBuf,
    pub module_path: String,
    pub is_api: bool,
    pub is_test_file: bool,
    pub text: String,
}

pub use crate::input::test_rules::FriendProductionMatch;

/// Options for [`compile_ast_file_ir_for_test`].
///
/// All fields default to empty, so callers that need no extra context can use
/// `FileIrCompileOptions::default()`.
pub struct FileIrCompileOptions<'a> {
    /// Package-alias bindings passed to the lowering context.
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    /// External type symbols (name → "module.Symbol" path) available during lowering.
    pub external_type_symbols: &'a BTreeMap<String, String>,
    /// Additional publication-DB source files (module_path, AST) needed for db-entity
    /// resolution in the file under test.
    pub publication_db_sources: &'a [(String, AstSourceFile)],
}

impl Default for FileIrCompileOptions<'_> {
    fn default() -> Self {
        static EMPTY_ALIASES: std::sync::LazyLock<BTreeMap<String, Vec<String>>> =
            std::sync::LazyLock::new(BTreeMap::new);
        static EMPTY_EXTERNAL: std::sync::LazyLock<BTreeMap<String, String>> =
            std::sync::LazyLock::new(BTreeMap::new);
        Self {
            package_aliases: &EMPTY_ALIASES,
            external_type_symbols: &EMPTY_EXTERNAL,
            publication_db_sources: &[],
        }
    }
}

/// Compile a pre-parsed AST into a [`PublishedFileIrArtifact`] for use in tests.
///
/// Extra compilation context is supplied via [`FileIrCompileOptions`].  Pass
/// `FileIrCompileOptions::default()` when no aliases, external types, or
/// publication-DB sources are needed.
pub fn compile_ast_file_ir_for_test(
    source_text: &str,
    source_path: impl Into<String>,
    module_path: impl Into<String>,
    role: impl Into<String>,
    ast: AstSourceFile,
    options: FileIrCompileOptions<'_>,
) -> skiff_syntax::error::Result<PublishedFileIrArtifact> {
    let source_path = source_path.into();
    let module_path = module_path.into();
    let role = role.into();
    if options.external_type_symbols.is_empty() {
        let additional_sources = options
            .publication_db_sources
            .iter()
            .map(|(module_path, ast)| {
                CompilerSourceFile::from_parsed_ast(
                    PathBuf::from(format!("{}.skiff", module_path.replace('.', "/"))),
                    module_path.clone(),
                    false,
                    false,
                    String::new(),
                    ast.clone(),
                )
            })
            .collect();
        let unit = crate::pipeline::compile_parsed_source_file_ir_unit_with_additional_sources(
            ast,
            source_text,
            source_path.clone(),
            module_path.clone(),
            role.clone(),
            options.package_aliases,
            additional_sources,
        )?;
        return Ok(published_file_ir_artifact_for_test(
            &unit,
            source_path,
            module_path,
            role,
        ));
    }

    let external_type_symbols = external_type_symbol_refs(options.external_type_symbols);
    let publication_db_metadata = crate::source_compile::publication_db_metadata_index(
        options
            .publication_db_sources
            .iter()
            .map(|(mp, ast)| (mp.as_str(), ast)),
        options.package_aliases,
        &external_type_symbols,
    )?;
    let unit =
        skiff_compiler_lowering::source_file_lowering::compile_parsed_source_file_ir_unit_with_publication_db_context(
            ast,
            source_text,
            source_path.clone(),
            module_path.clone(),
            role.clone(),
            options.package_aliases,
            &external_type_symbols,
            &publication_db_metadata,
        )?;
    Ok(published_file_ir_artifact_for_test(
        &unit,
        source_path,
        module_path,
        role,
    ))
}

fn external_type_symbol_refs(symbols: &BTreeMap<String, String>) -> PublicationTypeSymbolIndex {
    let mut index = PublicationTypeSymbolIndex::default();
    for (name, symbol) in symbols {
        let Some((module_path, symbol)) = symbol.rsplit_once('.') else {
            continue;
        };
        index.insert_resolved_symbol(name, SourceSymbolKey::new(module_path, symbol));
    }
    index
}

/// Compile already-parsed package AST fixtures through the low-level File IR path.
///
/// This helper intentionally bypasses publication root-ref and manifest API seed
/// behavior. Use
/// [`compile_package_ast_file_ir_artifacts_with_dependency_publications_unit_and_metadata_for_test`]
/// when package unit metadata or production publication semantics are under test.
pub fn compile_parsed_only_package_ast_file_ir_artifacts_for_test(
    package_id: &str,
    dependencies: &[PackageDependency],
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<PublishedFileIrArtifact>, PublicationError> {
    compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test(
        package_id,
        dependencies,
        package_root,
        sources,
        package_aliases,
    )
    .map(|compiled| compiled.file_ir_artifacts)
}

/// Compile already-parsed package AST fixtures and return File IR metadata only.
///
/// This is a parsed-only File IR helper: it sets `production_sources` empty and
/// `publication_api` to `None`, so it does not cover publication root-ref or
/// manifest API seed behavior.
pub fn compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test(
    package_id: &str,
    dependencies: &[PackageDependency],
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test_inner(
        package_id,
        dependencies,
        package_root,
        sources,
        package_aliases,
        None,
    )
}

fn compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test_inner(
    package_id: &str,
    dependencies: &[PackageDependency],
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    package_facts: Option<&[crate::source_compile::SourceCompilePackageFacts<'_>]>,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    let sources = compiler_sources_from_test_sources(sources);
    let parsed_sources =
        crate::source_compile::parsed_sources::parse_publication_sources(package_root, &sources)?;
    crate::source_compile::package_rules::validate_package_sources(
        package_id,
        dependencies,
        package_root,
        &parsed_sources,
    )?;
    let compiled = crate::pipeline::compile_parsed_publication_sources(
        CompileParsedPublicationSourcesInput {
            parsed_sources,
            // Parsed-only File IR tests provide already-parsed source fixtures and
            // deliberately do not model publication root-ref/API seed behavior.
            production_sources: Vec::new(),
            diagnostic_root: package_root,
            publication_api: None,
            package_aliases,
            package_dependencies: dependencies,
            package_facts,
            service_dependencies: Default::default(),
            service_ingress: None,
            policy: PublicationCompilePolicy::Package { package_id },
        },
    )?;
    test_package_compiled_artifacts(compiled, None)
}

/// Compile already-parsed package AST fixtures with dependency facts through the
/// low-level File IR path.
///
/// This helper is parsed-only and does not exercise production publication
/// root-ref/API seed behavior.
pub fn compile_parsed_only_package_ast_file_ir_artifacts_with_dependency_publications_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<Vec<PublishedFileIrArtifact>, PublicationError> {
    compile_parsed_only_package_ast_file_ir_artifacts_with_dependency_publications_and_metadata_for_test(
        manifest,
        package_root,
        sources,
        package_aliases,
        dependency_packages,
        available,
    )
    .map(|compiled| compiled.file_ir_artifacts)
}

/// Compile already-parsed package AST fixtures with dependency facts and return
/// File IR metadata only.
///
/// This helper is parsed-only and does not exercise production publication
/// root-ref/API seed behavior.
pub fn compile_parsed_only_package_ast_file_ir_artifacts_with_dependency_publications_and_metadata_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    let dependency_publications =
        compile_package_dependency_publications_for_test(manifest, dependency_packages, available)?;
    compile_parsed_only_package_ast_file_ir_artifacts_with_compiled_dependency_publications_and_metadata_for_test(
        manifest,
        package_root,
        sources,
        package_aliases,
        &dependency_publications,
    )
}

pub fn compile_parsed_only_package_ast_file_ir_artifacts_with_compiled_dependency_publications_and_metadata_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    compile_package_ast_with_dependency_publications_inner_for_test(
        manifest,
        package_root,
        sources,
        package_aliases,
        dependency_publications,
        false,
    )
}

pub fn compile_parsed_only_package_ast_file_ir_artifacts_with_compiled_dependency_publications_entrypoint_metadata_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
    entrypoint_function_names: &[String],
) -> Result<TestPackageCompiledArtifactsWithEntrypointMetadata, PublicationError> {
    let compiler_sources = compiler_sources_from_test_sources(sources);
    let production_sources = compiler_sources
        .iter()
        .filter(|source| !source.is_test_file)
        .cloned()
        .collect::<Vec<_>>();
    let parsed_sources = crate::source_compile::parsed_sources::parse_publication_sources(
        package_root,
        &compiler_sources,
    )?;
    crate::source_compile::package_rules::validate_package_sources(
        manifest.id.as_str(),
        &manifest.dependencies,
        package_root,
        &parsed_sources,
    )?;
    let package_facts = crate::pipeline::source_compile_package_facts_from_publications(
        dependency_publications.as_slice(),
    );
    let dependency_config_facts =
        dependency_package_config_facts_from_source_package_facts(&package_facts);
    let entrypoint_config_and_effect_metadata =
        crate::source_compile::source_config_and_effect_metadata_batches_from_parsed_sources(
            SourceConfigAndEffectMetadataBatchInput {
                diagnostic_root: package_root,
                parsed_sources: &parsed_sources,
                production_sources: &production_sources,
                package_dependencies: &manifest.dependencies,
                dependency_package_config_facts: Some(&dependency_config_facts),
                policy: PublicationCompilePolicy::Package {
                    package_id: &manifest.id,
                },
                publication_api: None,
                entrypoint_function_names,
            },
        )?
        .iter()
        .map(config_and_effect_metadata_from_source_metadata)
        .collect::<Result<Vec<_>, _>>()?;
    let compiled = crate::pipeline::compile_parsed_publication_sources(
        CompileParsedPublicationSourcesInput {
            parsed_sources,
            production_sources: Vec::new(),
            diagnostic_root: package_root,
            publication_api: None,
            package_aliases,
            package_dependencies: &manifest.dependencies,
            package_facts: Some(&package_facts),
            service_dependencies: Default::default(),
            service_ingress: None,
            policy: PublicationCompilePolicy::Package {
                package_id: &manifest.id,
            },
        },
    )?;
    Ok(TestPackageCompiledArtifactsWithEntrypointMetadata {
        compiled: test_package_compiled_artifacts(compiled, None)?,
        entrypoint_config_and_effect_metadata,
    })
}

pub fn compile_package_ast_file_ir_artifacts_with_dependency_publications_unit_and_metadata_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    let dependency_publications =
        compile_package_dependency_publications_for_test(manifest, dependency_packages, available)?;
    compile_package_ast_file_ir_artifacts_with_compiled_dependency_publications_unit_and_metadata_for_test(
        manifest,
        package_root,
        sources,
        package_aliases,
        &dependency_publications,
    )
}

pub fn compile_package_ast_file_ir_artifacts_with_compiled_dependency_publications_unit_and_metadata_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    compile_package_ast_with_dependency_publications_inner_for_test(
        manifest,
        package_root,
        sources,
        package_aliases,
        dependency_publications,
        true,
    )
}

pub fn compile_package_ast_config_and_effect_metadata_with_dependency_publications_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<ConfigAndEffectMetadata, PublicationError> {
    let dependency_publications =
        compile_package_dependency_publications_for_test(manifest, dependency_packages, available)?;
    compile_package_ast_config_and_effect_metadata_with_compiled_dependency_publications_for_test(
        manifest,
        package_root,
        sources,
        package_aliases,
        &dependency_publications,
    )
}

pub fn compile_package_ast_config_and_effect_metadata_with_compiled_dependency_publications_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
) -> Result<ConfigAndEffectMetadata, PublicationError> {
    // Keep this helper signature aligned with the File IR compile helpers.
    // Config metadata is collected before lowering, so alias bindings are not
    // part of this metadata-only path today.
    let _ = package_aliases;
    let compiler_sources = compiler_sources_from_test_sources(sources);
    let production_sources = compiler_sources
        .iter()
        .filter(|source| !source.is_test_file)
        .cloned()
        .collect::<Vec<_>>();
    let parsed_sources = crate::source_compile::parsed_sources::parse_publication_sources(
        package_root,
        &compiler_sources,
    )?;
    let package_facts = crate::pipeline::source_compile_package_facts_from_publications(
        dependency_publications.as_slice(),
    );
    let dependency_config_facts =
        dependency_package_config_facts_from_source_package_facts(&package_facts);
    let source_metadata =
        crate::source_compile::source_config_and_effect_metadata_from_parsed_sources(
            SourceConfigAndEffectMetadataInput {
                diagnostic_root: package_root,
                parsed_sources: &parsed_sources,
                production_sources: &production_sources,
                package_dependencies: &manifest.dependencies,
                dependency_package_config_facts: Some(&dependency_config_facts),
                policy: PublicationCompilePolicy::Package {
                    package_id: &manifest.id,
                },
                publication_api: None,
            },
        )?;
    config_and_effect_metadata_from_source_metadata(&source_metadata)
}

pub fn compile_package_ast_config_and_effect_metadata_batch_with_compiled_dependency_publications_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
    entrypoint_function_names: &[String],
) -> Result<Vec<ConfigAndEffectMetadata>, PublicationError> {
    // Keep this helper signature aligned with the File IR compile helpers.
    // Config metadata is collected before lowering, so alias bindings are not
    // part of this metadata-only path today.
    let _ = package_aliases;
    let compiler_sources = compiler_sources_from_test_sources(sources);
    let production_sources = compiler_sources
        .iter()
        .filter(|source| !source.is_test_file)
        .cloned()
        .collect::<Vec<_>>();
    let parsed_sources = crate::source_compile::parsed_sources::parse_publication_sources(
        package_root,
        &compiler_sources,
    )?;
    let package_facts = crate::pipeline::source_compile_package_facts_from_publications(
        dependency_publications.as_slice(),
    );
    let dependency_config_facts =
        dependency_package_config_facts_from_source_package_facts(&package_facts);
    crate::source_compile::source_config_and_effect_metadata_batches_from_parsed_sources(
        SourceConfigAndEffectMetadataBatchInput {
            diagnostic_root: package_root,
            parsed_sources: &parsed_sources,
            production_sources: &production_sources,
            package_dependencies: &manifest.dependencies,
            dependency_package_config_facts: Some(&dependency_config_facts),
            policy: PublicationCompilePolicy::Package {
                package_id: &manifest.id,
            },
            publication_api: None,
            entrypoint_function_names,
        },
    )?
    .iter()
    .map(config_and_effect_metadata_from_source_metadata)
    .collect()
}

fn compile_package_ast_with_dependency_publications_inner_for_test(
    manifest: &TestPackageManifest,
    package_root: &Path,
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_publications: &TestPackageDependencyPublications,
    include_package_unit: bool,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    let compiler_sources = compiler_sources_from_test_sources(sources);
    if include_package_unit {
        let publication = test_publication_for_compiler_sources(
            manifest,
            package_root,
            &compiler_sources,
            dependency_publications.as_slice(),
        );
        let input = PublicationInput::Package(PackagePublicationInput::new(
            &publication,
            package_aliases,
            manifest.id.as_str(),
        ));
        let compiled = crate::pipeline::compile_publication_with_package_publications(
            input,
            dependency_publications.as_slice(),
        )?;
        let package_unit = Some(package_unit_from_compiled_package_for_test(
            &publication,
            &compiled,
            dependency_publications.as_slice(),
        )?);
        return test_package_compiled_artifacts(compiled, package_unit);
    }
    let package_facts = crate::pipeline::source_compile_package_facts_from_publications(
        dependency_publications.as_slice(),
    );
    compile_parsed_only_package_ast_file_ir_artifacts_with_metadata_for_test_inner(
        &manifest.id,
        &manifest.dependencies,
        package_root,
        sources,
        package_aliases,
        Some(&package_facts),
    )
}

fn compiler_sources_from_test_sources(
    sources: &[(TestCompilerSourceFile, AstSourceFile)],
) -> Vec<CompilerSourceFile> {
    sources
        .iter()
        .map(|(source, ast)| {
            CompilerSourceFile::from_parsed_ast(
                source.relative_path.clone(),
                source.module_path.clone(),
                source.is_api,
                source.is_test_file,
                source.text.clone(),
                ast.clone(),
            )
        })
        .collect()
}

fn test_package_compiled_artifacts(
    compiled: CompiledPublication,
    package_unit: Option<PackageUnit>,
) -> Result<TestPackageCompiledArtifacts, PublicationError> {
    let projection_input = build_projection_input(&compiled);
    let projection_view = projection_input.view();
    let config_projection = skiff_compiler_projection::project_config_projection(
        projection_view.source().config_requirements(),
    )?;
    let config_and_effect_metadata =
        skiff_compiler_projection::package_unit_artifacts::config_and_effect_metadata_from_config_projection(
            &config_projection,
        );
    Ok(TestPackageCompiledArtifacts {
        file_ir_artifacts:
            crate::emission::file_ir_artifacts::published_file_ir_artifacts_from_projection_input(
                projection_view,
            )?,
        config_and_effect_metadata,
        package_unit,
    })
}

fn config_and_effect_metadata_from_source_metadata(
    metadata: &SourceConfigAndEffectMetadata,
) -> Result<ConfigAndEffectMetadata, PublicationError> {
    match metadata.effects() {
        SourceEffectMetadata::Empty => {}
    }
    let config = metadata.config();
    let requirements = ConfigRequirementsSeed::new(
        config_requirement_set_projection(config.legacy_config_projection_requirements()),
        config_requirement_set_projection(config.own_config_requirements()),
        config_requirement_set_projection(config.dependency_config_requirements()),
        config_requirement_set_projection(config.effective_config_requirements()),
    );
    let config_projection = skiff_compiler_projection::project_config_projection(&requirements)?;
    Ok(
        skiff_compiler_projection::package_unit_artifacts::config_and_effect_metadata_from_config_projection(
            &config_projection,
        ),
    )
}

fn dependency_package_config_facts_from_source_package_facts<'facts>(
    package_facts: &'facts [crate::source_compile::SourceCompilePackageFacts<'_>],
) -> Vec<crate::source_compile::DependencyPackageConfigFacts<'facts>> {
    package_facts
        .iter()
        .map(|package| crate::source_compile::DependencyPackageConfigFacts {
            id: package.id(),
            version: package.version(),
            dependencies: package
                .dependencies()
                .iter()
                .map(crate::source_compile::ConfigRequirementDependencyStep::from_package_dependency_fact)
                .collect(),
            own_config_requirements: package.compile_model().own_config_requirements(),
        })
        .collect()
}

fn config_requirement_set_projection(
    set: &crate::source_compile::ConfigRequirementSet,
) -> ConfigRequirementSetProjection {
    ConfigRequirementSetProjection::new(
        set.requirements()
            .iter()
            .map(config_requirement_projection)
            .collect(),
    )
}

fn config_requirement_projection(
    requirement: &crate::source_compile::ConfigRequirement,
) -> ConfigRequirementProjection {
    ConfigRequirementProjection {
        scope: config_requirement_scope_projection(requirement.scope()),
        path: requirement.path().to_string(),
        access: config_requirement_access_projection(requirement.access()),
        provenances: requirement
            .provenances()
            .iter()
            .map(|provenance| ConfigRequirementProvenanceProjection {
                source_path: provenance.source_path().to_string(),
                source_span: provenance.source_span().map(config_source_span_projection),
                declaring_publication: provenance.declaring_publication().map(|publication| {
                    ConfigRequirementPublicationProjection {
                        id: publication.id().to_string(),
                        version: publication.version().to_string(),
                    }
                }),
                dependency_path: provenance
                    .dependency_path()
                    .iter()
                    .map(|step| ConfigRequirementDependencyStepProjection {
                        id: step.id().to_string(),
                        version: step.version().to_string(),
                        alias: step.alias().map(str::to_string),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn config_requirement_scope_projection(
    scope: &crate::source_compile::ConfigRequirementScope,
) -> ConfigRequirementScopeProjection {
    match scope {
        crate::source_compile::ConfigRequirementScope::Service => {
            ConfigRequirementScopeProjection::Service
        }
        crate::source_compile::ConfigRequirementScope::Package { package_id } => {
            ConfigRequirementScopeProjection::Package {
                package_id: package_id.clone(),
            }
        }
    }
}

fn config_requirement_access_projection(
    access: &crate::source_compile::ConfigRequirementAccess,
) -> ConfigRequirementAccessProjection {
    match access {
        crate::source_compile::ConfigRequirementAccess::Require { ty } => {
            ConfigRequirementAccessProjection::Require { ty: ty.clone() }
        }
        crate::source_compile::ConfigRequirementAccess::Optional { ty } => {
            ConfigRequirementAccessProjection::Optional { ty: ty.clone() }
        }
        crate::source_compile::ConfigRequirementAccess::Has => {
            ConfigRequirementAccessProjection::Has
        }
    }
}

fn config_source_span_projection(
    span: crate::source_compile::ConfigSourceSpan,
) -> ConfigSourceSpanProjection {
    ConfigSourceSpanProjection {
        start: ConfigSourcePositionProjection {
            line: span.start.line,
            column: span.start.column,
            offset: span.start.offset,
        },
        end: ConfigSourcePositionProjection {
            line: span.end.line,
            column: span.end.column,
            offset: span.end.offset,
        },
    }
}

fn test_publication_for_compiler_sources(
    manifest: &TestPackageManifest,
    package_root: &Path,
    compiler_sources: &[CompilerSourceFile],
    _package_publications: &[PackagePublication],
) -> Publication {
    let source_tree = SourceTree {
        root: package_root.to_path_buf(),
        sources: compiler_sources
            .iter()
            .map(|source| SourceTreeFile {
                module_path: source.module_path.clone(),
                file_path: source.relative_path.clone(),
                is_test_file: source.is_test_file,
                byte_len: source.text.len() as u64,
            })
            .collect(),
    };
    Publication::new(
        manifest.clone().into_internal().into_publication(),
        source_tree,
        PublicationSourceGraph::from_compiler_sources(compiler_sources.to_vec()),
        ResolvedPackageGraph::declared_only(manifest.dependencies.clone()),
    )
}

fn package_unit_from_compiled_package_for_test(
    publication: &Publication,
    compiled: &CompiledPublication,
    _package_publications: &[PackagePublication],
) -> Result<PackageUnit, PublicationError> {
    let prelude_projection = crate::shared::prelude_registry::projection_prelude_context();
    let projection_context = skiff_compiler_projection::PackageProjectionContext::new(
        skiff_compiler_projection::context::PackageProjectionContextInput {
            package_id: publication.manifest.id.as_str(),
            version: publication.manifest.version.as_str(),
            dependencies: projected_package_dependencies_for_test(
                &publication.manifest.dependencies,
            ),
            api_entries: package_api_entries_for_test(&publication.manifest),
            api_source: publication.manifest.api.source.as_ref().map(|source| {
                skiff_compiler_projection::context::PackageApiSourceProjection {
                    relative_path: source.relative_path.clone(),
                    content_hash: source.content_hash.clone(),
                }
            }),
            package_root: &publication.source_tree.root,
            prelude: &prelude_projection,
        },
    );
    let projection_input = build_projection_input(compiled);
    let projection_view = projection_input.view();
    let package_projection =
        skiff_compiler_projection::project_package(projection_view, projection_context)?;
    let unit_artifacts = skiff_compiler_projection::package_unit_artifacts::project_package_ir_artifacts(
        skiff_compiler_projection::package_unit_artifacts::PackageIrProjectionSource {
            package_id: publication.manifest.id.as_str(),
            version: publication.manifest.version.as_str(),
            exports: &package_projection.exports,
            abi_identity_projection: &package_projection.abi_identity_projection,
            config_projection: &package_projection.config_projection,
            file_ir_units: package_projection
                .input
                .file_ir_units()
                .iter()
                .cloned()
                .map(skiff_compiler_projection::package_unit_artifacts::PackageFileIrProjection::from_unit)
                .collect(),
        },
        &projected_package_dependencies_for_test(&publication.manifest.dependencies),
    )?;
    Ok(unit_artifacts.unit)
}

pub fn compile_package_dependency_publications_for_test(
    current: &TestPackageManifest,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<TestPackageDependencyPublications, PublicationError> {
    Ok(TestPackageDependencyPublications {
        package_publications: package_publications_for_test(
            current,
            dependency_packages,
            available,
        )?,
    })
}

fn package_publications_for_test(
    current: &TestPackageManifest,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<Vec<PackagePublication>, PublicationError> {
    let available = internal_manifest_map(available);
    let packages = dependency_packages
        .iter()
        .filter(|package| package.manifest.id != current.id)
        .map(|package| ResolvedPackage {
            manifest: package.manifest.clone().into_internal(),
            config: package.config.clone(),
        })
        .collect::<Vec<_>>();
    let package_jobs = crate::input::package_job::build_package_jobs(packages)?;
    crate::pipeline::compile_package_jobs(package_jobs, &available)
}

pub fn package_unit_artifacts_for_test(
    packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<Vec<TestPackageUnitArtifact>, PublicationError> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    let available = internal_manifest_map(available);
    let packages = packages
        .iter()
        .map(|package| ResolvedPackage {
            manifest: package.manifest.clone().into_internal(),
            config: package.config.clone(),
        })
        .collect::<Vec<_>>();
    let package_jobs = crate::input::package_job::build_package_jobs(packages)?;
    let publications = crate::pipeline::compile_package_jobs(package_jobs, &available)?;
    package_unit_artifacts_from_package_publications(&publications)
}

pub fn package_unit_artifacts_from_dependency_publications_for_test(
    dependency_publications: &TestPackageDependencyPublications,
) -> Result<Vec<TestPackageUnitArtifact>, PublicationError> {
    package_unit_artifacts_from_package_publications(dependency_publications.as_slice())
}

fn package_unit_artifacts_from_package_publications(
    publications: &[PackagePublication],
) -> Result<Vec<TestPackageUnitArtifact>, PublicationError> {
    if publications.is_empty() {
        return Ok(Vec::new());
    }
    let projection_inputs = build_package_projection_inputs(publications);
    let prelude_projection = crate::shared::prelude_registry::projection_prelude_context();
    let package_projections = skiff_compiler_projection::project_package_publications(
        &projection_inputs,
        &prelude_projection,
    )?;
    let package_artifacts =
        crate::emission::package_artifacts::build_package_artifacts(&package_projections)?;
    let projections_by_id = package_projections
        .iter()
        .map(|projection| (projection.manifest().id().to_string(), projection))
        .collect::<BTreeMap<_, _>>();
    package_artifacts
        .into_iter()
        .map(|artifact| {
            let projection = projections_by_id
                .get(&artifact.package_id)
                .expect("package artifact must have matching package projection");
            let unit_artifacts =
                crate::emission::package_unit_artifacts::publish_package_ir_artifacts(
                    &artifact,
                    &projection.package_ir,
                )?;
            Ok(TestPackageUnitArtifact {
                package_id: artifact.package_id,
                package_version: artifact.version,
                package_dependencies: unit_artifacts.unit.dependencies.clone(),
                production_files: unit_artifacts.file_ir_units,
                unit: unit_artifacts.unit,
            })
        })
        .collect()
}

pub fn discover_package_manifests(
    root: &Path,
) -> Result<BTreeMap<PackageManifestKey, TestPackageManifest>, PackageConfigError> {
    crate::input::discover_package_manifests(root).map(|manifests| {
        manifests
            .into_iter()
            .map(|(key, manifest)| (key, test_manifest_from_internal(manifest)))
            .collect()
    })
}

pub fn discover_package_manifests_with_dirs(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
) -> Result<BTreeMap<PackageManifestKey, TestPackageManifest>, PackageConfigError> {
    crate::input::discover_package_manifests_with_dirs(root, package_dirs).map(|manifests| {
        manifests
            .into_iter()
            .map(|(key, manifest)| (key, test_manifest_from_internal(manifest)))
            .collect()
    })
}

pub fn discover_package_manifests_with_dependency_dirs(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
    dependencies: &[PackageDependency],
) -> Result<BTreeMap<PackageManifestKey, TestPackageManifest>, PackageConfigError> {
    crate::input::discover_package_manifests_with_dependency_dirs(root, package_dirs, dependencies)
        .map(|manifests| {
            manifests
                .into_iter()
                .map(|(key, manifest)| (key, test_manifest_from_internal(manifest)))
                .collect()
        })
}

pub fn read_user_package_manifest(path: &Path) -> Result<TestPackageManifest, PackageConfigError> {
    crate::input::read_user_package_manifest(path).map(test_manifest_from_internal)
}

pub fn resolve_package_imports(
    imports: &[Vec<String>],
    dependencies: &[PackageDependency],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<Vec<TestResolvedPackage>, PackageConfigError> {
    let available = internal_manifest_map(available);
    crate::input::resolve_package_imports(imports, dependencies, &available).map(|resolved| {
        resolved
            .into_iter()
            .map(|package| TestResolvedPackage {
                manifest: test_manifest_from_internal(package.manifest),
                config: package.config,
            })
            .collect()
    })
}

pub fn package_alias_bindings(
    dependencies: &[PackageDependency],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> BTreeMap<String, Vec<String>> {
    let available = internal_manifest_map(available);
    crate::input::package_alias_bindings(dependencies, &available)
}

pub fn package_unit_dependency_constraints_for_test(
    dependencies: &[PackageDependency],
    file_ir_units: &[PublishedFileIrArtifact],
    package_id: &str,
) -> Vec<PackageDependencyConstraint> {
    let file_ir_units = file_ir_units
        .iter()
        .map(|artifact| {
            skiff_compiler_projection::package_unit_artifacts::PackageFileIrProjection {
                unit: artifact.unit.clone(),
                identity: artifact.identity.clone(),
                source_ast_hash: artifact.unit.source_ast_hash.clone(),
                module_path: artifact.module_path.clone(),
            }
        })
        .collect::<Vec<_>>();
    skiff_compiler_projection::package_unit_artifacts::package_unit_dependency_constraints(
        &projected_package_dependencies_for_test(dependencies),
        &file_ir_units,
        package_id,
    )
}

pub(crate) fn package_publication_for_test(
    publication: Publication,
    compiled: CompiledPublication,
    dependency_config: serde_json::Value,
) -> PackagePublication {
    let info = crate::pipeline::package_publication_info_from_publication(&publication);
    PackagePublication::new(info, compiled, dependency_config)
}

pub(crate) fn projected_package_dependencies_for_test(
    dependencies: &[PackageDependency],
) -> Vec<skiff_compiler_projection::context::ProjectedPackageDependency> {
    dependencies
        .iter()
        .map(
            |dependency| skiff_compiler_projection::context::ProjectedPackageDependency {
                id: dependency.id.clone(),
                version: dependency.version.clone(),
                alias: dependency.alias.clone(),
                config: dependency.config.clone(),
                collection_name_mapping: dependency.collection_name_mapping.clone(),
            },
        )
        .collect()
}

pub(crate) fn package_api_entries_for_test(
    manifest: &PublicationManifest,
) -> Vec<skiff_compiler_projection::context::PackageApiEntryProjection> {
    manifest
        .api
        .entries()
        .map(
            |entry| skiff_compiler_projection::context::PackageApiEntryProjection {
                path: entry.public_path_string(),
                module: entry.source_module_hint().to_string(),
            },
        )
        .chain(manifest.api.public_instances().map(|entry| {
            skiff_compiler_projection::context::PackageApiEntryProjection {
                path: entry.public_path_string(),
                module: entry.source_module_hint().to_string(),
            }
        }))
        .collect()
}

pub fn is_complex_package_id(package_id: &str) -> bool {
    crate::input::is_complex_package_id(package_id)
}

pub fn package_public_path(package_id: &str, export_path: &str) -> String {
    skiff_compiler_input::export_config::package_public_path(package_id, export_path)
}

pub fn default_std_dir() -> PathBuf {
    skiff_compiler_source::prelude_registry::default_std_dir()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    skiff_compiler_core::json_utils::sha256_hex(bytes)
}

pub fn expr_path(expr: &Expr) -> Option<String> {
    skiff_syntax::ast_utils::expr_path(expr)
}

pub fn type_name_candidates(type_name: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    skiff_syntax::type_expr::TypeExpr::parse_lossy(type_name).for_each_named(|name| {
        if !name.is_empty() {
            candidates.push(name.to_string());
        }
    });
    candidates
}

pub fn generic_inner<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    skiff_syntax::type_syntax::generic_inner(input, name)
}

pub fn is_test_file_path(path: &Path) -> bool {
    crate::input::test_rules::is_test_file_path(path)
}

pub fn module_relative_path_for_test_file_without_friend(path: &Path) -> PathBuf {
    crate::input::test_rules::module_relative_path_for_test_file_without_friend(path)
}

pub fn is_friend_test_file_for_production(test_path: &Path, production_path: &Path) -> bool {
    crate::input::test_rules::is_friend_test_file_for_production(test_path, production_path)
}

pub fn production_friend_match_for_test_file(
    path: &Path,
) -> Result<FriendProductionMatch, std::io::Error> {
    crate::input::test_rules::production_friend_match_for_test_file(path)
}

pub fn validate_no_test_declarations_in_production_source(
    path: &str,
    ast: &AstSourceFile,
) -> Result<(), CompileError> {
    crate::input::test_rules::validate_no_test_declarations_in_production_source(path, ast)
}

pub fn synthetic_source_span() -> skiff_syntax::error::SourceSpan {
    skiff_syntax::error::SourceSpan::synthetic()
}

fn internal_manifest_map(
    manifests: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> BTreeMap<PackageManifestKey, crate::input::PackageManifest> {
    manifests
        .iter()
        .map(|(key, manifest)| (key.clone(), manifest.clone().into_internal()))
        .collect()
}

fn test_manifest_from_internal(manifest: crate::input::PackageManifest) -> TestPackageManifest {
    let publication = manifest.into_publication();
    TestPackageManifest {
        id: publication.id.to_string(),
        version: publication.version,
        api: publication
            .api
            .entries()
            .map(test_api_entry_from_internal)
            .collect(),
        dependencies: publication.dependencies,
        path: publication.provenance.path,
        synthetic: publication.provenance.synthetic,
    }
}

impl TestPackageManifest {
    fn into_internal(self) -> crate::input::PackageManifest {
        let api = crate::input::PackageApi::from_entries(
            self.api
                .into_iter()
                .map(test_api_entry_into_internal)
                .collect(),
        );
        let publication = PublicationManifest::new(
            skiff_compiler_core::id::PublicationId::parse(&self.id).unwrap(),
            self.version,
            api,
            self.dependencies,
            if self.synthetic {
                ManifestProvenance::synthetic(self.path, ManifestOwner::UserOrBuiltinPackage)
            } else {
                ManifestProvenance::file(self.path, ManifestOwner::UserOrBuiltinPackage)
            },
        );
        crate::input::PackageManifest::new(publication)
    }
}

fn test_api_entry_from_internal(entry: &crate::input::PackageApiEntry) -> TestPackageApiEntry {
    TestPackageApiEntry::source(
        entry.public_path_string(),
        entry.source_module_hint(),
        entry.source_symbol(),
    )
}

fn test_api_entry_into_internal(entry: TestPackageApiEntry) -> PublicationApiEntry {
    PublicationApiEntry::for_source(entry.path, entry.module, entry.symbol)
}
