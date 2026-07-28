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
    PackageArtifactPointerPath, RuntimeAssemblyPointerPath, ServiceContractPointerPath,
    ServiceDeploymentPointerPath,
};
use skiff_artifact_model::{
    parse_runtime_assembly_yml, ContractRequirement, PackageArtifact, PackageArtifactRef,
    ServiceAuthoringKind, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef,
};
use skiff_compiler_input::{
    package_config::{read_user_package_manifest, PackageManifest, PACKAGE_CONFIG_FILE},
    package_sources::read_package_sources,
    read_publication_resources, read_service_package_root, CompilerPlatformSources,
    HTTP_CONFIG_FILE, SERVICE_CONFIG_FILE, WEBSOCKET_CONFIG_FILE,
};
use skiff_compiler_source::prelude_registry::initialize_prelude_registry;
use skiff_compiler_source::source_graph::PublicationSourceGraph;
use skiff_deployment::{
    assembly::resolve_runtime_assembly,
    storage::{
        CanonicalArtifactStore, PackageArtifactPointer, RuntimeAssemblyPointer,
        ServiceContractPointer, ServiceDeploymentPointer,
    },
};

use crate::{
    compile_package, compile_service_package, generate_service_deployment,
    GeneratedServiceDeploymentInput, PackageCompileInput, PackageContractCompileDependency,
    PackageSourceInput,
};

mod package_publication;

pub use package_publication::{
    author_official_std_package, publish_package_artifact_records, PublishedPackageArtifactReceipt,
};

pub type AuthoringResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringObject {
    Package,
    Assembly,
}

impl AuthoringObject {
    pub fn parse(value: &str) -> AuthoringResult<Self> {
        match value {
            "package" => Ok(Self::Package),
            "assembly" => Ok(Self::Assembly),
            _ => Err(invalid_input(format!(
                "unknown authoring object {value}; expected package or assembly"
            ))),
        }
    }
}

pub fn build_authoring_object(
    platform_sources: &CompilerPlatformSources,
    object: AuthoringObject,
    root: &Path,
    artifact_root: &Path,
    environment: &str,
    publish_pointer: bool,
) -> AuthoringResult<Value> {
    let build_non_package: fn(&Path, &CanonicalArtifactStore, bool) -> AuthoringResult<Value> =
        match object {
            AuthoringObject::Package => {
                return run_after_platform_context_guard(platform_sources, || {
                    validate_external_manifest_inventory(root)?;
                    let manifest = read_user_package_manifest(&root.join(PACKAGE_CONFIG_FILE))?;
                    let store = CanonicalArtifactStore::create(artifact_root)?;
                    build_package_after_platform_context_guard(
                        platform_sources,
                        root,
                        &manifest,
                        &store,
                        environment,
                        publish_pointer,
                    )
                });
            }
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
    manifest: &PackageManifest,
    store: &CanonicalArtifactStore,
    environment: &str,
    publish_pointer: bool,
) -> AuthoringResult<Value> {
    reject_legacy_service_authoring(root)?;
    let service_root = root.join(SERVICE_CONFIG_FILE).is_file();
    if !service_root {
        reject_top_level_aliases_outside_test_service(root, manifest)?;
    }
    let dependencies = read_package_dependencies(store, &manifest)?;
    let contracts = read_contract_dependencies(store, &manifest)?;
    let aliases = package_aliases(&manifest, &dependencies);
    let package = read_package_source_input(root, &manifest)?;
    let package_id = manifest.id.to_string();
    let mut available = dependencies.clone();
    read_contract_provider_packages(store, &contracts, &mut available)?;
    read_optional_platform_std(store, &mut available)?;
    let input = PackageCompileInput::new(platform_sources, &package, &aliases, &package_id)
        .with_canonical_dependencies(&dependencies, &contracts)
        .with_available_canonical_packages(&available)
        .with_canonical_artifact_store(store);
    let (published, service_data, service_api_receipt) = if service_root {
        let service = read_service_package_root(root)?;
        if service.service.kind == ServiceAuthoringKind::Test {
            return Err(invalid_input(format!(
                "test service {} may be built only by `skiff test`; ordinary publish, deploy and watch workflows reject service.yml kind: test",
                root.display()
            )));
        }
        let compiled = compile_service_package(input, &service)?;
        let receipt = json!({
            "serviceId": &compiled.service_api.contract.service_id,
            "serviceProtocolIdentity": &compiled.service_api.contract.service_protocol_identity,
            "projection": &compiled.service_api.visibility,
        });
        (
            compiled.package,
            Some((service, compiled.service_api)),
            Some(receipt),
        )
    } else {
        let published = compile_package(input)?;
        (published, None, None)
    };
    let receipt = publish_package_artifact_records(store, &published)?;
    let mut output = Map::from_iter([(
        "packageArtifactReceipt".to_string(),
        serde_json::to_value(receipt)?,
    )]);
    if let Some(service_api_receipt) = service_api_receipt {
        output.insert("serviceApiReceipt".to_string(), service_api_receipt);
    }

    if let Some((service_root, service_api)) = service_data {
        let contract = &service_api.contract;
        let contract_path = store.write_service_contract(contract)?;
        let contract_ref = service_contract_ref(contract)?;
        output.insert(
            "serviceContractReceipt".to_string(),
            json!({
                "contract": contract_ref,
                "recordPath": relative_path(store, &contract_path)?,
            }),
        );

        let profile = match service_root.config_profiles.get(environment) {
            Some(profile) => &profile.authoring,
            None => {
                return Err(invalid_input(format!(
                    "service package {} requires config.{environment}.yml to generate its environment-specific ServiceDeployment",
                    root.display()
                )))
            }
        };
        let package_closure = reachable_package_closure(store, &published.artifact, &available)?;
        let package_schema_records = published.resolved_package_schema_type_records.clone();
        let deployment = generate_service_deployment(GeneratedServiceDeploymentInput {
            service: &service_root.service,
            http: service_root.http.as_ref(),
            websocket: service_root.websocket.as_ref(),
            profile_name: environment,
            profile,
            service_api: &service_api,
            implementation: &published.artifact,
            package_closure: &package_closure,
            package_schema_records: &package_schema_records,
        })?;
        let deployment_path = store.write_service_deployment(&deployment)?;
        let deployment_ref = service_deployment_ref(&deployment);
        output.insert(
            "serviceDeploymentReceipt".to_string(),
            json!({
                "deployment": deployment_ref,
                "recordPath": relative_path(store, &deployment_path)?,
            }),
        );

        if publish_pointer {
            let contract_candidate = ServiceContractPointer::new(contract_ref.clone())?;
            let expected_contract = store.read_service_contract_pointer(
                &contract_ref.service_id,
                &contract_ref.contract_version,
            )?;
            store.compare_and_swap_service_contract_pointer(
                expected_contract.as_ref(),
                &contract_candidate,
            )?;
            output.insert(
                "serviceContractPointerReceipt".to_string(),
                json!({
                    "pointer": contract_candidate,
                    "pointerPath": ServiceContractPointerPath::new(
                        &contract_ref.service_id,
                        &contract_ref.contract_version,
                    )?.as_str(),
                }),
            );

            let deployment_candidate = ServiceDeploymentPointer::new(deployment_ref.clone())?;
            let expected_deployment = store.read_service_deployment_pointer(
                &deployment_ref.service_id,
                &deployment_ref.contract_version,
            )?;
            store.compare_and_swap_service_deployment_pointer(
                expected_deployment.as_ref(),
                &deployment_candidate,
            )?;
            output.insert(
                "serviceDeploymentPointerReceipt".to_string(),
                json!({
                    "pointer": deployment_candidate,
                    "pointerPath": ServiceDeploymentPointerPath::new(
                        &deployment_ref.service_id,
                        &deployment_ref.contract_version,
                    )?.as_str(),
                }),
            );
        }
    }
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
        .services
        .iter()
        .map(|dependency| {
            let pointer = store
                .read_service_contract_pointer(&dependency.id, &dependency.version)?
                .ok_or_else(|| {
                    invalid_input(format!(
                        "service dependency {}@{} has no published ServiceContract pointer",
                        dependency.id, dependency.version
                    ))
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

fn read_contract_provider_packages(
    store: &CanonicalArtifactStore,
    contracts: &[PackageContractCompileDependency],
    available: &mut Vec<PackageArtifact>,
) -> AuthoringResult<()> {
    for dependency in contracts {
        let requirement = &dependency.requirement;
        let pointer = store
            .read_package_artifact_pointer(&requirement.service_id, &requirement.contract_version)?
            .ok_or_else(|| {
                invalid_input(format!(
                    "service dependency {}@{} has no published provider PackageArtifact pointer",
                    requirement.service_id, requirement.contract_version
                ))
            })?;
        available.push(
            store
                .read_package_artifact(&pointer.artifact)?
                .as_ref()
                .clone(),
        );
    }
    Ok(())
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

fn reachable_package_closure(
    store: &CanonicalArtifactStore,
    implementation: &PackageArtifact,
    loaded: &[PackageArtifact],
) -> AuthoringResult<Vec<PackageArtifact>> {
    resolve_reachable_package_closure(implementation, loaded, |package_id, package_version| {
        let pointer = store
            .read_package_artifact_pointer(package_id, package_version)?
            .ok_or_else(|| {
                invalid_input(format!(
                    "exact package requirement {package_id}@{package_version} has no published PackageArtifact pointer"
                ))
            })?;
        Ok(store
            .read_package_artifact(&pointer.artifact)?
            .as_ref()
            .clone())
    })
}

fn resolve_reachable_package_closure(
    implementation: &PackageArtifact,
    loaded: &[PackageArtifact],
    mut resolve: impl FnMut(&str, &str) -> AuthoringResult<PackageArtifact>,
) -> AuthoringResult<Vec<PackageArtifact>> {
    let mut candidates = loaded.to_vec();
    let mut pending = VecDeque::from_iter([implementation.clone()]);
    let mut visited = BTreeSet::new();
    let mut reachable = BTreeMap::new();

    while let Some(caller) = pending.pop_front() {
        if !visited.insert(caller.package_build_id.clone()) {
            continue;
        }
        for requirement in &caller.package_requirements {
            let matching_coordinates = candidates
                .iter()
                .filter(|candidate| {
                    candidate.package_id == requirement.package_id
                        && candidate.package_version == requirement.exact_version
                })
                .collect::<Vec<_>>();
            let candidate = if let Some(candidate) = matching_coordinates.iter().find(|candidate| {
                candidate.package_local_abi.local_abi_identity == requirement.expected_local_abi
                    && requirement
                        .expected_package_build
                        .as_ref()
                        .is_none_or(|expected| expected == &candidate.package_build_id)
            }) {
                (*candidate).clone()
            } else if matching_coordinates.is_empty() {
                let candidate = resolve(&requirement.package_id, &requirement.exact_version)?;
                validate_package_requirement_candidate(requirement, &candidate)?;
                candidates.push(candidate.clone());
                candidate
            } else {
                return Err(invalid_input(format!(
                    "exact package requirement {}@{} expected local ABI {}, but the loaded candidate has {}",
                    requirement.package_id,
                    requirement.exact_version,
                    requirement.expected_local_abi,
                    matching_coordinates[0]
                        .package_local_abi
                        .local_abi_identity
                )));
            };
            validate_package_requirement_candidate(requirement, &candidate)?;
            if candidate.package_build_id != implementation.package_build_id {
                reachable
                    .entry(candidate.package_build_id.clone())
                    .or_insert_with(|| candidate.clone());
            }
            pending.push_back(candidate);
        }
    }

    Ok(reachable.into_values().collect())
}

fn validate_package_requirement_candidate(
    requirement: &skiff_artifact_model::PackageRequirement,
    candidate: &PackageArtifact,
) -> AuthoringResult<()> {
    if candidate.package_id != requirement.package_id
        || candidate.package_version != requirement.exact_version
    {
        return Err(invalid_input(format!(
            "exact package requirement {}@{} resolved to {}@{}",
            requirement.package_id,
            requirement.exact_version,
            candidate.package_id,
            candidate.package_version
        )));
    }
    if candidate.package_local_abi.local_abi_identity != requirement.expected_local_abi {
        return Err(invalid_input(format!(
            "exact package requirement {}@{} expected local ABI {}, but resolved candidate has {}",
            requirement.package_id,
            requirement.exact_version,
            requirement.expected_local_abi,
            candidate.package_local_abi.local_abi_identity
        )));
    }
    if let Some(expected) = &requirement.expected_package_build {
        if &candidate.package_build_id != expected {
            return Err(invalid_input(format!(
                "exact package requirement {}@{} expected implementation build {}, but resolved candidate has {}",
                requirement.package_id,
                requirement.exact_version,
                expected,
                candidate.package_build_id
            )));
        }
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

fn reject_legacy_service_authoring(root: &Path) -> AuthoringResult<()> {
    for file_name in ["contract.yml", "deployment.yml"] {
        if root.join(file_name).exists() {
            return Err(invalid_input(format!(
                "{} is not an authoring input; ServiceContract and ServiceDeployment are generated from the service package root",
                root.join(file_name).display()
            )));
        }
    }
    Ok(())
}

fn reject_top_level_aliases_outside_test_service(
    root: &Path,
    manifest: &PackageManifest,
) -> AuthoringResult<()> {
    let aliases = manifest
        .dependencies
        .iter()
        .filter_map(|dependency| dependency.top_level_alias.as_deref())
        .collect::<Vec<_>>();
    if aliases.is_empty() {
        return Ok(());
    }
    Err(invalid_input(format!(
        "package {} declares topLevelAlias outside service.yml kind: test (aliases: {})",
        root.display(),
        aliases.join(", ")
    )))
}

fn validate_external_manifest_inventory(root: &Path) -> AuthoringResult<()> {
    let mut external = Vec::new();
    for file_name in [HTTP_CONFIG_FILE, WEBSOCKET_CONFIG_FILE] {
        let path = root.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    return Err(invalid_input(format!(
                        "external service manifest {} must be a regular file",
                        path.display()
                    )));
                }
                external.push(path);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if external.is_empty() {
        return Ok(());
    }
    for required in [PACKAGE_CONFIG_FILE, "api.yml", SERVICE_CONFIG_FILE] {
        let path = root.join(required);
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file()) {
            return Err(invalid_input(format!(
                "external service manifests require regular package.yml, api.yml, and service.yml in the same root; {} is missing or not a regular file",
                path.display()
            )));
        }
    }
    Ok(())
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

#[cfg(test)]
mod external_manifest_inventory_tests {
    use std::{
        fs, process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn root_inventory_rejects_external_only_and_ordinary_package_external_files() {
        let external_only = fixture_root("external-only");
        fs::write(external_only.join(HTTP_CONFIG_FILE), "{}\n").unwrap();
        assert!(validate_external_manifest_inventory(&external_only).is_err());
        fs::remove_dir_all(external_only).unwrap();

        let ordinary = fixture_root("ordinary");
        fs::write(
            ordinary.join(PACKAGE_CONFIG_FILE),
            "id: example.com/a\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(ordinary.join("api.yml"), "{}\n").unwrap();
        fs::write(ordinary.join(WEBSOCKET_CONFIG_FILE), "path: /chat\n").unwrap();
        assert!(validate_external_manifest_inventory(&ordinary).is_err());
        fs::remove_dir_all(ordinary).unwrap();
    }

    #[test]
    fn root_inventory_accepts_only_complete_regular_service_roots() {
        let complete = fixture_root("complete");
        fs::write(
            complete.join(PACKAGE_CONFIG_FILE),
            "id: example.com/a\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(complete.join("api.yml"), "{}\n").unwrap();
        fs::write(complete.join(SERVICE_CONFIG_FILE), "id: example.com/a\n").unwrap();
        fs::write(complete.join(HTTP_CONFIG_FILE), "{}\n").unwrap();
        validate_external_manifest_inventory(&complete).unwrap();
        fs::remove_dir_all(complete).unwrap();

        let non_regular = fixture_root("non-regular");
        fs::write(
            non_regular.join(PACKAGE_CONFIG_FILE),
            "id: example.com/a\nversion: 1.0.0\n",
        )
        .unwrap();
        fs::write(non_regular.join("api.yml"), "{}\n").unwrap();
        fs::write(non_regular.join(SERVICE_CONFIG_FILE), "id: example.com/a\n").unwrap();
        fs::create_dir(non_regular.join(HTTP_CONFIG_FILE)).unwrap();
        assert!(validate_external_manifest_inventory(&non_regular).is_err());
        fs::remove_dir_all(non_regular).unwrap();
    }

    fn fixture_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-authoring-inventory-{name}-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
