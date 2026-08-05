use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::SystemTime,
};

use sha2::{Digest, Sha256};
use skiff_artifact_identity::{
    assign_service_contract_identities, service_contract_ref, service_deployment_ref,
    ValidatedPackageArtifact,
};
use skiff_artifact_model::{
    DeploymentDiagnosticText, DeploymentGatewayEntry, DeploymentIngressBinding, DeploymentRevision,
    GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey, GatewayExternalSchema,
    IngressProtocol, IngressSelector, PackageArtifact, PackageArtifactRef, PackageBinding,
    PackageRequirementKey, PackageSchemaTypeId, PackageSchemaTypeRecord, ServiceContract,
    ServiceDeployment, ServiceDeploymentInput, ServiceDeploymentOperationInput,
    ServiceRequirementKey, ServiceSelectorBinding, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler_core::id::PublicationId;
use skiff_config_snapshot_tooling::{
    load_service_config, project_runtime_config_snapshot_with_base, ConfigSnapshotDeploymentInput,
    ConfigSnapshotPackageInput, ServiceConfigLayers,
};
use skiff_deployment::{
    assembly::resolve_runtime_assembly_with_validated_packages,
    projection::project_service_deployment_with_validated_packages,
};
use skiff_runtime_config_snapshot::new_runtime_config_snapshot_ref;

use crate::{
    canonical_fixture::{CanonicalFixtureError, SERVICE_TEST_FIXTURE_GUIDANCE},
    canonical_package::CanonicalPackageProject,
    canonical_store::{CanonicalBaseAssembly, CanonicalTestRecords},
    canonical_test_gateway::canonical_typed_null_gateway,
    test_discovery::TestServiceCase,
};

static TEST_SERVICE_EXECUTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const TEST_INGRESS_CONFIG_PATH: &str = "skiff.test.ingressUrl";
mod http_entry;

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
    pub entrypoint: CanonicalTestServiceEntrypoint,
}

#[derive(Debug, Clone)]
pub struct CanonicalTestServiceFixture {
    pub test_service: PackageArtifactRef,
    pub records: Arc<CanonicalTestRecords>,
    pub cases: Vec<CanonicalTestServiceCaseFixture>,
    package_identity_admission_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CanonicalTestServiceRunConfig {
    layers: ServiceConfigLayers,
}

impl CanonicalTestServiceFixture {
    #[doc(hidden)]
    pub fn package_identity_admission_count(&self) -> usize {
        self.package_identity_admission_count
    }

    pub fn publish(
        &self,
        source_artifact_root: &std::path::Path,
        runtime_artifact_root: &std::path::Path,
    ) -> Result<Vec<std::path::PathBuf>, CanonicalFixtureError> {
        self.records
            .publish(source_artifact_root, runtime_artifact_root)
    }
}

pub fn assemble_test_service_fixture(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    target_profile: &str,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    let scope = test_service_execution_nonce()?;
    let config = load_test_service_run_config(project, None)?;
    let mut admissions = PackageAdmissionCache::default();
    assemble_test_service_fixture_inner(
        project,
        cases,
        base,
        &scope,
        &config,
        target_profile,
        &mut admissions,
    )
}

pub fn assemble_test_service_fixture_for_run(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    run_scope: &str,
    target_profile: &str,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    let config = load_test_service_run_config(project, None)?;
    let mut admissions = PackageAdmissionCache::default();
    assemble_test_service_fixture_inner(
        project,
        cases,
        base,
        run_scope,
        &config,
        target_profile,
        &mut admissions,
    )
}

pub fn assemble_test_service_fixture_for_run_with_ingress(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    run_scope: &str,
    ingress_url: &str,
    target_profile: &str,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    let config = load_test_service_run_config(project, Some(ingress_url))?;
    let mut admissions = PackageAdmissionCache::default();
    assemble_test_service_fixture_inner(
        project,
        cases,
        base,
        run_scope,
        &config,
        target_profile,
        &mut admissions,
    )
}

pub(crate) fn load_test_service_run_config(
    project: &CanonicalPackageProject,
    ingress_url: Option<&str>,
) -> Result<CanonicalTestServiceRunConfig, CanonicalFixtureError> {
    let profile = project
        .test_service_profile
        .as_ref()
        .map(|profile| profile.profile_name.as_str())
        .ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "config snapshot projection requires service.yml kind: test: {}",
                SERVICE_TEST_FIXTURE_GUIDANCE
            ))
        })?;
    let mut layers = load_service_config(&project.source_root, profile)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    if let Some(ingress_url) = ingress_url {
        inject_runner_ingress_config(
            &mut layers,
            &project.package.artifact.package_id,
            ingress_url,
        )?;
    }
    Ok(CanonicalTestServiceRunConfig { layers })
}

pub(crate) fn assemble_test_service_fixture_for_run_with_config(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    run_scope: &str,
    config: &CanonicalTestServiceRunConfig,
    target_profile: &str,
    admissions: &mut PackageAdmissionCache,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    assemble_test_service_fixture_inner(
        project,
        cases,
        base,
        run_scope,
        config,
        target_profile,
        admissions,
    )
}

fn assemble_test_service_fixture_inner(
    project: &CanonicalPackageProject,
    cases: &[TestServiceCase],
    base: CanonicalBaseAssembly,
    run_scope: &str,
    config: &CanonicalTestServiceRunConfig,
    target_profile: &str,
    admissions: &mut PackageAdmissionCache,
) -> Result<CanonicalTestServiceFixture, CanonicalFixtureError> {
    if cases.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(
            "test service fixture requires at least one discovered case".to_string(),
        ));
    }
    let test_profile = project.test_service_profile.as_ref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "test execution requires service.yml kind: test".to_string(),
        )
    })?;
    let service_api = project.service_api.as_ref().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "compiled test service omitted its ordinary service API projection".to_string(),
        )
    })?;
    if run_scope.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(
            "test-service execution scope must be non-empty".to_string(),
        ));
    }
    let service_ids =
        test_service_case_ids(&project.package.artifact.package_id, run_scope, cases.len())?;
    let gateway_inputs = cases
        .iter()
        .enumerate()
        .map(|(index, case)| test_service_gateway_inputs(&project.package.artifact, index, case))
        .collect::<Result<Vec<_>, _>>()?;
    let mut deployment_packages = vec![project.package.artifact.clone()];
    deployment_packages.extend(project.dependency_packages.iter().cloned());
    let validated_deployment_packages = deployment_packages
        .iter()
        .map(|package| admissions.admit_clone(package))
        .collect::<Result<Vec<_>, _>>()?;
    let test_service_ref = validated_deployment_packages[0].reference().clone();
    let package_bindings = canonical_package_bindings(&validated_deployment_packages)?;
    let service_selectors = test_service_selectors(&deployment_packages, &base)?;
    let mut all_packages = deployment_packages.clone();
    let mut validated_all_packages = validated_deployment_packages.clone();
    extend_unique_validated_packages(
        &mut all_packages,
        &mut validated_all_packages,
        &base.packages,
        admissions,
    )?;
    let package_identity_admission_count = validated_all_packages.len();
    let generated_deployment_admissions = skiff_compiler::GeneratedServicePackageAdmissions::admit(
        &project.package.artifact,
        &project.dependency_packages,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let mut case_entries = Vec::with_capacity(cases.len());
    let mut contracts = Vec::with_capacity(cases.len());
    let mut deployments = Vec::with_capacity(cases.len());
    for (index, ((case, (gateway_entry_key, gateway_entry, ingress)), service_id)) in cases
        .iter()
        .zip(gateway_inputs)
        .zip(service_ids)
        .enumerate()
    {
        let contract = specialize_test_service_contract(&service_api.contract, service_id)?;
        let contract_ref = service_contract_ref(&contract)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let generated = http_entry::project(project, &contract, &generated_deployment_admissions)?;
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
        let deployment = project_service_deployment_with_validated_packages(
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
            ),
            &contract,
            &deployment_packages,
            &contract_package_schema_records(
                &contract,
                &project.package.resolved_package_schema_type_records,
            )?,
            &validated_deployment_packages,
        )
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let deployment_ref = service_deployment_ref(&deployment);
        let [ingress] = ingress.as_slice() else {
            return Err(CanonicalFixtureError::InvalidInput(
                "test-service case must produce exactly one ingress".to_string(),
            ));
        };
        contracts.push(contract);
        deployments.push(deployment);
        case_entries.push((
            contract_ref,
            CanonicalTestServiceEntrypoint {
                selector: ingress.selector.clone(),
                case: case.clone(),
                deployment: deployment_ref,
                gateway_entry_key,
                gateway_entry_identity: gateway_entry.gateway_entry_identity,
                mode: GatewayDispatchMode::Unary,
            },
        ));
    }
    let mut roots = base
        .assembly
        .as_ref()
        .map(|assembly| assembly.roots.clone())
        .unwrap_or_default();
    roots.extend(
        case_entries
            .iter()
            .map(|(_, entrypoint)| entrypoint.deployment.clone()),
    );
    let mut all_contracts = base.contracts.clone();
    all_contracts.extend(contracts.iter().cloned());
    let mut all_deployments = base.deployments.clone();
    all_deployments.extend(deployments.iter().cloned());
    let assembly = resolve_runtime_assembly_with_validated_packages(
        &roots,
        &all_deployments,
        &all_contracts,
        &all_packages,
        &validated_all_packages,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let config_snapshot = test_service_config_snapshot(
        project,
        &deployments,
        &deployment_packages,
        &base,
        config,
        target_profile,
    )?;
    let assembly_deployments = assembly
        .resolved_deployments
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let snapshot_deployments = config_snapshot
        .deployments()
        .iter()
        .map(|deployment| deployment.deployment().clone())
        .collect::<BTreeSet<_>>();
    if assembly_deployments != snapshot_deployments
        || snapshot_deployments.len() != config_snapshot.deployments().len()
    {
        return Err(CanonicalFixtureError::InvalidInput(
            "test assembly and config snapshot must contain the same exact deployment set"
                .to_string(),
        ));
    }
    let records = Arc::new(CanonicalTestRecords {
        packages: vec![project.package.clone()],
        contracts,
        deployments,
        assembly,
        config_snapshot,
        base_assembly: base.assembly.clone(),
    });
    let case_fixtures = case_entries
        .into_iter()
        .map(|(contract, entrypoint)| CanonicalTestServiceCaseFixture {
            contract,
            entrypoint,
        })
        .collect();
    Ok(CanonicalTestServiceFixture {
        test_service: test_service_ref,
        records,
        cases: case_fixtures,
        package_identity_admission_count,
    })
}

fn test_service_config_snapshot(
    project: &CanonicalPackageProject,
    deployments: &[ServiceDeployment],
    packages: &[PackageArtifact],
    base: &CanonicalBaseAssembly,
    config: &CanonicalTestServiceRunConfig,
    target_profile: &str,
) -> Result<skiff_runtime_config_snapshot::RuntimeConfigSnapshot, CanonicalFixtureError> {
    if base.assembly.is_some() != base.config_snapshot.is_some() {
        return Err(CanonicalFixtureError::InvalidInput(
            "base assembly and base config snapshot must form one exact pair".to_string(),
        ));
    }
    let package_inputs = packages
        .iter()
        .map(|package| ConfigSnapshotPackageInput {
            package_id: package.package_id.clone(),
            package_build_id: package.package_build_id.clone(),
            requirements: package.runtime_requirements.config.clone(),
        })
        .collect::<Vec<_>>();
    let inputs = deployments
        .iter()
        .map(|deployment| ConfigSnapshotDeploymentInput {
            deployment: service_deployment_ref(deployment),
            source_path: project.source_root.clone(),
            config: config.layers.clone(),
            packages: package_inputs.clone(),
        })
        .collect();
    project_runtime_config_snapshot_with_base(
        target_profile,
        new_runtime_config_snapshot_ref(),
        base.config_snapshot.as_ref(),
        inputs,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn inject_runner_ingress_config(
    config: &mut ServiceConfigLayers,
    implementation_package_id: &str,
    ingress_url: &str,
) -> Result<(), CanonicalFixtureError> {
    let package = config
        .entry(implementation_package_id.to_string())
        .or_default();
    let skiff = package
        .entry("skiff".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let skiff = skiff.as_object_mut().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!(
            "test service authored {TEST_INGRESS_CONFIG_PATH} below non-object path skiff"
        ))
    })?;
    let test = skiff
        .entry("test".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let test = test.as_object_mut().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!(
            "test service authored {TEST_INGRESS_CONFIG_PATH} below non-object path skiff.test"
        ))
    })?;
    if test.contains_key("ingressUrl") {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "test service authored reserved config path {TEST_INGRESS_CONFIG_PATH}"
        )));
    }
    test.insert(
        "ingressUrl".to_string(),
        serde_json::Value::String(ingress_url.to_string()),
    );
    Ok(())
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
    packages: &[ValidatedPackageArtifact],
) -> Result<Vec<PackageBinding>, CanonicalFixtureError> {
    let by_coordinate = packages
        .iter()
        .map(|package| {
            (
                (
                    package.artifact().package_id.as_str(),
                    package.artifact().package_version.as_str(),
                ),
                package,
            )
        })
        .collect::<BTreeMap<_, _>>();
    packages
        .iter()
        .flat_map(|caller| {
            caller
                .artifact()
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
            if dependency.artifact().package_local_abi.local_abi_identity
                != requirement.expected_local_abi
            {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "canonical dependency {} ABI does not match requirement {}",
                    requirement.package_id, requirement.alias
                )));
            }
            Ok(PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller.reference().package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package: dependency.reference().clone(),
            })
        })
        .collect()
}

/// Memoized in-memory admission for one exact immutable package closure.
///
/// Every activation batch assembles the same project/dependency/base package
/// closure. Admission recomputes canonical JSON and SHA-256 per package, which
/// dominates debug-build run planning, so admit each exact reference once and
/// reuse the validated token across batches. Rejecting conflicting content
/// under one reference preserves the previous fail-closed behavior.
#[derive(Debug, Default)]
pub(crate) struct PackageAdmissionCache {
    by_reference: BTreeMap<PackageArtifactRef, ValidatedPackageArtifact>,
}

impl PackageAdmissionCache {
    pub(crate) fn admit_clone(
        &mut self,
        artifact: &PackageArtifact,
    ) -> Result<ValidatedPackageArtifact, CanonicalFixtureError> {
        let reference = declared_package_ref(artifact);
        if let Some(admitted) = self.by_reference.get(&reference) {
            if !admitted.exactly_matches(artifact) {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "package reference {reference:?} has conflicting exact content"
                )));
            }
            return Ok(admitted.clone());
        }
        let admitted = ValidatedPackageArtifact::admit_clone(artifact)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        self.by_reference.insert(reference, admitted.clone());
        Ok(admitted)
    }
}

fn declared_package_ref(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
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
                    "runtime service requirement {}@{} needs exactly one --base-assembly contract; found {}: \
                     supply exact baseline identities as a pair \
                     (--base-assembly <identity> --base-config-snapshot <identity>); {}",
                    expected.service_id, expected.contract_version, matches.len(),
                    SERVICE_TEST_FIXTURE_GUIDANCE
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
) -> ServiceDeploymentInput {
    let revision = implementation
        .package_build_id
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or("test-service");
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
        diagnostic_text: DeploymentDiagnosticText {
            display_name: format!("{service_id} case {case_index}"),
            notes: BTreeMap::new(),
        },
    }
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

fn extend_unique_validated_packages(
    packages: &mut Vec<PackageArtifact>,
    validated: &mut Vec<ValidatedPackageArtifact>,
    candidates: &[PackageArtifact],
    admissions: &mut PackageAdmissionCache,
) -> Result<(), CanonicalFixtureError> {
    for candidate in candidates {
        let declared = declared_package_ref(candidate);
        if let Some(existing) = validated
            .iter()
            .find(|existing| existing.reference() == &declared)
        {
            if !existing.exactly_matches(candidate) {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "package reference {declared:?} has conflicting exact content"
                )));
            }
            continue;
        }
        let admission = admissions.admit_clone(candidate)?;
        packages.push(candidate.clone());
        validated.push(admission);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct TestServiceCaseOrigin<'a> {
    package_id: &'a str,
    case_index: usize,
}

fn test_service_case_ids(
    package_id: &str,
    execution_scope: &str,
    case_count: usize,
) -> Result<Vec<String>, CanonicalFixtureError> {
    let origins = (0..case_count)
        .map(|case_index| TestServiceCaseOrigin {
            package_id,
            case_index,
        })
        .collect::<Vec<_>>();
    test_service_case_ids_with_digest(
        &origins,
        execution_scope,
        test_service_package_digest,
        test_service_execution_digest,
    )
}

fn test_service_package_digest(package_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(package_id.as_bytes());
    let hash = format!("{:x}", digest.finalize());
    hash[..16].to_string()
}

fn test_service_execution_digest(execution_scope: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"skiff-test-service-execution-v1\0");
    digest.update(execution_scope.as_bytes());
    let hash = format!("{:x}", digest.finalize());
    hash[..16].to_string()
}

fn test_service_case_ids_with_digest(
    origins: &[TestServiceCaseOrigin<'_>],
    execution_scope: &str,
    package_digest: impl Fn(&str) -> String,
    execution_digest: impl Fn(&str) -> String,
) -> Result<Vec<String>, CanonicalFixtureError> {
    if execution_scope.is_empty() {
        return Err(CanonicalFixtureError::InvalidInput(
            "test-service execution scope must be non-empty".to_string(),
        ));
    }
    let execution_digest = execution_digest(execution_scope);
    let service_ids = origins
        .iter()
        .map(|origin| {
            format!(
                "test.skiff/p-{}/e-{}/case-{}",
                package_digest(origin.package_id),
                execution_digest,
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
mod tests;
