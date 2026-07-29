#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use skiff_artifact_model::PackageArtifact;
use skiff_compiler::{
    authoring::publish_package_artifact_records, compile_package, compile_service_package,
    CompiledServicePackage, PackageCompileInput, PackageContractCompileDependency,
    PackageSourceInput, PublishedPackageArtifact,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_input::{
    package_config::{package_alias_bindings, PackageManifest, PackageManifestKey},
    package_sources::{read_official_package_sources, read_package_sources},
    read_publication_resources, CompilerPlatformSources, ManifestOwner, ServicePackageRoot,
};
use skiff_compiler_source::source_graph::PublicationSourceGraph;
use skiff_deployment::storage::CanonicalArtifactStore;

use super::package_project::PackageProjectCompileError;

pub(super) struct PackageGraphCompiler<'a> {
    platform_sources: &'a CompilerPlatformSources,
    manifests: BTreeMap<PackageManifestKey, PackageManifest>,
    contract_dependencies: &'a BTreeMap<PackageManifestKey, Vec<PackageContractCompileDependency>>,
    artifact_store: CanonicalArtifactStore,
    published: BTreeMap<PackageManifestKey, PublishedPackageArtifact>,
    visiting: Vec<PackageManifestKey>,
    visiting_set: BTreeSet<PackageManifestKey>,
}

impl<'a> PackageGraphCompiler<'a> {
    pub(super) fn new(
        platform_sources: &'a CompilerPlatformSources,
        manifests: BTreeMap<PackageManifestKey, PackageManifest>,
        contract_dependencies: &'a BTreeMap<
            PackageManifestKey,
            Vec<PackageContractCompileDependency>,
        >,
        artifact_store: CanonicalArtifactStore,
    ) -> Self {
        Self {
            platform_sources,
            manifests,
            contract_dependencies,
            artifact_store,
            published: BTreeMap::new(),
            visiting: Vec::new(),
            visiting_set: BTreeSet::new(),
        }
    }

    pub(super) fn compile_platform_std(&mut self) -> Result<(), PackageProjectCompileError> {
        let std_key = self
            .manifests
            .keys()
            .find(|(package_id, _)| package_id == SKIFF_STD_PUBLICATION_ID)
            .cloned();
        if let Some(std_key) = std_key {
            self.compile(&std_key)?;
        }
        Ok(())
    }

    pub(super) fn compile(
        &mut self,
        key: &PackageManifestKey,
    ) -> Result<PublishedPackageArtifact, PackageProjectCompileError> {
        if let Some(published) = self.published.get(key) {
            return Ok(published.clone());
        }
        if self.visiting_set.contains(key) {
            let first = self
                .visiting
                .iter()
                .position(|visiting| visiting == key)
                .unwrap_or(0);
            let coordinates = self.visiting[first..]
                .iter()
                .chain(std::iter::once(key))
                .map(coordinate)
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(PackageProjectCompileError::DependencyCycle { coordinates });
        }
        let manifest = self.manifests.get(key).cloned().ok_or_else(|| {
            PackageProjectCompileError::MissingDependencyManifest {
                package_id: key.0.clone(),
                package_version: key.1.clone(),
            }
        })?;

        self.visiting.push(key.clone());
        self.visiting_set.insert(key.clone());
        let result = self.compile_manifest(&manifest, None);
        self.visiting.pop();
        self.visiting_set.remove(key);
        let (mut published, service_api) = result?;
        debug_assert!(service_api.is_none());
        let receipt = publish_package_artifact_records(self.artifact_store.root(), &published)
            .map_err(|error| PackageProjectCompileError::CanonicalArtifactStore {
                message: error.to_string(),
            })?;
        published.artifact = self
            .artifact_store
            .read_package_artifact(&receipt.artifact)
            .map_err(|error| PackageProjectCompileError::CanonicalArtifactStore {
                message: error.to_string(),
            })?
            .as_ref()
            .clone();
        self.published.insert(key.clone(), published.clone());
        Ok(published)
    }

    pub(super) fn compile_service(
        &mut self,
        key: &PackageManifestKey,
        service_root: &ServicePackageRoot,
    ) -> Result<CompiledServicePackage, PackageProjectCompileError> {
        if self.published.contains_key(key) {
            return Err(PackageProjectCompileError::ServiceRootAlreadyCompiled {
                package_id: key.0.clone(),
                package_version: key.1.clone(),
            });
        }
        if self.visiting_set.contains(key) {
            let first = self
                .visiting
                .iter()
                .position(|visiting| visiting == key)
                .unwrap_or(0);
            let coordinates = self.visiting[first..]
                .iter()
                .chain(std::iter::once(key))
                .map(coordinate)
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(PackageProjectCompileError::DependencyCycle { coordinates });
        }
        let manifest = self.manifests.get(key).cloned().ok_or_else(|| {
            PackageProjectCompileError::MissingDependencyManifest {
                package_id: key.0.clone(),
                package_version: key.1.clone(),
            }
        })?;

        self.visiting.push(key.clone());
        self.visiting_set.insert(key.clone());
        let result = self.compile_manifest(&manifest, Some(service_root));
        self.visiting.pop();
        self.visiting_set.remove(key);
        let (package, service_api) = result?;
        let service_api = service_api.expect("service compile must project a service API");
        self.published.insert(key.clone(), package.clone());
        Ok(CompiledServicePackage {
            package,
            service_api,
        })
    }

    pub(super) fn compiled_dependency_closure(
        &self,
        root: &PublishedPackageArtifact,
    ) -> Result<Vec<PublishedPackageArtifact>, PackageProjectCompileError> {
        let mut closure = BTreeSet::new();
        let mut pending = requirement_keys(root);
        while let Some(key) = pending.pop() {
            if !closure.insert(key.clone()) {
                continue;
            }
            let dependency = self.published.get(&key).ok_or_else(|| {
                PackageProjectCompileError::MissingDependencyArtifact {
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

    fn compile_manifest(
        &mut self,
        manifest: &PackageManifest,
        service_root: Option<&ServicePackageRoot>,
    ) -> Result<
        (
            PublishedPackageArtifact,
            Option<skiff_compiler::ServiceApiProjection>,
        ),
        PackageProjectCompileError,
    > {
        let mut dependency_packages = Vec::with_capacity(manifest.dependencies.len());
        for dependency in &manifest.dependencies {
            let key = (dependency.id.clone(), dependency.version.clone());
            dependency_packages.push((dependency, self.compile(&key)?));
        }
        let dependency_artifacts = dependency_packages
            .iter()
            .map(|(_, package)| package.artifact.clone())
            .collect::<Vec<_>>();

        let package = read_package_source_input(self.platform_sources, manifest)?;
        let package_id = manifest.id.to_string();
        let aliases = package_alias_bindings(&manifest.dependencies, &self.manifests);
        let available_artifacts = self
            .published
            .values()
            .map(|published| published.artifact.clone())
            .collect::<Vec<PackageArtifact>>();
        let contract_dependencies = self
            .contract_dependencies
            .get(&(package_id.clone(), manifest.version.clone()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut input =
            PackageCompileInput::new(self.platform_sources, &package, &aliases, &package_id)
                .with_canonical_dependencies(&dependency_artifacts, contract_dependencies)
                .with_available_canonical_packages(&available_artifacts)
                .with_canonical_artifact_root(self.artifact_store.root());
        if manifest
            .dependencies
            .iter()
            .any(|dependency| dependency.top_level_alias.is_some())
        {
            input = input.for_test_service();
        }
        match service_root {
            Some(service_root) => {
                let compiled = compile_service_package(input, service_root)?;
                Ok((compiled.package, Some(compiled.service_api)))
            }
            None => Ok((compile_package(input)?, None)),
        }
    }
}

fn read_package_source_input(
    platform_sources: &CompilerPlatformSources,
    manifest: &PackageManifest,
) -> Result<PackageSourceInput, PackageProjectCompileError> {
    let root = package_root(manifest)?;
    let raw_sources = match manifest.provenance.owner {
        ManifestOwner::CompilerStandardPackage => {
            read_official_package_sources(platform_sources, manifest)?
        }
        ManifestOwner::UserOrBuiltinPackage => read_package_sources(manifest, &root)?,
    };
    let source_tree = raw_sources.source_tree();
    let source_graph =
        PublicationSourceGraph::parse_raw_publication_sources(&raw_sources.into_source_graph())?;
    let resources = read_publication_resources(&root, &manifest.resources)?;
    Ok(PackageSourceInput::new(
        manifest.publication.clone(),
        source_tree,
        source_graph,
        resources,
    ))
}

fn package_root(manifest: &PackageManifest) -> Result<PathBuf, PackageProjectCompileError> {
    manifest
        .provenance
        .path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| PackageProjectCompileError::MissingPackageRoot {
            path: manifest.provenance.path.display().to_string(),
        })
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

fn coordinate(key: &PackageManifestKey) -> String {
    format!("{}@{}", key.0, key.1)
}
