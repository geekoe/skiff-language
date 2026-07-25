#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use skiff_artifact_model::PackageArtifact;
use skiff_compiler::{
    compile_package, PackageCompileInput, PackageContractCompileDependency, PackageSourceInput,
    PublishedPackageArtifact, ResolvedPackageSchema,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_input::{
    package_config::{package_alias_bindings, PackageManifest, PackageManifestKey},
    package_sources::{read_official_package_sources, read_package_sources},
    read_publication_resources, CompilerPlatformSources, ManifestOwner,
};
use skiff_compiler_source::source_graph::PublicationSourceGraph;

use super::{
    package_project::PackageProjectCompileError, package_schemas::resolved_package_schema,
};

pub(super) struct PackageGraphCompiler<'a> {
    platform_sources: &'a CompilerPlatformSources,
    manifests: BTreeMap<PackageManifestKey, PackageManifest>,
    contract_dependencies: &'a BTreeMap<PackageManifestKey, Vec<PackageContractCompileDependency>>,
    explicit_package_schemas: &'a BTreeMap<PackageManifestKey, Vec<ResolvedPackageSchema>>,
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
        explicit_package_schemas: &'a BTreeMap<PackageManifestKey, Vec<ResolvedPackageSchema>>,
    ) -> Self {
        Self {
            platform_sources,
            manifests,
            contract_dependencies,
            explicit_package_schemas,
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
        let result = self.compile_manifest(&manifest);
        self.visiting.pop();
        self.visiting_set.remove(key);
        let published = result?;
        self.published.insert(key.clone(), published.clone());
        Ok(published)
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
    ) -> Result<PublishedPackageArtifact, PackageProjectCompileError> {
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
        let mut resolved_package_schemas = dependency_packages
            .iter()
            .map(|(dependency, package)| {
                resolved_package_schema(dependency.effective_alias(), package)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if manifest.id.as_str() != SKIFF_STD_PUBLICATION_ID
            && !resolved_package_schemas
                .iter()
                .any(|schema| schema.package_id() == SKIFF_STD_PUBLICATION_ID)
        {
            if let Some(std) = self
                .published
                .values()
                .find(|published| published.artifact.package_id == SKIFF_STD_PUBLICATION_ID)
            {
                resolved_package_schemas.push(resolved_package_schema("std", std)?);
            }
        }
        if let Some(explicit) = self
            .explicit_package_schemas
            .get(&(package_id.clone(), manifest.version.clone()))
        {
            for schema in explicit {
                if let Some(position) = resolved_package_schemas.iter().position(|candidate| {
                    candidate.alias() == schema.alias()
                        && candidate.package_id() == schema.package_id()
                        && candidate.exact_version() == schema.exact_version()
                }) {
                    resolved_package_schemas[position] = schema.clone();
                } else {
                    resolved_package_schemas.push(schema.clone());
                }
            }
        }
        let contract_dependencies = self
            .contract_dependencies
            .get(&(package_id.clone(), manifest.version.clone()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let input =
            PackageCompileInput::new(self.platform_sources, &package, &aliases, &package_id)
                .with_canonical_dependencies(&dependency_artifacts, contract_dependencies)
                .with_available_canonical_packages(&available_artifacts)
                .with_resolved_package_schemas(&resolved_package_schemas);
        Ok(compile_package(input)?)
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
