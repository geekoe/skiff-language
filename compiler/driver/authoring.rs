//! Canonical authoring consumers for the four-object ecosystem.
//!
//! This module is intentionally a thin coordinator. Authoring parsing, record
//! paths, identity validation, immutable writes, pointer CAS, deployment
//! projection and assembly closure resolution remain owned by their T01 typed
//! boundaries.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::{json, Map, Value};
use skiff_artifact_identity::{
    package_artifact_ref, runtime_assembly_ref, service_contract_ref, service_deployment_ref,
    PackageArtifactPointerPath, PackageFileIrRecordPath, PackageResourceRecordPath,
    RuntimeAssemblyPointerPath, ServiceContractPointerPath, ServiceDeploymentPointerPath,
};
use skiff_artifact_model::{
    parse_runtime_assembly_yml, parse_service_contract_definition_yml,
    parse_service_deployment_yml, ContractRequirement, PackageArtifact, PackageArtifactRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler_input::{
    package_config::{read_user_package_manifest, PackageManifest, PACKAGE_CONFIG_FILE},
    package_sources::read_package_sources,
    read_publication_resources, CompilerPlatformSources,
};
use skiff_compiler_source::prelude_registry::initialize_prelude_registry;
use skiff_compiler_source::source_graph::PublicationSourceGraph;
use skiff_deployment::{
    assembly::resolve_runtime_assembly,
    projection::project_service_deployment,
    storage::{
        CanonicalArtifactStore, PackageArtifactPointer, RuntimeAssemblyPointer,
        ServiceContractPointer, ServiceDeploymentPointer,
    },
};

use crate::{
    compile_contract, compile_package, PackageCompileInput, PackageContractCompileDependency,
    PackageSourceInput, PublishedPackageArtifact, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};

pub type AuthoringResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringObject {
    Package,
    Contract,
    Deployment,
    Assembly,
}

impl AuthoringObject {
    pub fn parse(value: &str) -> AuthoringResult<Self> {
        match value {
            "package" => Ok(Self::Package),
            "contract" => Ok(Self::Contract),
            "deployment" => Ok(Self::Deployment),
            "assembly" => Ok(Self::Assembly),
            _ => Err(invalid_input(format!(
                "unknown authoring object {value}; expected package, contract, deployment, or assembly"
            ))),
        }
    }
}

pub fn build_authoring_object(
    platform_sources: &CompilerPlatformSources,
    object: AuthoringObject,
    root: &Path,
    artifact_root: &Path,
    publish_pointer: bool,
) -> AuthoringResult<Value> {
    let build_non_package: fn(&Path, &CanonicalArtifactStore, bool) -> AuthoringResult<Value> =
        match object {
            AuthoringObject::Package => {
                return run_after_platform_context_guard(platform_sources, || {
                    let store = CanonicalArtifactStore::create(artifact_root)?;
                    build_package_after_platform_context_guard(
                        platform_sources,
                        root,
                        &store,
                        publish_pointer,
                    )
                });
            }
            AuthoringObject::Contract => build_contract,
            AuthoringObject::Deployment => build_deployment,
            AuthoringObject::Assembly => build_assembly,
        };
    let store = CanonicalArtifactStore::create(artifact_root)?;
    build_non_package(root, &store, publish_pointer)
}

fn run_after_platform_context_guard<T>(
    platform_sources: &CompilerPlatformSources,
    operation: impl FnOnce() -> AuthoringResult<T>,
) -> AuthoringResult<T> {
    initialize_prelude_registry(platform_sources)?;
    operation()
}

fn build_package_after_platform_context_guard(
    platform_sources: &CompilerPlatformSources,
    root: &Path,
    store: &CanonicalArtifactStore,
    publish_pointer: bool,
) -> AuthoringResult<Value> {
    let manifest = read_user_package_manifest(&root.join(PACKAGE_CONFIG_FILE))?;
    let dependencies = read_package_dependencies(store, &manifest)?;
    let contracts = read_contract_dependencies(store, &manifest)?;
    let aliases = package_aliases(&manifest, &dependencies);
    let package = read_package_source_input(root, &manifest)?;
    let package_id = manifest.id.to_string();
    let mut available = dependencies.clone();
    read_optional_platform_std(store, &mut available)?;
    let input = PackageCompileInput::new(platform_sources, &package, &aliases, &package_id)
        .with_canonical_dependencies(&dependencies, &contracts)
        .with_available_canonical_packages(&available);
    let published = compile_package(input)?;
    let receipt = write_package_records(store, &published)?;
    let mut output = Map::from_iter([("packageArtifactReceipt".to_string(), receipt)]);

    if publish_pointer {
        let reference = package_artifact_ref(&published.artifact)?;
        let candidate = PackageArtifactPointer::new(reference.clone())?;
        let expected = store
            .read_package_artifact_pointer(&reference.package_id, &reference.package_version)?;
        store.compare_and_swap_package_artifact_pointer(expected.as_ref(), &candidate)?;
        output.insert(
            "packagePointerReceipt".to_string(),
            json!({
                "pointer": candidate,
                "pointerPath": PackageArtifactPointerPath::new(
                    &reference.package_id,
                    &reference.package_version,
                )?.as_str(),
            }),
        );
    }
    Ok(Value::Object(output))
}

fn build_contract(
    root: &Path,
    store: &CanonicalArtifactStore,
    publish_pointer: bool,
) -> AuthoringResult<Value> {
    let path = authoring_path(root, "contract.yml");
    let parsed = parse_service_contract_definition_yml(&fs::read_to_string(&path)?)?;
    let contract = compile_contract(ServiceContractDefinition {
        service_id: parsed.service_id,
        contract_version: parsed.contract_version,
        operations: parsed.operations,
        boundary_schema: parsed.boundary_schema,
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: parsed.diagnostic_text.service,
            operations: parsed.diagnostic_text.operations,
            types: parsed.diagnostic_text.types,
        },
    })?;
    let record_path = store.write_service_contract(&contract)?;
    let reference = service_contract_ref(&contract)?;
    let mut output = Map::from_iter([(
        "serviceContractReceipt".to_string(),
        json!({
            "contract": reference,
            "recordPath": relative_path(store, &record_path)?,
        }),
    )]);

    if publish_pointer {
        let candidate = ServiceContractPointer::new(reference.clone())?;
        let expected = store
            .read_service_contract_pointer(&reference.service_id, &reference.contract_version)?;
        store.compare_and_swap_service_contract_pointer(expected.as_ref(), &candidate)?;
        output.insert(
            "serviceContractPointerReceipt".to_string(),
            json!({
                "pointer": candidate,
                "pointerPath": ServiceContractPointerPath::new(
                    &reference.service_id,
                    &reference.contract_version,
                )?.as_str(),
            }),
        );
    }
    Ok(Value::Object(output))
}

fn build_deployment(
    root: &Path,
    store: &CanonicalArtifactStore,
    publish_pointer: bool,
) -> AuthoringResult<Value> {
    let path = authoring_path(root, "deployment.yml");
    let input = parse_service_deployment_yml(&fs::read_to_string(&path)?)?;
    let contract = store.read_service_contract(&input.contract)?;
    let package_refs = std::iter::once(input.implementation.clone())
        .chain(
            input
                .package_bindings
                .iter()
                .map(|binding| binding.package.clone()),
        )
        .collect::<BTreeSet<_>>();
    let packages = package_refs
        .iter()
        .map(|reference| store.read_package_artifact(reference))
        .collect::<Result<Vec<_>, _>>()?;
    let package_values = packages
        .iter()
        .map(|artifact| artifact.as_ref().clone())
        .collect::<Vec<_>>();
    let deployment = project_service_deployment(input, &contract, &package_values)?;
    let record_path = store.write_service_deployment(&deployment)?;
    let reference = service_deployment_ref(&deployment);
    let mut output = Map::from_iter([(
        "serviceDeploymentReceipt".to_string(),
        json!({
            "deployment": reference,
            "recordPath": relative_path(store, &record_path)?,
        }),
    )]);

    if publish_pointer {
        let candidate = ServiceDeploymentPointer::new(reference.clone())?;
        let expected = store
            .read_service_deployment_pointer(&reference.service_id, &reference.contract_version)?;
        store.compare_and_swap_service_deployment_pointer(expected.as_ref(), &candidate)?;
        output.insert(
            "serviceDeploymentPointerReceipt".to_string(),
            json!({
                "pointer": candidate,
                "pointerPath": ServiceDeploymentPointerPath::new(
                    &reference.service_id,
                    &reference.contract_version,
                )?.as_str(),
            }),
        );
    }
    Ok(Value::Object(output))
}

fn build_assembly(
    root: &Path,
    store: &CanonicalArtifactStore,
    publish_pointer: bool,
) -> AuthoringResult<Value> {
    let path = authoring_path(root, "assembly.yml");
    let authoring = parse_runtime_assembly_yml(&fs::read_to_string(&path)?)?;
    let deployment_values = read_assembly_deployments(store, &authoring.root_deployments)?;
    let contracts = read_assembly_contracts(store, &deployment_values)?;
    let packages = read_assembly_packages(store, &deployment_values)?;
    let assembly = resolve_runtime_assembly(
        &authoring.root_deployments,
        &deployment_values,
        &contracts,
        &packages,
    )?;
    let record_path = store.write_runtime_assembly(&assembly)?;
    let reference = runtime_assembly_ref(&assembly)?;
    let mut output = Map::from_iter([(
        "runtimeAssemblyReceipt".to_string(),
        json!({
            "environment": authoring.environment,
            "assembly": reference,
            "recordPath": relative_path(store, &record_path)?,
        }),
    )]);

    if publish_pointer {
        let release = authoring.environment;
        let candidate = RuntimeAssemblyPointer::new(&release, reference.clone())?;
        let expected = store.read_runtime_assembly_pointer(&release)?;
        store.compare_and_swap_runtime_assembly_pointer(expected.as_ref(), &candidate)?;
        output.insert(
            "runtimeAssemblyPointerReceipt".to_string(),
            json!({
                "pointer": candidate,
                "pointerPath": RuntimeAssemblyPointerPath::new(&release)?.as_str(),
            }),
        );
    }
    Ok(Value::Object(output))
}

fn read_package_source_input(
    root: &Path,
    manifest: &PackageManifest,
) -> AuthoringResult<PackageSourceInput> {
    let raw_sources = read_package_sources(manifest, root)?;
    let source_tree = raw_sources.source_tree();
    let source_graph =
        PublicationSourceGraph::parse_raw_publication_sources(&raw_sources.into_source_graph())?;
    let resources = read_publication_resources(root, &manifest.resources)?;
    Ok(PackageSourceInput::new(
        manifest.publication.clone(),
        source_tree,
        source_graph,
        resources,
    ))
}

fn read_package_dependencies(
    store: &CanonicalArtifactStore,
    manifest: &PackageManifest,
) -> AuthoringResult<Vec<PackageArtifact>> {
    manifest
        .dependencies
        .iter()
        .map(|dependency| {
            let pointer = store
                .read_package_artifact_pointer(&dependency.id, &dependency.version)?
                .ok_or_else(|| {
                    invalid_input(format!(
                        "package dependency {}@{} has no published PackageArtifact pointer",
                        dependency.id, dependency.version
                    ))
                })?;
            Ok(store
                .read_package_artifact(&pointer.artifact)?
                .as_ref()
                .clone())
        })
        .collect()
}

fn read_contract_dependencies(
    store: &CanonicalArtifactStore,
    manifest: &PackageManifest,
) -> AuthoringResult<Vec<PackageContractCompileDependency>> {
    manifest
        .contracts
        .iter()
        .map(|dependency| {
            let pointer = store
                .read_service_contract_pointer(
                    &dependency.service_id,
                    &dependency.contract_version,
                )?
                .ok_or_else(|| {
                    invalid_input(format!(
                        "contract dependency {}@{} has no published ServiceContract pointer",
                        dependency.service_id, dependency.contract_version
                    ))
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
) -> AuthoringResult<()> {
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

fn package_aliases(
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

fn write_package_records(
    store: &CanonicalArtifactStore,
    published: &PublishedPackageArtifact,
) -> AuthoringResult<Value> {
    let mut artifact = published.artifact.clone();
    for file in &mut artifact.files {
        file.artifact_path = None;
    }
    for resource in &mut artifact.static_resources {
        resource.artifact_path = None;
    }
    let reference = package_artifact_ref(&artifact)?;
    for file in &mut artifact.files {
        file.artifact_path = Some(PackageFileIrRecordPath::new(&reference, file)?.to_string());
    }
    for resource in &mut artifact.static_resources {
        resource.artifact_path =
            Some(PackageResourceRecordPath::new(&reference, resource)?.to_string());
    }
    let mut file_paths = Vec::new();
    for file_ref in &artifact.files {
        let file = published
            .file_ir_units
            .iter()
            .find(|candidate| {
                candidate.identity == file_ref.file_ir_identity
                    && candidate.module_path == file_ref.module_path
            })
            .ok_or_else(|| {
                invalid_input(format!(
                    "PackageArtifact FileIrUnit {} has no emitted typed payload",
                    file_ref.file_ir_identity
                ))
            })?;
        let path = store.write_file_ir(&reference, file_ref, &file.unit)?;
        file_paths.push(relative_path(store, &path)?);
    }
    let mut resource_paths = Vec::new();
    for resource_ref in &artifact.static_resources {
        let resource = published
            .resource_blobs
            .iter()
            .find(|candidate| {
                candidate.logical_path == resource_ref.path
                    && candidate.sha256 == resource_ref.sha256
            })
            .ok_or_else(|| {
                invalid_input(format!(
                    "PackageArtifact resource {} has no emitted typed payload",
                    resource_ref.path
                ))
            })?;
        let path = store.write_static_resource(&reference, resource_ref, &resource.bytes)?;
        resource_paths.push(relative_path(store, &path)?);
    }
    let record_path = store.write_package_artifact(&artifact)?;
    Ok(json!({
        "artifact": reference,
        "recordPath": relative_path(store, &record_path)?,
        "fileIrRecordPaths": file_paths,
        "resourceRecordPaths": resource_paths,
    }))
}

fn read_assembly_contracts(
    store: &CanonicalArtifactStore,
    deployments: &[ServiceDeployment],
) -> AuthoringResult<Vec<ServiceContract>> {
    deployments
        .iter()
        .flat_map(|deployment| {
            std::iter::once(deployment.contract.clone()).chain(
                deployment
                    .service_selectors
                    .iter()
                    .map(|binding| binding.contract.clone()),
            )
        })
        .collect::<BTreeSet<ServiceContractRef>>()
        .iter()
        .map(|reference| Ok(store.read_service_contract(reference)?.as_ref().clone()))
        .collect()
}

fn read_assembly_deployments(
    store: &CanonicalArtifactStore,
    roots: &[ServiceDeploymentRef],
) -> AuthoringResult<Vec<ServiceDeployment>> {
    let mut deployments = BTreeMap::new();
    for reference in roots {
        deployments.insert(
            reference.clone(),
            store.read_service_deployment(reference)?.as_ref().clone(),
        );
    }
    let mut pending = VecDeque::from_iter(roots.iter().cloned());
    while let Some(reference) = pending.pop_front() {
        let deployment = deployments
            .get(&reference)
            .expect("pending deployment is loaded")
            .clone();
        for selector in &deployment.service_selectors {
            let contract = &selector.contract;
            if deployments
                .values()
                .any(|candidate| &candidate.contract == contract)
            {
                continue;
            }
            let pointer = store
                .read_service_deployment_pointer(&contract.service_id, &contract.contract_version)?
                .ok_or_else(|| {
                    invalid_input(format!(
                        "service dependency {}@{} has no published ServiceDeployment pointer",
                        contract.service_id, contract.contract_version
                    ))
                })?;
            if !deployments.contains_key(&pointer.deployment) {
                let provider = store
                    .read_service_deployment(&pointer.deployment)?
                    .as_ref()
                    .clone();
                pending.push_back(pointer.deployment.clone());
                deployments.insert(pointer.deployment, provider);
            }
        }
    }
    Ok(deployments.into_values().collect())
}

fn read_assembly_packages(
    store: &CanonicalArtifactStore,
    deployments: &[ServiceDeployment],
) -> AuthoringResult<Vec<PackageArtifact>> {
    deployments
        .iter()
        .flat_map(|deployment| {
            std::iter::once(deployment.implementation.clone()).chain(
                deployment
                    .package_bindings
                    .iter()
                    .map(|binding| binding.package.clone()),
            )
        })
        .collect::<BTreeSet<PackageArtifactRef>>()
        .iter()
        .map(|reference| Ok(store.read_package_artifact(reference)?.as_ref().clone()))
        .collect()
}

fn authoring_path(root: &Path, file_name: &str) -> PathBuf {
    if root.is_dir() {
        root.join(file_name)
    } else {
        root.to_path_buf()
    }
}

fn relative_path(store: &CanonicalArtifactStore, path: &Path) -> AuthoringResult<String> {
    Ok(path
        .strip_prefix(store.root())
        .map_err(|_| {
            invalid_input(format!(
                "record path {} escaped artifact root",
                path.display()
            ))
        })?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn invalid_input(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}

#[cfg(test)]
#[path = "authoring/tests.rs"]
mod tests;
