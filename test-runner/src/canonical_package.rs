use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use skiff_artifact_model::{
    ContractRequirement, PackageArtifact, PackageLocalAbiIdentity, ServiceAuthoringKind,
};
use skiff_compiler::{
    compile_package, compile_service_package, CompilerPlatformSources, PackageCompileError,
    PackageCompileInput, PackageContractCompileDependency, PackageSourceInput,
    PublishedPackageArtifact, ServiceApiProjection, ServicePackageCompileError,
};
use skiff_compiler_input::{
    package_config::{
        discover_package_manifests, read_user_package_manifest, PackageConfigError,
        PackageManifest, PACKAGE_CONFIG_FILE,
    },
    package_sources::{
        read_official_package_sources, read_package_sources, read_test_service_package_sources,
    },
    read_publication_resources, read_service_package_root, InputAssemblyError, ManifestOwner,
    ServicePackageRoot, ServiceSourceConfigError, HTTP_CONFIG_FILE, SERVICE_CONFIG_FILE,
    WEBSOCKET_CONFIG_FILE,
};
use skiff_compiler_source::{
    prelude_registry::{initialize_prelude_registry, PreludeRegistryInitializationError},
    source_graph::PublicationSourceGraph,
    SourceCompileError,
};
use skiff_deployment::storage::{CanonicalArtifactStore, EcosystemStorageError};
use thiserror::Error;

const TEST_SERVICE_CONFIG_PROFILE: &str = "skiff-test";

#[derive(Debug, Clone)]
pub struct CanonicalPackageProject {
    pub source_root: PathBuf,
    pub package: PublishedPackageArtifact,
    pub dependency_packages: Vec<PackageArtifact>,
    pub contract_dependencies: Vec<PackageContractCompileDependency>,
    pub test_service_profile: Option<CanonicalTestServiceProfile>,
    pub service_api: Option<ServiceApiProjection>,
}

#[derive(Debug, Clone)]
pub struct CanonicalTestServiceProfile {
    pub service_id: String,
    pub profile_name: String,
    pub service_root: ServicePackageRoot,
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
    PlatformContext(#[from] PreludeRegistryInitializationError),
    #[error(transparent)]
    PackageConfig(#[from] PackageConfigError),
    #[error(transparent)]
    ServiceConfig(#[from] ServiceSourceConfigError),
    #[error(transparent)]
    Input(#[from] InputAssemblyError),
    #[error(transparent)]
    Source(#[from] SourceCompileError),
    #[error(transparent)]
    Compile(#[from] PackageCompileError),
    #[error(transparent)]
    ServiceCompile(#[from] ServicePackageCompileError),
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
    #[error("test service {service_id} may only be compiled by the skiff test workflow")]
    TestServiceWorkflowRequired { service_id: String },
}

/// Compile one package source input through the production canonical pipeline.
pub fn compile_package_artifact(
    platform_sources: &CompilerPlatformSources,
    package: &PackageSourceInput,
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_packages: &[PackageArtifact],
    available_packages: &[PackageArtifact],
    contract_dependencies: &[PackageContractCompileDependency],
) -> Result<PublishedPackageArtifact, PackageCompileError> {
    compile_package_artifact_with_context(
        platform_sources,
        package,
        CanonicalPackageCompileContext::new(
            package_aliases,
            dependency_packages,
            available_packages,
            contract_dependencies,
        ),
    )
}

pub(crate) struct CanonicalPackageCompileContext<'a> {
    package_aliases: &'a BTreeMap<String, Vec<String>>,
    dependency_packages: &'a [PackageArtifact],
    available_packages: &'a [PackageArtifact],
    contract_dependencies: &'a [PackageContractCompileDependency],
    canonical_artifact_store: Option<&'a CanonicalArtifactStore>,
    test_service: bool,
}

impl<'a> CanonicalPackageCompileContext<'a> {
    pub(crate) fn new(
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        dependency_packages: &'a [PackageArtifact],
        available_packages: &'a [PackageArtifact],
        contract_dependencies: &'a [PackageContractCompileDependency],
    ) -> Self {
        Self {
            package_aliases,
            dependency_packages,
            available_packages,
            contract_dependencies,
            canonical_artifact_store: None,
            test_service: false,
        }
    }

    pub(crate) fn with_store(mut self, store: &'a CanonicalArtifactStore) -> Self {
        self.canonical_artifact_store = Some(store);
        self
    }

    pub(crate) fn with_test_service(mut self, test_service: bool) -> Self {
        self.test_service = test_service;
        self
    }
}

pub(crate) fn compile_package_artifact_with_context(
    platform_sources: &CompilerPlatformSources,
    package: &PackageSourceInput,
    context: CanonicalPackageCompileContext<'_>,
) -> Result<PublishedPackageArtifact, PackageCompileError> {
    let package_id = package.manifest().id.to_string();
    let mut input = PackageCompileInput::new(
        platform_sources,
        package,
        context.package_aliases,
        &package_id,
    )
    .with_canonical_dependencies(context.dependency_packages, context.contract_dependencies)
    .with_available_canonical_packages(context.available_packages);
    if let Some(store) = context.canonical_artifact_store {
        input = input.with_canonical_artifact_root(store.root());
    }
    if context.test_service {
        input = input.for_test_service();
    }
    compile_package(input)
}

fn compile_test_service_artifact_with_context(
    platform_sources: &CompilerPlatformSources,
    package: &PackageSourceInput,
    service_root: &ServicePackageRoot,
    context: CanonicalPackageCompileContext<'_>,
) -> Result<skiff_compiler::CompiledServicePackage, ServicePackageCompileError> {
    let package_id = package.manifest().id.to_string();
    let mut input = PackageCompileInput::new(
        platform_sources,
        package,
        context.package_aliases,
        &package_id,
    )
    .with_canonical_dependencies(context.dependency_packages, context.contract_dependencies)
    .with_available_canonical_packages(context.available_packages)
    .for_test_service();
    if let Some(store) = context.canonical_artifact_store {
        input = input.with_canonical_artifact_root(store.root());
    }
    compile_service_package(input, service_root)
}

/// Compile only the source package selected by `package.yml`.
///
/// Every dependency is loaded through an exact typed pointer and immutable record
/// in the canonical store. Dependency source is never discovered or compiled.
pub fn compile_package_project(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
) -> Result<CanonicalPackageProject, CanonicalPackageProjectError> {
    compile_package_project_for_workflow(
        platform_sources,
        root,
        artifact_root,
        PackageCompileWorkflow::Ordinary,
    )
}

/// Compile the source root selected by a real `skiff test` invocation.
///
/// An explicit `kind: test` service is compiler-authorized for top-level
/// dependency access, includes its test-only sources in its own ordinary
/// package artifact and always binds the fixed `skiff-test` profile.
pub fn compile_package_project_for_test(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
) -> Result<CanonicalPackageProject, CanonicalPackageProjectError> {
    compile_package_project_for_workflow(
        platform_sources,
        root,
        artifact_root,
        PackageCompileWorkflow::Test,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageCompileWorkflow {
    Ordinary,
    Test,
}

fn compile_package_project_for_workflow(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
    workflow: PackageCompileWorkflow,
) -> Result<CanonicalPackageProject, CanonicalPackageProjectError> {
    run_after_platform_context_guard(platform_sources, || {
        compile_package_project_after_platform_context_guard(
            platform_sources,
            root,
            artifact_root,
            workflow,
        )
    })
}

fn run_after_platform_context_guard<T>(
    platform_sources: &CompilerPlatformSources,
    operation: impl FnOnce() -> Result<T, CanonicalPackageProjectError>,
) -> Result<T, CanonicalPackageProjectError> {
    initialize_prelude_registry(platform_sources)?;
    operation()
}

fn compile_package_project_after_platform_context_guard(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
    workflow: PackageCompileWorkflow,
) -> Result<CanonicalPackageProject, CanonicalPackageProjectError> {
    let manifest = read_root_package_manifest(platform_sources, root)?;
    let test_service_profile = read_test_service_profile(root, workflow)?;
    let is_test_service = test_service_profile.is_some();
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
    let source = read_package_source_input(platform_sources, &manifest, is_test_service)?;
    let context = CanonicalPackageCompileContext::new(
        &aliases,
        &direct_dependencies,
        &available,
        &contract_dependencies,
    )
    .with_store(&store)
    .with_test_service(is_test_service);
    let (package, service_api) = match &test_service_profile {
        Some(test_service) => {
            let compiled = compile_test_service_artifact_with_context(
                platform_sources,
                &source,
                &test_service.service_root,
                context,
            )?;
            (compiled.package, Some(compiled.service_api))
        }
        None => (
            compile_package_artifact_with_context(platform_sources, &source, context)?,
            None,
        ),
    };
    let dependency_packages = read_compiled_dependency_closure(&store, &package.artifact)?;
    Ok(CanonicalPackageProject {
        source_root: root.to_path_buf(),
        package,
        dependency_packages,
        contract_dependencies,
        test_service_profile,
        service_api,
    })
}

fn read_test_service_profile(
    root: &Path,
    workflow: PackageCompileWorkflow,
) -> Result<Option<CanonicalTestServiceProfile>, CanonicalPackageProjectError> {
    if !root.join(SERVICE_CONFIG_FILE).is_file() {
        if has_external_service_control_file(root)? {
            return match read_service_package_root(root) {
                Ok(_) => unreachable!(
                    "typed service root reader cannot accept external controls without a regular service.yml"
                ),
                Err(error) => Err(error.into()),
            };
        }
        return Ok(None);
    }
    let service = read_service_package_root(root)?;
    if service.service.kind != ServiceAuthoringKind::Test {
        return Ok(None);
    }
    if workflow != PackageCompileWorkflow::Test {
        return Err(CanonicalPackageProjectError::TestServiceWorkflowRequired {
            service_id: service.service.id,
        });
    }
    let service_id = service.service.id.clone();
    Ok(Some(CanonicalTestServiceProfile {
        service_id,
        profile_name: TEST_SERVICE_CONFIG_PROFILE.to_string(),
        service_root: service,
    }))
}

fn has_external_service_control_file(root: &Path) -> Result<bool, CanonicalPackageProjectError> {
    for file_name in [HTTP_CONFIG_FILE, WEBSOCKET_CONFIG_FILE] {
        let path = root.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(_) => return Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ServiceSourceConfigError::Read {
                    path: path.display().to_string(),
                    source,
                }
                .into());
            }
        }
    }
    Ok(false)
}

pub(crate) fn read_root_package_manifest(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
) -> Result<PackageManifest, CanonicalPackageProjectError> {
    let path = root.join(PACKAGE_CONFIG_FILE);
    match read_user_package_manifest(&path) {
        Ok(manifest) => Ok(manifest),
        Err(user_error) => {
            let Ok(manifests) = discover_package_manifests(platform_sources, root) else {
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

pub(crate) fn read_compiled_dependency_closure(
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
        .services
        .iter()
        .map(|dependency| {
            let pointer = store
                .read_service_contract_pointer(&dependency.id, &dependency.version)?
                .ok_or_else(|| CanonicalPackageProjectError::MissingContractPointer {
                    service_id: dependency.id.clone(),
                    contract_version: dependency.version.clone(),
                })?;
            let contract = store.read_service_contract(&pointer.contract)?;
            Ok(PackageContractCompileDependency {
                requirement: ContractRequirement {
                    alias: dependency.effective_alias().to_string(),
                    service_id: dependency.id.clone(),
                    contract_version: dependency.version.clone(),
                    expected_protocol_identity: contract.service_protocol_identity.clone(),
                },
                contract: contract.as_ref().clone(),
            })
        })
        .collect()
}

pub(crate) fn read_optional_platform_std(
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
    platform_sources: &CompilerPlatformSources,
    manifest: &PackageManifest,
    test_service: bool,
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
        ManifestOwner::CompilerStandardPackage => {
            read_official_package_sources(platform_sources, manifest)?
        }
        ManifestOwner::UserOrBuiltinPackage if test_service => {
            read_test_service_package_sources(manifest, &root)?
        }
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

#[cfg(test)]
#[path = "canonical_package/tests.rs"]
mod tests;
