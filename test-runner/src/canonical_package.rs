use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use skiff_artifact_model::PackageArtifact;
use skiff_compiler::{
    compile_package, PackageCompileError, PackageCompileInput, PackageContractCompileDependency,
    PackageSourceInput, PublishedPackageArtifact,
};
use skiff_compiler_input::{
    package_config::{
        discover_package_manifests_with_dependency_dirs, package_alias_bindings,
        read_user_package_manifest, PackageConfigError, PackageManifest, PackageManifestKey,
        PackageResolutionDirs, PACKAGE_CONFIG_FILE,
    },
    package_sources::{read_official_package_sources, read_package_sources},
    read_publication_resources, InputAssemblyError, ManifestOwner,
};
use skiff_compiler_source::{source_graph::PublicationSourceGraph, SourceCompileError};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct CanonicalPackageProject {
    pub package: PublishedPackageArtifact,
    pub dependency_packages: Vec<PublishedPackageArtifact>,
}

impl CanonicalPackageProject {
    pub fn packages(&self) -> impl Iterator<Item = &PublishedPackageArtifact> {
        std::iter::once(&self.package).chain(&self.dependency_packages)
    }

    pub fn artifact(
        &self,
        package_id: &str,
        package_version: &str,
    ) -> Option<&PublishedPackageArtifact> {
        self.packages().find(|package| {
            package.artifact.package_id == package_id
                && package.artifact.package_version == package_version
        })
    }
}

#[derive(Debug, Error)]
pub enum CanonicalPackageProjectError {
    #[error(transparent)]
    PackageConfig(#[from] PackageConfigError),
    #[error(transparent)]
    Input(#[from] InputAssemblyError),
    #[error(transparent)]
    Source(#[from] SourceCompileError),
    #[error(transparent)]
    Compile(#[from] PackageCompileError),
    #[error("package dependency {package_id}@{package_version} has no manifest")]
    MissingDependencyManifest {
        package_id: String,
        package_version: String,
    },
    #[error("compiled requirement {package_id}@{package_version} has no canonical artifact")]
    MissingDependencyArtifact {
        package_id: String,
        package_version: String,
    },
    #[error("package dependency graph contains a cycle: {coordinates}")]
    DependencyCycle { coordinates: String },
    #[error("package manifest path {path} has no package root")]
    MissingPackageRoot { path: String },
}

/// Compile one package source input through the production canonical pipeline.
pub fn compile_package_artifact(
    package: &PackageSourceInput,
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_packages: &[PackageArtifact],
    available_packages: &[PackageArtifact],
    contract_dependencies: &[PackageContractCompileDependency],
) -> Result<PublishedPackageArtifact, PackageCompileError> {
    let package_id = package.manifest().id.to_string();
    let input = PackageCompileInput::new(package, package_aliases, &package_id)
        .with_canonical_dependencies(dependency_packages, contract_dependencies)
        .with_available_canonical_packages(available_packages);
    compile_package(input)
}

/// Compile the exact package graph selected by `package.yml`.
///
/// Every dependency is a previously validated `PackageArtifact`; no legacy
/// publication aggregate is constructed or consulted.
pub fn compile_package_project(
    root: &Path,
    package_dirs: &[PathBuf],
    contract_dependencies: &BTreeMap<PackageManifestKey, Vec<PackageContractCompileDependency>>,
) -> Result<CanonicalPackageProject, CanonicalPackageProjectError> {
    let root_manifest = read_user_package_manifest(&root.join(PACKAGE_CONFIG_FILE))?;
    let root_key = (root_manifest.id.to_string(), root_manifest.version.clone());
    let mut resolution_dirs = package_dirs.to_vec();
    let local_store = root.join(".skiff-packages");
    if local_store.is_dir() && !resolution_dirs.contains(&local_store) {
        resolution_dirs.push(local_store);
    }
    let manifests = discover_package_manifests_with_dependency_dirs(
        root,
        &PackageResolutionDirs {
            package_dirs: resolution_dirs,
        },
        &root_manifest.dependencies,
    )?;
    let mut graph = PackageGraphCompiler::new(manifests, contract_dependencies);
    graph.compile_platform_std()?;
    let package = graph.compile(&root_key)?;
    let dependency_packages = graph.compiled_dependency_closure(&package)?;
    Ok(CanonicalPackageProject {
        package,
        dependency_packages,
    })
}

pub fn find_package_root(input: &Path, input_is_file: bool) -> Option<PathBuf> {
    let mut current = if input_is_file {
        input.parent()?.to_path_buf()
    } else {
        input.to_path_buf()
    };
    loop {
        if current.join(PACKAGE_CONFIG_FILE).is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

struct PackageGraphCompiler<'a> {
    manifests: BTreeMap<PackageManifestKey, PackageManifest>,
    contract_dependencies: &'a BTreeMap<PackageManifestKey, Vec<PackageContractCompileDependency>>,
    published: BTreeMap<PackageManifestKey, PublishedPackageArtifact>,
    visiting: Vec<PackageManifestKey>,
    visiting_set: BTreeSet<PackageManifestKey>,
}

impl<'a> PackageGraphCompiler<'a> {
    fn new(
        manifests: BTreeMap<PackageManifestKey, PackageManifest>,
        contract_dependencies: &'a BTreeMap<
            PackageManifestKey,
            Vec<PackageContractCompileDependency>,
        >,
    ) -> Self {
        Self {
            manifests,
            contract_dependencies,
            published: BTreeMap::new(),
            visiting: Vec::new(),
            visiting_set: BTreeSet::new(),
        }
    }

    fn compile_platform_std(&mut self) -> Result<(), CanonicalPackageProjectError> {
        let key = self
            .manifests
            .keys()
            .find(|(package_id, _)| package_id == "skiff.run/std")
            .cloned();
        if let Some(key) = key {
            self.compile(&key)?;
        }
        Ok(())
    }

    fn compile(
        &mut self,
        key: &PackageManifestKey,
    ) -> Result<PublishedPackageArtifact, CanonicalPackageProjectError> {
        if let Some(package) = self.published.get(key) {
            return Ok(package.clone());
        }
        if self.visiting_set.contains(key) {
            let first = self
                .visiting
                .iter()
                .position(|candidate| candidate == key)
                .unwrap_or(0);
            let coordinates = self.visiting[first..]
                .iter()
                .chain(std::iter::once(key))
                .map(|(id, version)| format!("{id}@{version}"))
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(CanonicalPackageProjectError::DependencyCycle { coordinates });
        }
        let manifest = self.manifests.get(key).cloned().ok_or_else(|| {
            CanonicalPackageProjectError::MissingDependencyManifest {
                package_id: key.0.clone(),
                package_version: key.1.clone(),
            }
        })?;
        self.visiting.push(key.clone());
        self.visiting_set.insert(key.clone());
        let result = self.compile_manifest(&manifest);
        self.visiting.pop();
        self.visiting_set.remove(key);
        let package = result?;
        self.published.insert(key.clone(), package.clone());
        Ok(package)
    }

    fn compile_manifest(
        &mut self,
        manifest: &PackageManifest,
    ) -> Result<PublishedPackageArtifact, CanonicalPackageProjectError> {
        let mut dependencies = Vec::with_capacity(manifest.dependencies.len());
        for dependency in &manifest.dependencies {
            dependencies.push(
                self.compile(&(dependency.id.clone(), dependency.version.clone()))?
                    .artifact,
            );
        }
        let source = read_package_source_input(manifest)?;
        let package_id = manifest.id.to_string();
        let aliases = package_alias_bindings(&manifest.dependencies, &self.manifests);
        let available = self
            .published
            .values()
            .map(|package| package.artifact.clone())
            .collect::<Vec<_>>();
        let contracts = self
            .contract_dependencies
            .get(&(package_id, manifest.version.clone()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        Ok(compile_package_artifact(
            &source,
            &aliases,
            &dependencies,
            &available,
            contracts,
        )?)
    }

    fn compiled_dependency_closure(
        &self,
        root: &PublishedPackageArtifact,
    ) -> Result<Vec<PublishedPackageArtifact>, CanonicalPackageProjectError> {
        let mut closure = BTreeSet::new();
        let mut pending = requirement_keys(root);
        while let Some(key) = pending.pop() {
            if !closure.insert(key.clone()) {
                continue;
            }
            let dependency = self.published.get(&key).ok_or_else(|| {
                CanonicalPackageProjectError::MissingDependencyArtifact {
                    package_id: key.0.clone(),
                    package_version: key.1.clone(),
                }
            })?;
            pending.extend(requirement_keys(dependency));
        }
        Ok(self
            .published
            .iter()
            .filter_map(|(key, package)| closure.contains(key).then_some(package.clone()))
            .collect())
    }
}

fn read_package_source_input(
    manifest: &PackageManifest,
) -> Result<PackageSourceInput, CanonicalPackageProjectError> {
    let root = manifest
        .provenance
        .path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| CanonicalPackageProjectError::MissingPackageRoot {
            path: manifest.provenance.path.display().to_string(),
        })?;
    let sources = match manifest.provenance.owner {
        ManifestOwner::CompilerStandardPackage => read_official_package_sources(manifest, &root)?,
        ManifestOwner::UserOrBuiltinPackage => read_package_sources(manifest, &root)?,
    };
    let source_tree = sources.source_tree();
    let source_graph =
        PublicationSourceGraph::parse_raw_publication_sources(&sources.into_source_graph())?;
    let resources = read_publication_resources(&root, &manifest.resources)?;
    Ok(PackageSourceInput::new(
        manifest.publication.clone(),
        source_tree,
        source_graph,
        resources,
    ))
}

fn requirement_keys(package: &PublishedPackageArtifact) -> Vec<PackageManifestKey> {
    package
        .artifact
        .package_requirements
        .iter()
        .map(|requirement| {
            (
                requirement.package_id.clone(),
                requirement.exact_version.clone(),
            )
        })
        .collect()
}
