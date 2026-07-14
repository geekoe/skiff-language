use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use thiserror::Error;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use skiff_artifact_model::FileIrUnit;
use skiff_compiler::{
    test_support::{TestPackageDependencyPublications, TestPackageTestDependencyPackageInput},
    PackageConfigError, PackageResolutionDirs, PublicationError, SourceTreeFile,
};
use skiff_syntax::ast::SourceFile as AstSourceFile;
use skiff_syntax::error::CompileError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkiffTestSummary {
    pub passed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub results: Vec<SkiffTestResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkiffTestResult {
    pub module_path: String,
    pub name: String,
    pub passed: bool,
    pub skipped: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestEffectDouble {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expect_request: Option<JsonValue>,
    pub(crate) response: JsonValue,
}

#[derive(Debug, Clone, Default)]
pub struct SkiffTestOptions {
    pub live: bool,
    pub allow_network: bool,
    pub config_path: Option<PathBuf>,
    pub package_dirs: Vec<PathBuf>,
    pub service_artifact_roots: Vec<PathBuf>,
    pub router_reload_url: Option<String>,
    pub artifact_root: Option<PathBuf>,
    pub package_test_concurrency: Option<usize>,
}

impl SkiffTestOptions {
    pub(crate) fn package_resolution_dirs_for(&self, _root: &Path) -> PackageResolutionDirs {
        PackageResolutionDirs {
            package_dirs: self.package_dirs.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum SkiffTestError {
    #[error("failed to inspect input {path}: {source}")]
    Metadata {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("service config error: {0}")]
    ServiceConfig(#[from] skiff_compiler::ServiceConfigError),
    #[error("service project error: {0}")]
    ServiceProject(#[from] PublicationError),
    #[error("source tree error: {0}")]
    SourceTree(#[from] skiff_compiler::SourceTreeError),
    #[error("package config error: {0}")]
    PackageConfig(#[from] PackageConfigError),
    #[error("failed to read {path}: {source}")]
    ReadSource {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read test doubles {path}: {source}")]
    ReadTestDoubles {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse test doubles {path}: {source}")]
    ParseTestDoubles {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid test double in {path}: {message}")]
    InvalidTestDouble { path: String, message: String },
    #[error("parse failed in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: CompileError,
    },
    #[error("compile failed for test {name}: {source}")]
    Compile {
        name: String,
        #[source]
        source: CompileError,
    },
    #[error("runtime test setup failed: {message}")]
    RuntimeSetup { message: String },
    #[error("invalid root reference in {path}: {message}")]
    RootPathReference { path: String, message: String },
    #[error("input {path} is neither a file nor a directory")]
    InvalidInput { path: String },
}

#[derive(Debug, Clone)]
pub(super) struct ParsedSource {
    pub(super) source: SourceTreeFile,
    pub(super) text: String,
    pub(super) ast: AstSourceFile,
}

#[derive(Debug, Clone)]
pub(super) struct TestCase {
    pub(super) module_path: String,
    pub(super) name: String,
    pub(super) test_index: usize,
    pub(super) source: ParsedSource,
    pub(super) function_name: String,
}

/// Fully resolved inputs for running service tests through the synthetic service
/// publication path. Package tests now use native package-test artifacts and do
/// not flow through `run_resolved_publication_tests`.
pub(super) struct ResolvedPublicationTestInputs {
    pub(super) service_config: skiff_compiler::ServiceConfig,
    /// Scope component used to mint a *fresh* synthetic service id for every
    /// individual test (see `synthetic_test_service_id`). Each test must run as
    /// its own service id so the runtime projects it to its own Mongo database
    /// namespace; otherwise a global `db find` in one test would observe rows
    /// written by sibling tests sharing the same database. The concrete per-test
    /// id is generated inside `run_resolved_publication_tests`, never reused.
    pub(super) service_id_scope: String,
    /// All production sources for the publication (root-resolved). For packages
    /// this is the whole package flattened as service root sources.
    pub(super) production_sources: Vec<ParsedSource>,
    /// Test sources whose tests should be collected and run (root-resolved).
    pub(super) test_sources: Vec<ParsedSource>,
    pub(super) test_doubles: crate::doubles::RuntimeTestDoubles,
    pub(super) package_aliases: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub(super) struct PackageTestSource {
    pub(super) relative_path: PathBuf,
    pub(super) module_path: String,
    pub(super) is_test_file: bool,
    pub(super) text: String,
    pub(super) ast: AstSourceFile,
}

#[derive(Debug, Clone)]
pub(super) struct PackageTestCase {
    pub(super) module_path: String,
    pub(super) name: String,
    pub(super) test_index: usize,
    pub(super) source: PackageTestSource,
    pub(super) function_name: String,
}

pub(super) struct PackageDependencyArtifacts {
    pub(super) package_test_dependency_packages: Vec<TestPackageTestDependencyPackageInput>,
    pub(super) dependency_publications: TestPackageDependencyPublications,
    pub(super) production_exports: BTreeMap<String, ProductionModuleSymbols>,
    pub(super) function_return_types: BTreeMap<String, String>,
    pub(super) package_aliases: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RuntimeTestArtifact {
    pub(super) source_path: String,
    pub(super) module_path: String,
    pub(super) role: String,
    pub(super) package_id: Option<String>,
    pub(super) file_ir: FileIrUnit,
}

impl Default for PackageDependencyArtifacts {
    fn default() -> Self {
        Self {
            package_test_dependency_packages: Vec::new(),
            dependency_publications: TestPackageDependencyPublications::default(),
            production_exports: BTreeMap::new(),
            function_return_types: BTreeMap::new(),
            package_aliases: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ProductionModuleSymbols {
    pub(super) symbols: BTreeMap<String, ProductionSymbol>,
    pub(super) db_objects: BTreeSet<String>,
    pub(super) member_symbols: BTreeMap<String, ProductionSymbol>,
}

#[derive(Debug, Clone)]
pub(super) struct ProductionSymbol {
    pub(super) kind: ProductionSymbolKind,
    pub(super) exported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProductionSymbolKind {
    Type,
    DbObject,
    Interface,
    Function,
    Const,
    Method,
}
