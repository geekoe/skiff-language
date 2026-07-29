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
    DeploymentDiagnosticText, DeploymentGatewayEntry, DeploymentIngressBinding, DeploymentRevision,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayExternalSchema,
    IngressProtocol, IngressSelector, PackageArtifact, PackageArtifactRef, PackageBinding,
    PackageRequirementKey, PackageSchemaTypeId, PackageSchemaTypeRecord, RuntimeCapabilityBinding,
    ServiceContract, ServiceDeployment, ServiceDeploymentInput, ServiceDeploymentOperationInput,
    ServiceRequirementKey, ServiceSelectorBinding, StateBinding, StateBindingKind,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
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
    test_discovery::TestServiceCase,
};

static TEST_SERVICE_EXECUTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
mod http_entry;
mod profile;

use profile::{selected_profile_bindings, SelectedProfileBindings};

#[derive(Debug, Clone)]
pub struct CanonicalTestServiceEntrypoint {
    pub case: TestServiceCase,
    pub selector: IngressSelector,
    pub deployment: skiff_artifact_model::ServiceDeploymentRef,
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub mode: GatewayDispatchMode,
}

#[derive(Debug, Clone)]
pub struct CanonicalTestServiceCaseFixture {
    pub contract: skiff_artifact_model::ServiceContractRef,
    pub records: CanonicalTestRecords,
    pub entrypoint: CanonicalTestServiceEntrypoint,
}

#[derive(Debug, Clone)]
pub struct CanonicalTestServiceFixture {
    pub test_service: PackageArtifactRef,
    pub cases: Vec<CanonicalTestServiceCaseFixture>,
}

impl CanonicalTestServiceFixture {
    pub fn publish(
        &self,
        source_artifact_root: &std::path::Path,
        runtime_artifact_root: &std::path::Path,
    ) -> Result<Vec<std::path::PathBuf>, CanonicalFixtureError> {
        let mut written = Vec::new();
        for case in &self.cases {
            written.extend(
                case.records
                    .publish(source_artifact_root, runtime_artifact_root)?,
            );
        }
        written.sort();
        written.dedup();
        Ok(written)
    }
}

pub fn assemble_test_service_fixture(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    let scope = format!("fixture:{}", project.package.artifact.package_build_id);
    assemble_test_service_fixture_inner(project, cases, base, &scope, None)
}

pub fn assemble_test_service_fixture_for_run(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    run_scope: &str,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    assemble_test_service_fixture_inner(project, cases, base, run_scope, None)
}

pub fn assemble_test_service_fixture_for_run_with_ingress(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    run_scope: &str,
    ingress_url: &str,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    assemble_test_service_fixture_inner(project, cases, base, run_scope, Some(ingress_url))
}

fn assemble_test_service_fixture_inner(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    run_scope: &str,
    ingress_url: Option<&str>,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    if cases.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(
            "test service fixture requires at least one discovered case".to_string(),
        ));
    }
    let test_profile = project.test_service_profile.as_ref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "test execution requires service.yml kind: test and config.skiff-test.yml".to_string(),
        )
    })?;
    let service_api = project.service_api.as_ref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "compiled test service omitted its ordinary service API projection".to_string(),
        )
    })?;
    let execution_nonce = test_service_execution_nonce()?;
    let service_ids = test_service_case_ids(&project.package.artifact.package_id, cases.len())?;
    let gateway_inputs = cases
        .iter()
        .enumerate()
        .map(|(index, case)| test_service_gateway_inputs(&project.package.artifact, index, case))
        .collect::<Result<Vec<_>, _>>()?;
    let test_service_ref = package_artifact_ref(&project.package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;

    let mut deployment_packages = vec![project.package.artifact.clone()];
    deployment_packages.extend(project.dependency_packages.iter().cloned());
    let package_bindings = canonical_package_bindings(&deployment_packages)?;
    let service_selectors = test_service_selectors(&deployment_packages, &base)?;
    let state_requirements = test_service_state_requirements(&deployment_packages)?;
    let profile_bindings = selected_profile_bindings(
        project,
        &project.package.artifact,
        &state_requirements,
        None,
        ingress_url,
    )?;
    let runtime_capability_bindings =
        selected_runtime_capability_bindings(project, &deployment_packages, None);
    let mut case_fixtures = Vec::with_capacity(cases.len());
    for (index, ((case, (gateway_entry_key, gateway_entry, ingress)), service_id)) in cases
        .iter()
        .zip(gateway_inputs)
        .zip(service_ids)
        .enumerate()
    {
        let contract = specialize_test_service_contract(&service_api.contract, service_id)?;
        let contract_ref = service_contract_ref(&contract)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let case_scope = test_service_case_scope(run_scope, &execution_nonce, index, case);
        let state_bindings = test_service_state_bindings(&state_requirements, &case_scope)?;
        let generated = http_entry::project(project, &contract, ingress_url)?;
        let operation_bindings = generated
            .operation_bindings
            .into_iter()
            .map(|binding| ServiceDeploymentOperationInput {
                contract_operation_id: binding.contract_operation_id,
                package_callable_id: binding.package_callable_id,
            })
            .collect();
        let mut gateway_entries = generated.gateway_entries;
        let mut deployment_ingress = generated.ingress;
        if gateway_entries
            .insert(gateway_entry_key.clone(), gateway_entry.clone())
            .is_some()
        {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test service external manifest entry key {gateway_entry_key} conflicts with the runner-owned case entry"
            )));
        }
        deployment_ingress.extend(ingress.iter().cloned());
        let deployment = project_service_deployment(
            test_service_deployment_input(
                &test_profile.service_id,
                index,
                contract_ref.clone(),
                test_service_ref.clone(),
                operation_bindings,
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
            &contract_package_schema_records(
                &contract,
                &project.package.resolved_package_schema_type_records,
            )?,
        )
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let deployment_ref = service_deployment_ref(&deployment);
        let [ingress] = ingress.as_slice() else {
            return Err(CanonicalFixtureError::InvalidInput(
                "test-service case must produce exactly one ingress".to_string(),
            ));
        };
        let mut all_packages = base.packages.clone();
        all_packages.extend(deployment_packages.iter().cloned());
        let all_packages = unique_packages(all_packages)?;
        let mut all_contracts = base.contracts.clone();
        all_contracts.push(contract.clone());
        let mut all_deployments = base.deployments.clone();
        all_deployments.push(deployment.clone());
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&deployment_ref),
            &all_deployments,
            &all_contracts,
            &all_packages,
        )
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        case_fixtures.push(CanonicalTestServiceCaseFixture {
            contract: contract_ref,
            records: CanonicalTestRecords {
                packages: vec![project.package.clone()],
                contracts: vec![contract.clone()],
                deployments: vec![deployment],
                assembly,
                base_assembly: base.assembly.clone(),
            },
            entrypoint: CanonicalTestServiceEntrypoint {
                selector: ingress.selector.clone(),
                case: case.clone(),
                deployment: deployment_ref,
                gateway_entry_key,
                gateway_entry_identity: gateway_entry.gateway_entry_identity,
                mode: GatewayDispatchMode::Unary,
            },
        });
    }
    Ok(CanonicalTestServiceFixture {
        test_service: test_service_ref,
        cases: case_fixtures,
    })
}

fn contract_package_schema_records(
    contract: &ServiceContract,
    available: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>, CanonicalFixtureError> {
    contract
        .package_type_requirements
        .iter()
        .flat_map(|requirement| {
            requirement
                .required_type_ids
                .iter()
                .map(move |type_id| (&requirement.package_id, type_id))
        })
        .map(|(expected_owner, type_id)| {
            let record = available.get(type_id).ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "test-service contract requires unavailable Package schema record {type_id}"
                ))
            })?;
            if &record.package_id != expected_owner {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "test-service contract Package schema record {type_id} expected owner {expected_owner}, got {}",
                    record.package_id
                )));
            }
            Ok((type_id.clone(), record.clone()))
        })
        .collect()
}

fn specialize_test_service_contract(
    ordinary: &ServiceContract,
    service_id: String,
) -> Result<ServiceContract, CanonicalFixtureError> {
    PublicationId::parse(&service_id).map_err(|error| {
        CanonicalFixtureError::InvalidInput(format!(
            "generated test-service case id {service_id} is not canonical: {error}"
        ))
    })?;
    let mut contract = ordinary.clone();
    contract.service_id = service_id;
    assign_service_contract_identities(&mut contract)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    Ok(contract)
}

fn test_service_gateway_inputs(
    implementation: &PackageArtifact,
    index: usize,
    case: &TestServiceCase,
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
    let selector = format!("{}.{}Gateway", case.module_path, case.function_name);
    let entry =
        canonical_typed_null_gateway(implementation, &selector, GatewayExternalSchema::Null)
            .map_err(CanonicalFixtureError::InvalidInput)?;
    Ok((
        key.clone(),
        entry,
        vec![DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: format!("/__skiff/test/{index}"),
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

fn test_service_selectors(
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

#[allow(clippy::too_many_arguments)]
fn test_service_deployment_input(
    service_id: &str,
    case_index: usize,
    contract: skiff_artifact_model::ServiceContractRef,
    implementation: PackageArtifactRef,
    operation_bindings: Vec<ServiceDeploymentOperationInput>,
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
        .unwrap_or("test-service");
    let SelectedProfileBindings {
        config_literals,
        secret_refs,
        resource_bindings,
        policy,
    } = profile_bindings;
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new(format!("test-{revision}-case-{case_index}")),
        implementation,
        operation_bindings,
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
            display_name: format!("{service_id} case {case_index}"),
            notes: BTreeMap::from([
                ("configOwner".to_string(), "test deployment".to_string()),
                ("stateOwner".to_string(), "test deployment".to_string()),
                ("resourceOwner".to_string(), "test deployment".to_string()),
            ]),
        },
    }
}

fn test_service_state_requirements(
    packages: &[PackageArtifact],
) -> Result<BTreeMap<String, StateBindingKind>, CanonicalFixtureError> {
    let mut requirements = BTreeMap::<String, StateBindingKind>::new();
    for package in packages {
        for requirement in &package.runtime_requirements.state {
            match requirements.get(&requirement.key) {
                Some(kind) if kind != &requirement.kind => {
                    return Err(CanonicalFixtureError::InvalidInput(format!(
                        "test-service state requirement {} has conflicting kinds {:?} and {:?}",
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

fn test_service_state_bindings(
    requirements: &BTreeMap<String, StateBindingKind>,
    run_scope: &str,
) -> Result<Vec<StateBinding>, CanonicalFixtureError> {
    if run_scope.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(
            "test-service state run scope must be non-empty".to_string(),
        ));
    }
    Ok(requirements
        .iter()
        .map(|(requirement_key, kind)| StateBinding {
            namespace: test_service_state_namespace(run_scope, requirement_key, *kind),
            requirement_key: requirement_key.clone(),
            kind: *kind,
        })
        .collect())
}

fn test_service_execution_nonce() -> Result<String, CanonicalFixtureError> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| {
            CanonicalFixtureError::InvalidInput(format!(
                "test-service clock is before the Unix epoch: {error}"
            ))
        })?
        .as_nanos();
    let sequence = TEST_SERVICE_EXECUTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{}-{timestamp}-{sequence}", std::process::id()))
}

fn test_service_case_scope(
    run_scope: &str,
    assembly_nonce: &str,
    index: usize,
    case: &TestServiceCase,
) -> String {
    format!(
        "{run_scope}\0execution:{assembly_nonce}\0case:{index}\0{}::{}",
        case.module_path, case.name
    )
}

fn test_service_state_namespace(
    run_scope: &str,
    requirement_key: &str,
    kind: StateBindingKind,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"skiff-test-service-state-v1\0");
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
struct TestServiceCaseOrigin<'a> {
    package_id: &'a str,
    case_index: usize,
}

fn test_service_case_ids(
    package_id: &str,
    case_count: usize,
) -> Result<Vec<String>, CanonicalFixtureError> {
    let origins = (0..case_count)
        .map(|case_index| TestServiceCaseOrigin {
            package_id,
            case_index,
        })
        .collect::<Vec<_>>();
    test_service_case_ids_with_digest(&origins, test_service_package_digest)
}

fn test_service_package_digest(package_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(package_id.as_bytes());
    let hash = format!("{:x}", digest.finalize());
    hash[..32].to_string()
}

fn test_service_case_ids_with_digest(
    origins: &[TestServiceCaseOrigin<'_>],
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
                "generated test-service id collision between package {} case index {} and package {} case index {}",
                first.package_id, first.case_index, origin.package_id, origin.case_index
            )));
        }
    }
    Ok(service_ids)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        test_service_case_ids, test_service_case_ids_with_digest, test_service_execution_nonce,
        TestServiceCaseOrigin,
    };

    #[test]
    fn test_service_execution_nonces_are_unique_under_parallel_allocation() {
        let handles = (0..32)
            .map(|_| std::thread::spawn(test_service_execution_nonce))
            .collect::<Vec<_>>();
        let nonces = handles
            .into_iter()
            .map(|handle| handle.join().expect("nonce worker").expect("nonce"))
            .collect::<BTreeSet<_>>();

        assert_eq!(nonces.len(), 32);
    }

    #[test]
    fn case_ids_use_the_canonical_package_digest_coordinate() {
        let [service_id] = test_service_case_ids("agine.ai/api-tests", 1)
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            service_id,
            "test.skiff/p-df02597bee463b7bd9eb5efe9b37360e/case-0"
        );
    }

    #[test]
    fn case_id_collision_reports_both_origins_without_digest_input() {
        let origins = [
            TestServiceCaseOrigin {
                package_id: "first.example/package",
                case_index: 7,
            },
            TestServiceCaseOrigin {
                package_id: "second.example/package",
                case_index: 7,
            },
        ];
        let digest_input = "test-only-secret-digest-input";
        let error = test_service_case_ids_with_digest(&origins, |_| {
            let _captured_without_diagnostic_exposure = digest_input;
            "00000000000000000000000000000000".to_string()
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("first.example/package"));
        assert!(error.contains("second.example/package"));
        assert!(error.contains("case index 7"));
        assert!(!error.contains(digest_input));
    }
}
