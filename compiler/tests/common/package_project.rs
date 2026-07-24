#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skiff_compiler::{
    PackageCompileError, PackageContractCompileDependency, PublishedPackageArtifact,
    ResolvedPackageSchema, ResolvedPackageSchemaError,
};
use skiff_compiler_input::{
    package_config::{
        discover_package_manifests_with_dependency_dirs, read_user_package_manifest,
        PackageConfigError, PackageManifestKey, PackageResolutionDirs, PACKAGE_CONFIG_FILE,
    },
    CompilerPlatformSources, CompilerPlatformSourcesError, InputAssemblyError,
};
use skiff_compiler_source::SourceCompileError;
use thiserror::Error;

use super::package_graph::PackageGraphCompiler;

const LOCAL_PACKAGE_STORE: &str = ".skiff-packages";

#[derive(Debug, Clone)]
pub struct PublishedPackageProject {
    pub package: PublishedPackageArtifact,
    pub dependency_packages: Vec<PublishedPackageArtifact>,
}

impl PublishedPackageProject {
    pub fn artifact(
        &self,
        package_id: &str,
        package_version: &str,
    ) -> Option<&PublishedPackageArtifact> {
        std::iter::once(&self.package)
            .chain(&self.dependency_packages)
            .find(|package| {
                package.artifact.package_id == package_id
                    && package.artifact.package_version == package_version
            })
    }

    pub fn dependency(
        &self,
        package_id: &str,
        package_version: &str,
    ) -> Option<&PublishedPackageArtifact> {
        self.dependency_packages.iter().find(|package| {
            package.artifact.package_id == package_id
                && package.artifact.package_version == package_version
        })
    }

    pub fn artifacts(&self) -> impl Iterator<Item = &PublishedPackageArtifact> {
        std::iter::once(&self.package).chain(&self.dependency_packages)
    }
}

#[derive(Debug, Error)]
pub enum PackageProjectCompileError {
    #[error(transparent)]
    PlatformSources(#[from] CompilerPlatformSourcesError),
    #[error(transparent)]
    PackageConfig(#[from] PackageConfigError),
    #[error(transparent)]
    Input(#[from] InputAssemblyError),
    #[error(transparent)]
    Source(#[from] SourceCompileError),
    #[error(transparent)]
    Compile(#[from] PackageCompileError),
    #[error(transparent)]
    ResolvedPackageSchema(#[from] ResolvedPackageSchemaError),
    #[error("package dependency {package_id}@{package_version} has no discovered package.yml")]
    MissingDependencyManifest {
        package_id: String,
        package_version: String,
    },
    #[error(
        "compiled package requirement {package_id}@{package_version} has no canonical artifact"
    )]
    MissingDependencyArtifact {
        package_id: String,
        package_version: String,
    },
    #[error("package dependency graph contains a cycle: {coordinates}")]
    DependencyCycle { coordinates: String },
    #[error("package manifest path {path} has no package root")]
    MissingPackageRoot { path: String },
}

/// Compiles a package project rooted at `package.yml`, using a local
/// `.skiff-packages` store when present.
pub fn compile_package_project(
    root: &Path,
) -> Result<PublishedPackageProject, PackageProjectCompileError> {
    compile_package_project_with_contract_dependencies(root, &BTreeMap::new())
}

/// Compiles a package graph with validated contract dependencies attached to
/// the package coordinate that declares them.
pub fn compile_package_project_with_contract_dependencies(
    root: &Path,
    contract_dependencies: &BTreeMap<PackageManifestKey, Vec<PackageContractCompileDependency>>,
) -> Result<PublishedPackageProject, PackageProjectCompileError> {
    let store = root.join(LOCAL_PACKAGE_STORE);
    let package_dirs = PackageResolutionDirs {
        package_dirs: store.is_dir().then_some(store).into_iter().collect(),
    };
    compile_package_project_with_dirs_contract_dependencies_and_schemas(
        root,
        &package_dirs,
        contract_dependencies,
        &BTreeMap::new(),
    )
}

pub fn compile_package_project_with_contract_dependencies_and_schemas(
    root: &Path,
    contract_dependencies: &BTreeMap<PackageManifestKey, Vec<PackageContractCompileDependency>>,
    resolved_package_schemas: &BTreeMap<PackageManifestKey, Vec<ResolvedPackageSchema>>,
) -> Result<PublishedPackageProject, PackageProjectCompileError> {
    let store = root.join(LOCAL_PACKAGE_STORE);
    let package_dirs = PackageResolutionDirs {
        package_dirs: store.is_dir().then_some(store).into_iter().collect(),
    };
    compile_package_project_with_dirs_contract_dependencies_and_schemas(
        root,
        &package_dirs,
        contract_dependencies,
        resolved_package_schemas,
    )
}

/// Compiles the exact package dependency graph selected by `package.yml` and
/// explicit package stores. Every node goes through the public
/// `compile_package` entrypoint; no test-only compiler pipeline is used.
pub fn compile_package_project_with_dirs(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
) -> Result<PublishedPackageProject, PackageProjectCompileError> {
    compile_package_project_with_dirs_contract_dependencies_and_schemas(
        root,
        package_dirs,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

fn compile_package_project_with_dirs_contract_dependencies_and_schemas(
    root: &Path,
    package_dirs: &PackageResolutionDirs,
    contract_dependencies: &BTreeMap<PackageManifestKey, Vec<PackageContractCompileDependency>>,
    resolved_package_schemas: &BTreeMap<PackageManifestKey, Vec<ResolvedPackageSchema>>,
) -> Result<PublishedPackageProject, PackageProjectCompileError> {
    let platform_sources = repository_platform_sources()?;
    let root_manifest = read_user_package_manifest(&root.join(PACKAGE_CONFIG_FILE))?;
    let root_key = (root_manifest.id.to_string(), root_manifest.version.clone());
    let manifests = discover_package_manifests_with_dependency_dirs(
        &platform_sources,
        root,
        package_dirs,
        &root_manifest.dependencies,
    )?;
    let mut graph = PackageGraphCompiler::new(
        &platform_sources,
        manifests,
        contract_dependencies,
        resolved_package_schemas,
    );
    graph.compile_platform_std()?;
    let package = graph.compile(&root_key)?;
    let dependency_packages = graph.compiled_dependency_closure(&package)?;
    Ok(PublishedPackageProject {
        package,
        dependency_packages,
    })
}

fn repository_platform_sources() -> Result<CompilerPlatformSources, CompilerPlatformSourcesError> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler crate manifest directory should have a repository parent")
        .to_path_buf();
    CompilerPlatformSources::new(&root)
}
