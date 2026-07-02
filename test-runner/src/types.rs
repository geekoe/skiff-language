use std::{
    collections::{BTreeMap, BTreeSet},
    env,
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
    pub package_test_concurrency: Option<usize>,
}

impl SkiffTestOptions {
    pub(crate) fn package_resolution_dirs_for(&self, _root: &Path) -> PackageResolutionDirs {
        PackageResolutionDirs {
            package_dirs: self.package_dirs.clone(),
        }
    }
}

pub(crate) fn default_skiff_dev_home() -> Option<PathBuf> {
    if let Some(value) = env::var_os("SKIFF_DEV_HOME") {
        let path = PathBuf::from(&value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".skiff").join("dev"))
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
    #[error("ambiguous friend test {path}: matches multiple production files: {candidates}")]
    AmbiguousFriendTest { path: String, candidates: String },
}

#[derive(Debug, Clone)]
pub(super) struct ParsedSource {
    pub(super) source: SourceTreeFile,
    pub(super) text: String,
    pub(super) ast: AstSourceFile,
    pub(super) synthetic_imports: BTreeSet<String>,
    pub(super) friend_module_path: Option<String>,
    pub(super) private_visibility_scope: PrivateVisibilityScope,
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
    pub(super) production_exports: BTreeMap<String, ProductionModuleSymbols>,
    pub(super) package_ids: BTreeSet<String>,
    pub(super) package_aliases: BTreeMap<String, Vec<String>>,
    pub(super) disallowed_import_modules: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(super) struct PackageTestSource {
    pub(super) relative_path: PathBuf,
    pub(super) module_path: String,
    pub(super) is_test_file: bool,
    pub(super) text: String,
    pub(super) ast: AstSourceFile,
    pub(super) synthetic_imports: BTreeSet<String>,
    pub(super) friend_module_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum PrivateVisibilityScope {
    #[default]
    PublicOnly,
    Module(String),
    AllModules,
    Modules(BTreeSet<String>),
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
    pub(super) package_ids: BTreeSet<String>,
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
            package_ids: BTreeSet::new(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SymbolUseKind {
    Value,
    Type,
}

#[derive(Debug, Clone, Default)]
pub(super) struct TestLocalSymbols {
    pub(super) values: BTreeSet<String>,
    pub(super) types: BTreeSet<String>,
    pub(super) imports: BTreeSet<String>,
    pub(super) synthetic_imports: BTreeSet<String>,
    pub(super) package_ids: BTreeSet<String>,
    pub(super) package_aliases: BTreeMap<String, Vec<String>>,
    pub(super) disallowed_import_modules: BTreeSet<String>,
}

impl TestLocalSymbols {
    pub(super) fn contains(&self, name: &str, use_kind: SymbolUseKind) -> bool {
        match use_kind {
            SymbolUseKind::Value => self.values.contains(name),
            SymbolUseKind::Type => self.types.contains(name),
        }
    }

    pub(super) fn imports_module(&self, module_path: &str) -> bool {
        self.imports.iter().any(|import| {
            let alias_match = self.package_aliases.get(import).is_some_and(|roots| {
                roots
                    .iter()
                    .any(|root| module_path == root || module_path.starts_with(&format!("{root}.")))
            });
            (module_path == import && !self.disallowed_import_modules.contains(module_path))
                || alias_match
                || (self.package_ids.contains(import)
                    && module_path.starts_with(&format!("{import}.")))
                || (import == "std" && module_path.starts_with("std."))
        })
    }

    pub(super) fn imports_path(&self, path: &str) -> bool {
        self.imports.contains(path) || self.synthetic_imports.contains(path)
    }
}

impl ProductionSymbolKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::DbObject => "db object",
            Self::Interface => "interface",
            Self::Function => "function",
            Self::Const => "const",
            Self::Method => "method",
        }
    }

    pub(super) fn matches_use(self, use_kind: SymbolUseKind) -> bool {
        matches!(
            (self, use_kind),
            (
                Self::Type | Self::DbObject | Self::Interface,
                SymbolUseKind::Type
            ) | (
                Self::Function | Self::Const | Self::Method,
                SymbolUseKind::Value
            )
        )
    }
}
