use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use skiff_artifact_identity::{
    assign_service_contract_identities, package_artifact_ref, service_contract_ref,
    service_deployment_ref,
};
use skiff_artifact_model::{
    ActivationPolicy, ConfigLiteralBinding, ContractDiagnosticText, DeploymentDiagnosticText,
    DeploymentGatewayEntry, DeploymentIngressBinding, DeploymentPolicy, DeploymentRevision,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayExternalSchema,
    IngressProtocol, IngressSelector, MetadataValue, PackageArtifact, PackageArtifactRef,
    PackageBinding, PackageRequirementKey, ResourceBinding, ResourcePolicy,
    RuntimeCapabilityBinding, SecretRefBinding, ServiceContract, ServiceDeployment,
    ServiceDeploymentInput, ServiceProtocolIdentity, ServiceRequirementKey, ServiceSelectorBinding,
    StateBinding, StateBindingKind, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use crate::{
    canonical_fixture::CanonicalFixtureError,
    canonical_package::CanonicalPackageProject,
    canonical_store::{CanonicalBaseAssembly, CanonicalTestRecords},
    canonical_test_gateway::canonical_typed_null_gateway,
    test_discovery::PackageTestCase,
    test_overlay::PublishedPackageTestOverlay,
};

static PACKAGE_TEST_ASSEMBLY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct CanonicalPackageTestEntrypoint {
    pub case: PackageTestCase,
    pub selector: IngressSelector,
    pub deployment: skiff_artifact_model::ServiceDeploymentRef,
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub mode: GatewayDispatchMode,
}

#[derive(Debug, Clone)]
pub struct CanonicalPackageTestFixture {
    pub production: PackageArtifactRef,
    pub overlay: PackageArtifactRef,
    pub records: CanonicalTestRecords,
    pub entrypoints: Vec<CanonicalPackageTestEntrypoint>,
}

#[derive(Debug, Clone)]
struct SelectedProfileBindings {
    config_literals: Vec<ConfigLiteralBinding>,
    secret_refs: Vec<SecretRefBinding>,
    resource_bindings: Vec<ResourceBinding>,
    policy: DeploymentPolicy,
}

pub fn assemble_package_test_fixture(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    let scope = format!("fixture:{}", overlay.overlay.artifact.package_build_id);
    assemble_package_test_fixture_for_run(project, overlay, base, &scope)
}

pub fn assemble_package_test_fixture_for_run(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
    run_scope: &str,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    let assembly_nonce = package_test_assembly_nonce()?;
    let gateway_inputs = overlay
        .bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| package_test_gateway_inputs(&overlay, index, binding))
        .collect::<Result<Vec<_>, _>>()?;
    let overlay_ref = package_artifact_ref(&overlay.overlay.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let production_ref = package_artifact_ref(&project.package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;

    let mut deployment_packages = vec![overlay.overlay.artifact.clone()];
    deployment_packages.extend(overlay.dependency_packages.iter().cloned());
    let package_bindings = canonical_package_bindings(&deployment_packages)?;
    let service_selectors = package_test_service_selectors(&deployment_packages, &base)?;
    let owner = binding_owner(&base, &production_ref)?;
    let state_requirements = package_test_state_requirements(&deployment_packages)?;
    let profile_bindings = selected_profile_bindings(project, &state_requirements, owner)?;
    let runtime_capability_bindings =
        selected_runtime_capability_bindings(project, &deployment_packages, owner);
    let mut contracts = Vec::with_capacity(overlay.bindings.len());
    let mut deployments = Vec::with_capacity(overlay.bindings.len());
    let mut entrypoints = Vec::with_capacity(overlay.bindings.len());
    for (index, (binding, (gateway_entry_key, gateway_entry, ingress))) in
        overlay.bindings.iter().zip(gateway_inputs).enumerate()
    {
        let contract = compile_package_test_contract(&overlay, index)?;
        let contract_ref = service_contract_ref(&contract)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let case_scope = package_test_case_scope(run_scope, &assembly_nonce, index, &binding.case);
        let state_bindings = package_test_state_bindings(&state_requirements, &case_scope)?;
        let deployment = project_service_deployment(
            package_test_deployment_input(
                &overlay,
                contract_ref.clone(),
                overlay_ref.clone(),
                package_bindings.clone(),
                service_selectors.clone(),
                BTreeMap::from([(gateway_entry_key.clone(), gateway_entry.clone())]),
                ingress.clone(),
                profile_bindings.clone(),
                state_bindings,
                runtime_capability_bindings.clone(),
            ),
            &contract,
            &deployment_packages,
            &BTreeMap::new(),
        )
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let deployment_ref = service_deployment_ref(&deployment);
        let [ingress] = ingress.as_slice() else {
            return Err(CanonicalFixtureError::InvalidInput(
                "package-test case must produce exactly one ingress".to_string(),
            ));
        };
        entrypoints.push(CanonicalPackageTestEntrypoint {
            selector: ingress.selector.clone(),
            case: binding.case.clone(),
            deployment: deployment_ref,
            gateway_entry_key,
            gateway_entry_identity: gateway_entry.gateway_entry_identity,
            mode: GatewayDispatchMode::Unary,
        });
        contracts.push(contract);
        deployments.push(deployment);
    }

    let mut all_packages = base.packages.clone();
    all_packages.push(project.package.artifact.clone());
    all_packages.extend(deployment_packages.iter().cloned());
    all_packages = unique_packages(all_packages)?;
    let mut all_contracts = base.contracts.clone();
    all_contracts.extend(contracts.iter().cloned());
    let mut all_deployments = base.deployments.clone();
    all_deployments.extend(deployments.iter().cloned());
    // The base assembly supplies immutable provider/binding candidates, not
    // additional roots. Keeping its production subject root would load both
    // the production Package and the test overlay for the same Package ID,
    // which is neither isolated nor a valid execution image.
    let roots = deployments
        .iter()
        .map(service_deployment_ref)
        .collect::<Vec<_>>();
    let assembly =
        resolve_runtime_assembly(&roots, &all_deployments, &all_contracts, &all_packages)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    Ok(CanonicalPackageTestFixture {
        production: overlay.production,
        overlay: overlay_ref,
        records: CanonicalTestRecords {
            packages: vec![project.package.clone(), overlay.overlay],
            contracts,
            deployments,
            assembly,
            base_assembly: base.assembly,
        },
        entrypoints,
    })
}

fn compile_package_test_contract(
    overlay: &PublishedPackageTestOverlay,
    index: usize,
) -> Result<ServiceContract, CanonicalFixtureError> {
    canonical_zero_operation_contract(
        format!(
            "test.skiff/package/{}/case-{index}",
            safe_coordinate(&overlay.production.package_id),
        ),
        overlay.production.package_version.clone(),
        format!("package tests for {}", overlay.production.package_id),
    )
}

pub(crate) fn canonical_zero_operation_contract(
    service_id: String,
    contract_version: String,
    diagnostic_service: String,
) -> Result<ServiceContract, CanonicalFixtureError> {
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id,
        contract_version,
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: diagnostic_service,
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    Ok(contract)
}

fn package_test_gateway_inputs(
    overlay: &PublishedPackageTestOverlay,
    index: usize,
    binding: &crate::test_overlay::PackageTestOverlayBinding,
) -> Result<
    (
        GatewayEntryKey,
        DeploymentGatewayEntry,
        Vec<DeploymentIngressBinding>,
    ),
    CanonicalFixtureError,
> {
    let key = GatewayEntryKey::parse("run")
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let entry = canonical_typed_null_gateway(
        &overlay.overlay.artifact,
        &binding.gateway_selector,
        GatewayExternalSchema::Null,
    )
    .map_err(CanonicalFixtureError::InvalidInput)?;
    if entry.handler != binding.gateway_callable_id {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "package-test gateway handler {} does not match overlay binding {}",
            entry.handler, binding.gateway_callable_id
        )));
    }
    Ok((
        key.clone(),
        entry,
        vec![DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: format!("case-{index}.package-test.skiff.localhost"),
                method: Some("POST".to_string()),
                path: format!("/__skiff/package-test/{index}"),
            },
            gateway_entry_key: key,
        }],
    ))
}

pub(crate) fn canonical_package_bindings(
    packages: &[PackageArtifact],
) -> Result<Vec<PackageBinding>, CanonicalFixtureError> {
    let by_coordinate = packages
        .iter()
        .map(|package| {
            (
                (
                    package.package_id.as_str(),
                    package.package_version.as_str(),
                ),
                package,
            )
        })
        .collect::<BTreeMap<_, _>>();
    packages
        .iter()
        .flat_map(|caller| {
            caller
                .package_requirements
                .iter()
                .map(move |requirement| (caller, requirement))
        })
        .map(|(caller, requirement)| {
            let dependency = by_coordinate
                .get(&(
                    requirement.package_id.as_str(),
                    requirement.exact_version.as_str(),
                ))
                .ok_or_else(|| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "canonical dependency {}@{} is absent from the exact closure",
                        requirement.package_id, requirement.exact_version
                    ))
                })?;
            if dependency.package_local_abi.local_abi_identity != requirement.expected_local_abi {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "canonical dependency {} ABI does not match requirement {}",
                    requirement.package_id, requirement.alias
                )));
            }
            Ok(PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package: package_artifact_ref(dependency)
                    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?,
            })
        })
        .collect()
}

fn package_test_service_selectors(
    packages: &[PackageArtifact],
    base: &CanonicalBaseAssembly,
) -> Result<Vec<ServiceSelectorBinding>, CanonicalFixtureError> {
    packages
        .iter()
        .flat_map(|caller| caller.service_requirements.iter().map(move |requirement| (caller, requirement)))
        .map(|(caller, requirement)| {
            let expected = &requirement.contract_requirement;
            let matches = base
                .contracts
                .iter()
                .filter(|contract| {
                    contract.service_id == expected.service_id
                        && contract.contract_version == expected.contract_version
                        && contract.service_protocol_identity == expected.expected_protocol_identity
                })
                .collect::<Vec<_>>();
            let [contract] = matches.as_slice() else {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "runtime service requirement {}@{} needs exactly one --base-assembly contract; found {}",
                    expected.service_id, expected.contract_version, matches.len()
                )));
            };
            Ok(ServiceSelectorBinding {
                key: ServiceRequirementKey {
                    caller_package_build_id: caller.package_build_id.clone(),
                    service_requirement_slot: requirement.service_binding_slot,
                },
                contract: service_contract_ref(contract)
                    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?,
            })
        })
        .collect()
}

fn binding_owner<'a>(
    base: &'a CanonicalBaseAssembly,
    production: &PackageArtifactRef,
) -> Result<Option<&'a ServiceDeployment>, CanonicalFixtureError> {
    let owners = base
        .deployments
        .iter()
        .filter(|deployment| &deployment.implementation == production)
        .collect::<Vec<_>>();
    match owners.as_slice() {
        [] => Ok(None),
        [owner] => Ok(Some(*owner)),
        many => Err(CanonicalFixtureError::InvalidInput(format!(
            "base assembly has {} binding owners for production package {}",
            many.len(),
            production.package_build_id
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn package_test_deployment_input(
    overlay: &PublishedPackageTestOverlay,
    contract: skiff_artifact_model::ServiceContractRef,
    implementation: PackageArtifactRef,
    package_bindings: Vec<PackageBinding>,
    service_selectors: Vec<ServiceSelectorBinding>,
    gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    ingress: Vec<DeploymentIngressBinding>,
    profile_bindings: SelectedProfileBindings,
    state_bindings: Vec<StateBinding>,
    runtime_capability_bindings: Vec<RuntimeCapabilityBinding>,
) -> ServiceDeploymentInput {
    let revision = implementation
        .package_build_id
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or("overlay");
    let SelectedProfileBindings {
        config_literals,
        secret_refs,
        resource_bindings,
        policy,
    } = profile_bindings;
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new(format!("test-{revision}")),
        implementation,
        operation_bindings: Vec::new(),
        package_bindings,
        service_selectors,
        gateway_entries,
        ingress,
        config_literals,
        secret_refs,
        state_bindings,
        resource_bindings,
        runtime_capability_bindings,
        policy,
        diagnostic_text: DeploymentDiagnosticText {
            display_name: format!("package tests for {}", overlay.production.package_id),
            notes: BTreeMap::from([
                ("configOwner".to_string(), "test deployment".to_string()),
                ("stateOwner".to_string(), "test deployment".to_string()),
                ("resourceOwner".to_string(), "test deployment".to_string()),
            ]),
        },
    }
}

fn package_test_state_requirements(
    packages: &[PackageArtifact],
) -> Result<BTreeMap<String, StateBindingKind>, CanonicalFixtureError> {
    let mut requirements = BTreeMap::<String, StateBindingKind>::new();
    for package in packages {
        for requirement in &package.runtime_requirements.state {
            match requirements.get(&requirement.key) {
                Some(kind) if kind != &requirement.kind => {
                    return Err(CanonicalFixtureError::InvalidInput(format!(
                        "package-test state requirement {} has conflicting kinds {:?} and {:?}",
                        requirement.key, kind, requirement.kind
                    )));
                }
                Some(_) => {}
                None => {
                    requirements.insert(requirement.key.clone(), requirement.kind);
                }
            }
        }
    }
    Ok(requirements)
}

fn package_test_state_bindings(
    requirements: &BTreeMap<String, StateBindingKind>,
    run_scope: &str,
) -> Result<Vec<StateBinding>, CanonicalFixtureError> {
    if run_scope.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(
            "package-test state run scope must be non-empty".to_string(),
        ));
    }
    Ok(requirements
        .iter()
        .map(|(requirement_key, kind)| StateBinding {
            namespace: package_test_state_namespace(run_scope, requirement_key, *kind),
            requirement_key: requirement_key.clone(),
            kind: *kind,
        })
        .collect())
}

fn package_test_assembly_nonce() -> Result<String, CanonicalFixtureError> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "package-test clock is before the Unix epoch: {error}"
            ))
        })?
        .as_nanos();
    let sequence = PACKAGE_TEST_ASSEMBLY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{}-{timestamp}-{sequence}", std::process::id()))
}

fn package_test_case_scope(
    run_scope: &str,
    assembly_nonce: &str,
    index: usize,
    case: &PackageTestCase,
) -> String {
    format!(
        "{run_scope}\0execution:{assembly_nonce}\0case:{index}\0{}::{}",
        case.module_path, case.name
    )
}

fn package_test_state_namespace(
    run_scope: &str,
    requirement_key: &str,
    kind: StateBindingKind,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"skiff-package-test-state-v1\0");
    digest.update(run_scope.as_bytes());
    digest.update(b"\0");
    if kind != StateBindingKind::Database {
        digest.update(requirement_key.as_bytes());
    }
    digest.update(b"\0");
    digest.update(format!("{kind:?}").as_bytes());
    let hash = format!("{:x}", digest.finalize());
    format!("skiff_pt_{}", &hash[..40])
}

/// Project the ordinary profile bindings that may be shared by every case.
///
/// State keys/kinds are validated separately; the runner replaces authored
/// namespaces with fresh per-case namespaces after that validation.
fn selected_profile_bindings(
    project: &CanonicalPackageProject,
    state_requirements: &BTreeMap<String, StateBindingKind>,
    owner: Option<&ServiceDeployment>,
) -> Result<SelectedProfileBindings, CanonicalFixtureError> {
    let Some(test_service) = &project.test_service_profile else {
        return Ok(owner
            .map(|deployment| SelectedProfileBindings {
                config_literals: deployment.config_literals.clone(),
                secret_refs: deployment.secret_refs.clone(),
                resource_bindings: deployment.resource_bindings.clone(),
                policy: deployment.policy.clone(),
            })
            .unwrap_or_else(default_package_test_policy));
    };
    let config = profile_map::<serde_json::Value>(test_service, "config")?;
    let secrets = profile_map::<String>(test_service, "secrets")?;
    let resources = profile_map::<TestResourceAuthoring>(test_service, "resources")?;
    let states = profile_map::<TestStateAuthoring>(test_service, "state")?;
    validate_test_service_states(test_service, state_requirements, &states)?;
    Ok(SelectedProfileBindings {
        config_literals: config
            .into_iter()
            .map(|(path, value)| ConfigLiteralBinding {
                path,
                value: MetadataValue::from_json(value),
            })
            .collect(),
        secret_refs: secrets
            .into_iter()
            .map(|(path, secret_ref)| SecretRefBinding { path, secret_ref })
            .collect(),
        resource_bindings: resources
            .into_iter()
            .map(|(requirement_key, binding)| ResourceBinding {
                requirement_key,
                capability: binding.capability,
                resource_ref: binding.resource_ref,
            })
            .collect(),
        policy: test_service_policy(test_service)?,
    })
}

fn default_package_test_policy() -> SelectedProfileBindings {
    SelectedProfileBindings {
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        resource_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: Some(30_000),
            resources: ResourcePolicy {
                cpu_millis: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            activation: ActivationPolicy {
                max_concurrency: 1,
                idle_timeout_ms: None,
            },
            principal: "test:package-runner".to_string(),
        },
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestStateAuthoring {
    kind: StateBindingKind,
    namespace: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestResourceAuthoring {
    capability: String,
    resource_ref: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestQuotaAuthoring {
    cpu_millis: u32,
    memory_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestLifecycleAuthoring {
    max_concurrency: u32,
    #[serde(default)]
    idle_timeout_ms: Option<u64>,
}

fn profile_map<T: for<'de> Deserialize<'de>>(
    test_service: &crate::canonical_package::CanonicalTestServiceProfile,
    field: &'static str,
) -> Result<BTreeMap<String, T>, CanonicalFixtureError> {
    let value = match field {
        "config" => &test_service.authoring.config,
        "secrets" => &test_service.authoring.secrets,
        "state" => &test_service.authoring.state,
        "resources" => &test_service.authoring.resources,
        _ => unreachable!("profile map field is compiler-owned"),
    };
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_value(value.clone()).map_err(|error| {
        CanonicalFixtureError::InvalidInput(format!(
            "test service {} config.{}.yml field {field} must be a path-keyed object: {error}",
            test_service.service_id, test_service.profile_name
        ))
    })
}

fn validate_test_service_states(
    test_service: &crate::canonical_package::CanonicalTestServiceProfile,
    expected: &BTreeMap<String, StateBindingKind>,
    authored: &BTreeMap<String, TestStateAuthoring>,
) -> Result<(), CanonicalFixtureError> {
    for (key, kind) in expected {
        let binding = authored.get(key).ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "test service {} config.{}.yml is missing state binding {key}",
                test_service.service_id, test_service.profile_name
            ))
        })?;
        if &binding.kind != kind {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test service {} config.{}.yml state binding {key} must be {kind:?}, got {:?}",
                test_service.service_id, test_service.profile_name, binding.kind
            )));
        }
        if binding.namespace.trim().is_empty() {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test service {} config.{}.yml state binding {key} namespace must not be empty",
                test_service.service_id, test_service.profile_name
            )));
        }
    }
    if let Some(key) = authored.keys().find(|key| !expected.contains_key(*key)) {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "test service {} config.{}.yml has extra state binding {key}",
            test_service.service_id, test_service.profile_name
        )));
    }
    Ok(())
}

fn test_service_policy(
    test_service: &crate::canonical_package::CanonicalTestServiceProfile,
) -> Result<DeploymentPolicy, CanonicalFixtureError> {
    let quota = profile_value::<TestQuotaAuthoring>(test_service, "quota")?;
    let lifecycle = profile_value::<TestLifecycleAuthoring>(test_service, "lifecycle")?;
    let principal = profile_value::<String>(test_service, "principal")?;
    let timeout_ms = if test_service.authoring.timeout.is_null() {
        None
    } else {
        Some(profile_value::<u64>(test_service, "timeout")?)
    };
    Ok(DeploymentPolicy {
        timeout_ms,
        resources: ResourcePolicy {
            cpu_millis: quota.cpu_millis,
            memory_bytes: quota.memory_bytes,
        },
        activation: ActivationPolicy {
            max_concurrency: lifecycle.max_concurrency,
            idle_timeout_ms: lifecycle.idle_timeout_ms,
        },
        principal,
    })
}

fn profile_value<T: for<'de> Deserialize<'de>>(
    test_service: &crate::canonical_package::CanonicalTestServiceProfile,
    field: &'static str,
) -> Result<T, CanonicalFixtureError> {
    let value = match field {
        "timeout" => &test_service.authoring.timeout,
        "quota" => &test_service.authoring.quota,
        "principal" => &test_service.authoring.principal,
        "lifecycle" => &test_service.authoring.lifecycle,
        _ => unreachable!("profile scalar field is compiler-owned"),
    };
    serde_json::from_value(value.clone()).map_err(|error| {
        CanonicalFixtureError::InvalidInput(format!(
            "test service {} config.{}.yml field {field} is invalid: {error}",
            test_service.service_id, test_service.profile_name
        ))
    })
}

fn selected_runtime_capability_bindings(
    project: &CanonicalPackageProject,
    packages: &[PackageArtifact],
    owner: Option<&ServiceDeployment>,
) -> Vec<RuntimeCapabilityBinding> {
    if project.test_service_profile.is_none() {
        return owner
            .map(|deployment| deployment.runtime_capability_bindings.clone())
            .unwrap_or_default();
    }
    packages
        .iter()
        .flat_map(|package| &package.runtime_requirements.runtime_capabilities)
        .map(|requirement| {
            (
                requirement.capability.clone(),
                requirement.required_version.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .map(|(capability, version)| RuntimeCapabilityBinding {
            capability,
            version,
        })
        .collect()
}

fn unique_packages(
    packages: Vec<PackageArtifact>,
) -> Result<Vec<PackageArtifact>, CanonicalFixtureError> {
    let mut unique = BTreeMap::new();
    for package in packages {
        let reference = package_artifact_ref(&package)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        unique.entry(reference).or_insert(package);
    }
    Ok(unique.into_values().collect())
}

fn safe_coordinate(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '/' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::package_test_assembly_nonce;

    #[test]
    fn package_test_assembly_nonces_are_unique_under_parallel_allocation() {
        let handles = (0..32)
            .map(|_| std::thread::spawn(package_test_assembly_nonce))
            .collect::<Vec<_>>();
        let nonces = handles
            .into_iter()
            .map(|handle| handle.join().expect("nonce worker").expect("nonce"))
            .collect::<BTreeSet<_>>();

        assert_eq!(nonces.len(), 32);
    }
}
