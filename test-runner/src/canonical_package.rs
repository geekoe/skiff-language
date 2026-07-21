use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
};

use skiff_artifact_model::{ContractRequirement, PackageArtifact, PackageLocalAbiIdentity};
use skiff_compiler::{
    compile_package, PackageCompileError, PackageCompileInput, PackageContractCompileDependency,
    PackageSourceInput, PublishedPackageArtifact,
};
use skiff_compiler_input::{
    package_config::{
        discover_package_manifests, read_user_package_manifest, PackageConfigError,
        PackageManifest, PACKAGE_CONFIG_FILE,
    },
    package_sources::{read_official_package_sources, read_package_sources},
    read_publication_resources, InputAssemblyError, ManifestOwner,
};
use skiff_compiler_source::{source_graph::PublicationSourceGraph, SourceCompileError};
use skiff_deployment::storage::{CanonicalArtifactStore, EcosystemStorageError};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct CanonicalPackageProject {
    pub package: PublishedPackageArtifact,
    pub dependency_packages: Vec<PackageArtifact>,
    pub contract_dependencies: Vec<PackageContractCompileDependency>,
}

impl CanonicalPackageProject {
    pub fn artifacts(&self) -> impl Iterator<Item = &PackageArtifact> {
        std::iter::once(&self.package.artifact).chain(&self.dependency_packages)
    }

    pub fn artifact(&self, package_id: &str, package_version: &str) -> Option<&PackageArtifact> {
        self.artifacts().find(|package| {
            package.package_id == package_id && package.package_version == package_version
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
    #[error(transparent)]
    Storage(#[from] EcosystemStorageError),
    #[error(
        "package dependency {package_id}@{package_version} has no published canonical pointer"
    )]
    MissingDependencyPointer {
        package_id: String,
        package_version: String,
    },
    #[error(
        "contract dependency {service_id}@{contract_version} has no published canonical pointer"
    )]
    MissingContractPointer {
        service_id: String,
        contract_version: String,
    },
    #[error("package dependency {package_id}@{package_version} pointer ABI does not match the typed requirement")]
    DependencyAbiMismatch {
        package_id: String,
        package_version: String,
    },
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

/// Compile only the source package selected by `package.yml`.
///
/// Every dependency is loaded through an exact typed pointer and immutable record
/// in the canonical store. Dependency source is never discovered or compiled.
pub fn compile_package_project(
    root: &Path,
    artifact_root: &Path,
) -> Result<CanonicalPackageProject, CanonicalPackageProjectError> {
    let manifest = read_root_package_manifest(root)?;
    let store = CanonicalArtifactStore::open(artifact_root)?;
    let manifest_dependencies = read_package_dependency_closure(&store, &manifest)?;
    let direct_dependencies = manifest
        .dependencies
        .iter()
        .map(|dependency| {
            manifest_dependencies
                .iter()
                .find(|artifact| {
                    artifact.package_id == dependency.id
                        && artifact.package_version == dependency.version
                })
                .cloned()
                .ok_or_else(|| CanonicalPackageProjectError::MissingDependencyPointer {
                    package_id: dependency.id.clone(),
                    package_version: dependency.version.clone(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let contract_dependencies = read_contract_dependencies(&store, &manifest)?;
    let aliases = package_aliases(&manifest, &manifest_dependencies);
    let mut available = manifest_dependencies.clone();
    read_optional_platform_std(&store, &mut available)?;
    let source = read_package_source_input(&manifest)?;
    let package = compile_package_artifact(
        &source,
        &aliases,
        &direct_dependencies,
        &available,
        &contract_dependencies,
    )?;
    let dependency_packages = read_compiled_dependency_closure(&store, &package.artifact)?;
    Ok(CanonicalPackageProject {
        package,
        dependency_packages,
        contract_dependencies,
    })
}

pub(crate) fn read_root_package_manifest(
    root: &Path,
) -> Result<PackageManifest, CanonicalPackageProjectError> {
    let path = root.join(PACKAGE_CONFIG_FILE);
    match read_user_package_manifest(&path) {
        Ok(manifest) => Ok(manifest),
        Err(user_error) => {
            let Ok(manifests) = discover_package_manifests(root) else {
                return Err(user_error.into());
            };
            let canonical_path = path.canonicalize().ok();
            manifests
                .into_values()
                .find(|manifest| {
                    canonical_path.is_some()
                        && manifest.provenance.path.canonicalize().ok() == canonical_path
                })
                .ok_or(CanonicalPackageProjectError::PackageConfig(user_error))
        }
    }
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

fn read_package_dependency_closure(
    store: &CanonicalArtifactStore,
    manifest: &PackageManifest,
) -> Result<Vec<PackageArtifact>, CanonicalPackageProjectError> {
    read_package_closure(
        store,
        manifest
            .dependencies
            .iter()
            .map(|dependency| (dependency.id.clone(), dependency.version.clone(), None)),
    )
}

fn read_compiled_dependency_closure(
    store: &CanonicalArtifactStore,
    package: &PackageArtifact,
) -> Result<Vec<PackageArtifact>, CanonicalPackageProjectError> {
    read_package_closure(
        store,
        package.package_requirements.iter().map(|requirement| {
            (
                requirement.package_id.clone(),
                requirement.exact_version.clone(),
                Some(requirement.expected_local_abi.clone()),
            )
        }),
    )
}

fn read_package_closure(
    store: &CanonicalArtifactStore,
    roots: impl IntoIterator<Item = (String, String, Option<PackageLocalAbiIdentity>)>,
) -> Result<Vec<PackageArtifact>, CanonicalPackageProjectError> {
    let mut pending = VecDeque::from_iter(roots);
    let mut closure = BTreeMap::<(String, String), PackageArtifact>::new();
    while let Some((package_id, package_version, expected_abi)) = pending.pop_front() {
        if let Some(existing) = closure.get(&(package_id.clone(), package_version.clone())) {
            if expected_abi
                .as_ref()
                .is_some_and(|expected| &existing.package_local_abi.local_abi_identity != expected)
            {
                return Err(CanonicalPackageProjectError::DependencyAbiMismatch {
                    package_id,
                    package_version,
                });
            }
            continue;
        }
        let pointer = store
            .read_package_artifact_pointer(&package_id, &package_version)?
            .ok_or_else(|| CanonicalPackageProjectError::MissingDependencyPointer {
                package_id: package_id.clone(),
                package_version: package_version.clone(),
            })?;
        let artifact = store
            .read_package_artifact(&pointer.artifact)?
            .as_ref()
            .clone();
        if expected_abi
            .as_ref()
            .is_some_and(|expected| &artifact.package_local_abi.local_abi_identity != expected)
        {
            return Err(CanonicalPackageProjectError::DependencyAbiMismatch {
                package_id,
                package_version,
            });
        }
        pending.extend(artifact.package_requirements.iter().map(|requirement| {
            (
                requirement.package_id.clone(),
                requirement.exact_version.clone(),
                Some(requirement.expected_local_abi.clone()),
            )
        }));
        closure.insert((package_id, package_version), artifact);
    }
    Ok(closure.into_values().collect())
}

fn read_contract_dependencies(
    store: &CanonicalArtifactStore,
    manifest: &PackageManifest,
) -> Result<Vec<PackageContractCompileDependency>, CanonicalPackageProjectError> {
    manifest
        .contracts
        .iter()
        .map(|dependency| {
            let pointer = store
                .read_service_contract_pointer(
                    &dependency.service_id,
                    &dependency.contract_version,
                )?
                .ok_or_else(|| CanonicalPackageProjectError::MissingContractPointer {
                    service_id: dependency.service_id.clone(),
                    contract_version: dependency.contract_version.clone(),
                })?;
            let contract = store.read_service_contract(&pointer.contract)?;
            Ok(PackageContractCompileDependency {
                requirement: ContractRequirement {
                    alias: dependency.alias.clone(),
                    service_id: dependency.service_id.clone(),
                    contract_version: dependency.contract_version.clone(),
                    expected_protocol_identity: contract.service_protocol_identity.clone(),
                },
                contract: contract.as_ref().clone(),
            })
        })
        .collect()
}

fn read_optional_platform_std(
    store: &CanonicalArtifactStore,
    available: &mut Vec<PackageArtifact>,
) -> Result<(), CanonicalPackageProjectError> {
    if available
        .iter()
        .any(|artifact| artifact.package_id == "skiff.run/std")
    {
        return Ok(());
    }
    if let Some(pointer) = store.read_package_artifact_pointer("skiff.run/std", "1.0.0")? {
        available.push(
            store
                .read_package_artifact(&pointer.artifact)?
                .as_ref()
                .clone(),
        );
    }
    Ok(())
}

pub(crate) fn package_aliases(
    manifest: &PackageManifest,
    dependencies: &[PackageArtifact],
) -> BTreeMap<String, Vec<String>> {
    manifest
        .dependencies
        .iter()
        .filter_map(|dependency| {
            let alias = dependency.alias.clone()?;
            let artifact = dependencies.iter().find(|artifact| {
                artifact.package_id == dependency.id
                    && artifact.package_version == dependency.version
            })?;
            let mut roots = artifact
                .package_local_abi
                .public_symbols
                .keys()
                .map(|path| path.split('.').take(2).collect::<Vec<_>>().join("."))
                .collect::<Vec<_>>();
            roots.sort();
            roots.dedup();
            Some((alias, roots))
        })
        .collect()
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
