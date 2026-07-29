use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use sha2::{Digest, Sha256};
use skiff_artifact_identity::{
    assign_service_contract_identities, package_artifact_ref, service_contract_ref,
    service_deployment_ref,
};
use skiff_artifact_model::{
    ContractDiagnosticText, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentRevision, GatewayDispatchMode, GatewayEntryIdentity,
    GatewayEntryKey, GatewayExternalSchema, IngressProtocol, IngressSelector, PackageArtifact,
    PackageArtifactRef, PackageBinding, PackageRequirementKey, RuntimeCapabilityBinding,
    ServiceContract, ServiceDeployment, ServiceDeploymentInput, ServiceProtocolIdentity,
    ServiceRequirementKey, ServiceSelectorBinding, StateBinding, StateBindingKind,
    SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler_core::id::PublicationId;
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
mod http_entry;
mod profile;

use profile::{selected_profile_bindings, SelectedProfileBindings};

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

pub fn assemble_package_test_fixture(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    let scope = format!("fixture:{}", overlay.overlay.artifact.package_build_id);
    assemble_package_test_fixture_inner(project, overlay, base, &scope, None)
}

pub fn assemble_package_test_fixture_for_run(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
    run_scope: &str,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    assemble_package_test_fixture_inner(project, overlay, base, run_scope, None)
}

pub fn assemble_package_test_fixture_for_run_with_ingress(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
    run_scope: &str,
    ingress_url: &str,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    assemble_package_test_fixture_inner(project, overlay, base, run_scope, Some(ingress_url))
}

fn assemble_package_test_fixture_inner(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
    run_scope: &str,
    ingress_url: Option<&str>,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    let assembly_nonce = package_test_assembly_nonce()?;
    let gateway_inputs = overlay
        .bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| package_test_gateway_inputs(&overlay, index, binding))
        .collect::<Result<Vec<_>, _>>()?;
    let service_ids =
        package_test_service_ids(&overlay.production.package_id, overlay.bindings.len())?;
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
    let profile_bindings = selected_profile_bindings(
        project,
        &overlay.overlay.artifact,
        &state_requirements,
        owner,
        ingress_url,
    )?;
    let runtime_capability_bindings =
        selected_runtime_capability_bindings(project, &deployment_packages, owner);
    let mut contracts = Vec::with_capacity(overlay.bindings.len());
    let mut deployments = Vec::with_capacity(overlay.bindings.len());
    let mut entrypoints = Vec::with_capacity(overlay.bindings.len());
    for (index, ((binding, (gateway_entry_key, gateway_entry, ingress)), service_id)) in overlay
        .bindings
        .iter()
        .zip(gateway_inputs)
        .zip(service_ids)
        .enumerate()
    {
        let contract = compile_package_test_contract(&overlay, service_id)?;
        let contract_ref = service_contract_ref(&contract)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let case_scope = package_test_case_scope(run_scope, &assembly_nonce, index, &binding.case);
        let state_bindings = package_test_state_bindings(&state_requirements, &case_scope)?;
        let (mut gateway_entries, mut deployment_ingress) =
            http_entry::project(project, &overlay, &contract, ingress_url)?;
        if gateway_entries
            .insert(gateway_entry_key.clone(), gateway_entry.clone())
            .is_some()
        {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test service http.yml entry key {gateway_entry_key} conflicts with the package-test control entry"
            )));
        }
        deployment_ingress.extend(ingress.iter().cloned());
        let deployment = project_service_deployment(
            package_test_deployment_input(
                &overlay,
                contract_ref.clone(),
                overlay_ref.clone(),
                package_bindings.clone(),
                service_selectors.clone(),
                gateway_entries,
                deployment_ingress,
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
    service_id: String,
) -> Result<ServiceContract, CanonicalFixtureError> {
    let contract = canonical_zero_operation_contract(
        service_id,
        overlay.production.package_version.clone(),
        format!("package tests for {}", overlay.production.package_id),
    )?;
    PublicationId::parse(&contract.service_id).map_err(|error| {
        CanonicalFixtureError::InvalidInput(format!(
            "generated package-test service id {} is not canonical: {error}",
            contract.service_id
        ))
    })?;
    Ok(contract)
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
    let handler = entry.handler.as_ref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "package-test HTTP gateway is missing its required handler".to_string(),
        )
    })?;
    if handler != &binding.gateway_callable_id {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "package-test gateway handler {} does not match overlay binding {}",
            handler, binding.gateway_callable_id
        )));
    }
    Ok((
        key.clone(),
        entry,
        vec![DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
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
                collection_name_mapping: requirement.collection_name_mapping.clone(),
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

#[derive(Debug, Clone, Copy)]
struct PackageTestServiceOrigin<'a> {
    package_id: &'a str,
    case_index: usize,
}

fn package_test_service_ids(
    package_id: &str,
    case_count: usize,
) -> Result<Vec<String>, CanonicalFixtureError> {
    let origins = (0..case_count)
        .map(|case_index| PackageTestServiceOrigin {
            package_id,
            case_index,
        })
        .collect::<Vec<_>>();
    package_test_service_ids_with_digest(&origins, package_test_package_digest)
}

fn package_test_package_digest(package_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(package_id.as_bytes());
    let hash = format!("{:x}", digest.finalize());
    hash[..32].to_string()
}

fn package_test_service_ids_with_digest(
    origins: &[PackageTestServiceOrigin<'_>],
    package_digest: impl Fn(&str) -> String,
) -> Result<Vec<String>, CanonicalFixtureError> {
    let service_ids = origins
        .iter()
        .map(|origin| {
            format!(
                "test.skiff/p-{}/case-{}",
                package_digest(origin.package_id),
                origin.case_index
            )
        })
        .collect::<Vec<_>>();

    let mut origin_by_service_id = BTreeMap::new();
    for (origin, service_id) in origins.iter().zip(&service_ids) {
        if let Some(first) = origin_by_service_id.insert(service_id, origin) {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "generated package-test service id collision between package {} case index {} and package {} case index {}",
                first.package_id, first.case_index, origin.package_id, origin.case_index
            )));
        }
    }
    Ok(service_ids)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use skiff_compiler_core::id::PublicationId;

    use super::{
        canonical_zero_operation_contract, package_test_assembly_nonce, package_test_service_ids,
        package_test_service_ids_with_digest, PackageTestServiceOrigin,
    };

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

    #[test]
    fn package_test_service_ids_use_a_canonical_digest_coordinate() {
        assert!(
            PublicationId::parse("test.skiff/agine.ai/api-tests").is_err(),
            "the former dotted local segment must remain illegal"
        );

        let [service_id] = package_test_service_ids("agine.ai/api-tests", 1)
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            service_id,
            "test.skiff/p-df02597bee463b7bd9eb5efe9b37360e/case-0"
        );
        let digest = service_id
            .strip_prefix("test.skiff/p-")
            .and_then(|suffix| suffix.strip_suffix("/case-0"))
            .unwrap();
        assert_eq!(digest.len(), 32);
        assert!(
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
            "digest coordinate must match [0-9a-f]{{32}}"
        );
        assert_eq!(
            PublicationId::parse(&service_id).unwrap().as_str(),
            service_id
        );
    }

    #[test]
    fn package_test_service_ids_are_stable_and_separate_packages_and_cases() {
        let first = package_test_service_ids("example.com/package", 2).unwrap();
        assert_eq!(
            first,
            package_test_service_ids("example.com/package", 2).unwrap()
        );
        assert_ne!(
            first[0],
            package_test_service_ids("example.com/other-package", 1).unwrap()[0]
        );
        assert_ne!(first[0], first[1]);
    }

    #[test]
    fn package_test_service_ids_do_not_encode_version() {
        let [service_id] = package_test_service_ids("example.com/package", 1)
            .unwrap()
            .try_into()
            .unwrap();
        let first = canonical_zero_operation_contract(
            service_id.clone(),
            "1.0.0".to_string(),
            "first".to_string(),
        )
        .unwrap();
        let second = canonical_zero_operation_contract(
            service_id,
            "999.0.0".to_string(),
            "second".to_string(),
        )
        .unwrap();

        assert_eq!(first.service_id, second.service_id);
        assert_ne!(first.contract_version, second.contract_version);
    }

    #[test]
    fn package_test_service_ids_bound_arbitrarily_long_package_ids() {
        let package_id = format!("example.com/{}", "very-long-segment-".repeat(1_000));
        let [service_id] = package_test_service_ids(&package_id, 1)
            .unwrap()
            .try_into()
            .unwrap();

        assert_eq!(service_id.len(), 52);
        assert!(PublicationId::parse(&service_id).is_ok());
    }

    #[test]
    fn package_test_service_id_collision_reports_origins_without_digest_input() {
        let origins = [
            PackageTestServiceOrigin {
                package_id: "first.example/package",
                case_index: 7,
            },
            PackageTestServiceOrigin {
                package_id: "second.example/package",
                case_index: 7,
            },
        ];
        let digest_input = "test-only-secret-digest-input";
        let error = package_test_service_ids_with_digest(&origins, |_| {
            let _captured_without_diagnostic_exposure = digest_input;
            "00000000000000000000000000000000".to_string()
        })
        .unwrap_err()
        .to_string();

        assert!(error.contains("first.example/package"));
        assert!(error.contains("case index 7"));
        assert!(error.contains("second.example/package"));
        assert!(!error.contains(digest_input));
    }

    #[test]
    fn every_generated_package_test_contract_has_a_canonical_service_id() {
        for service_id in package_test_service_ids("agine.ai/api-tests", 16).unwrap() {
            let contract = canonical_zero_operation_contract(
                service_id,
                "2.3.4".to_string(),
                "canonical parser coverage".to_string(),
            )
            .unwrap();
            assert_eq!(
                PublicationId::parse(&contract.service_id).unwrap().as_str(),
                contract.service_id
            );
        }
    }
}
