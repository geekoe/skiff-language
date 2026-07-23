use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skiff_artifact_identity::{package_artifact_ref, service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    ActivationPolicy, BoundaryCallableProjection, ConfigLiteralBinding, DeploymentDiagnosticText,
    DeploymentIngressBinding, DeploymentPolicy, DeploymentRevision, IngressProtocol,
    IngressSelector, MetadataValue, PackageArtifact, PackageArtifactRef, PackageBinding,
    PackageRequirementKey, ResourcePolicy, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentInput, ServiceDeploymentOperationInput, ServiceRequirementKey,
    ServiceSelectorBinding, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_contract, ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use crate::{
    canonical_fixture::CanonicalFixtureError,
    canonical_package::CanonicalPackageProject,
    canonical_store::{CanonicalBaseAssembly, CanonicalTestRecords},
    test_discovery::PackageTestCase,
    test_overlay::PublishedPackageTestOverlay,
};

#[derive(Debug, Clone)]
pub struct CanonicalPackageTestEntrypoint {
    pub case: PackageTestCase,
    pub selector: IngressSelector,
    pub deployment: skiff_artifact_model::ServiceDeploymentRef,
    pub contract: ServiceContractRef,
    pub operation: skiff_artifact_model::ContractOperationId,
}

#[derive(Debug, Clone)]
pub struct CanonicalPackageTestFixture {
    pub production: PackageArtifactRef,
    pub overlay: PackageArtifactRef,
    pub records: CanonicalTestRecords,
    pub entrypoints: Vec<CanonicalPackageTestEntrypoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestConfigLiteral {
    pub package: PackageArtifactRef,
    pub key: String,
    pub value: MetadataValue,
}

pub fn assemble_package_test_fixture(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    assemble_package_test_fixture_with_config(project, overlay, base, &[])
}

pub fn assemble_package_test_fixture_with_config(
    project: &CanonicalPackageProject,
    overlay: PublishedPackageTestOverlay,
    base: CanonicalBaseAssembly,
    test_config_literals: &[PackageTestConfigLiteral],
) -> Result<CanonicalPackageTestFixture, CanonicalFixtureError> {
    let contract = compile_package_test_contract(&overlay)?;
    let contract_ref = service_contract_ref(&contract)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let overlay_ref = package_artifact_ref(&overlay.overlay.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let production_ref = package_artifact_ref(&project.package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let (operation_bindings, ingress) = package_test_operation_inputs(&contract, &overlay)?;

    let mut deployment_packages = vec![overlay.overlay.artifact.clone()];
    deployment_packages.extend(project.dependency_packages.iter().cloned());
    let package_bindings = canonical_package_bindings(&deployment_packages)?;
    let service_selectors = package_test_service_selectors(&deployment_packages, &base)?;
    let owner = binding_owner(&base, &production_ref)?;
    let config_literals = package_test_config_literals(
        &deployment_packages,
        &production_ref,
        &overlay_ref,
        owner,
        test_config_literals,
    )?;
    let deployment = project_service_deployment(
        package_test_deployment_input(
            &overlay,
            contract_ref.clone(),
            overlay_ref.clone(),
            operation_bindings,
            package_bindings,
            service_selectors,
            ingress.clone(),
            owner,
            config_literals,
        ),
        &contract,
        &deployment_packages,
    )
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let deployment_ref = service_deployment_ref(&deployment);

    let mut all_packages = base.packages.clone();
    all_packages.push(project.package.artifact.clone());
    all_packages.extend(deployment_packages.iter().cloned());
    all_packages = unique_packages(all_packages)?;
    let mut all_contracts = base.contracts.clone();
    all_contracts.push(contract.clone());
    let mut all_deployments = base.deployments.clone();
    all_deployments.push(deployment.clone());
    let mut roots = base
        .assembly
        .as_ref()
        .map(|assembly| assembly.roots.clone())
        .unwrap_or_default();
    roots.push(deployment_ref.clone());
    let assembly =
        resolve_runtime_assembly(&roots, &all_deployments, &all_contracts, &all_packages)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    let entrypoints = overlay
        .bindings
        .into_iter()
        .zip(ingress)
        .map(|(binding, ingress)| CanonicalPackageTestEntrypoint {
            operation: ingress.contract_operation_id,
            selector: ingress.selector,
            case: binding.case,
            deployment: deployment_ref.clone(),
            contract: contract_ref.clone(),
        })
        .collect();
    Ok(CanonicalPackageTestFixture {
        production: overlay.production,
        overlay: overlay_ref,
        records: CanonicalTestRecords {
            packages: vec![project.package.clone(), overlay.overlay],
            contracts: vec![contract],
            deployments: vec![deployment],
            assembly,
            base_assembly: base.assembly,
        },
        entrypoints,
    })
}

fn compile_package_test_contract(
    overlay: &PublishedPackageTestOverlay,
) -> Result<ServiceContract, CanonicalFixtureError> {
    let mut operations = BTreeMap::new();
    for (index, binding) in overlay.bindings.iter().enumerate() {
        let projection = overlay
            .overlay
            .artifact
            .boundary_projections
            .get(&binding.callable_id)
            .ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "test callable {} has no boundary projection",
                    binding.callable_id
                ))
            })?;
        let operation_contract = match projection {
            BoundaryCallableProjection::Available {
                operation_contract, ..
            } => operation_contract,
            BoundaryCallableProjection::Unavailable { reasons } => {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "test callable {} cannot cross the canonical test boundary: {reasons:?}",
                    binding.callable_id
                )));
            }
        };
        operations.insert(format!("case{index}"), operation_contract.clone());
    }
    compile_contract(ServiceContractDefinition {
        service_id: format!(
            "test.skiff/package/{}",
            safe_coordinate(&overlay.production.package_id)
        ),
        contract_version: overlay.production.package_version.clone(),
        operations,
        boundary_schema: BTreeMap::new(),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: format!("package tests for {}", overlay.production.package_id),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    })
    .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
}

fn package_test_operation_inputs(
    contract: &ServiceContract,
    overlay: &PublishedPackageTestOverlay,
) -> Result<
    (
        Vec<ServiceDeploymentOperationInput>,
        Vec<DeploymentIngressBinding>,
    ),
    CanonicalFixtureError,
> {
    let mut operations = contract
        .operations
        .values()
        .map(|descriptor| {
            (
                descriptor.stable_key.as_str(),
                descriptor.operation_id.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut operation_bindings = Vec::new();
    let mut ingress = Vec::new();
    for (index, binding) in overlay.bindings.iter().enumerate() {
        let stable_key = format!("case{index}");
        let operation = operations.remove(stable_key.as_str()).ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "compiled test contract omitted stable key {stable_key}"
            ))
        })?;
        operation_bindings.push(ServiceDeploymentOperationInput {
            contract_operation_id: operation.clone(),
            package_public_path: binding.public_path.clone(),
        });
        ingress.push(DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: format!("case-{index}.package-test.skiff.localhost"),
                method: Some("POST".to_string()),
                path: format!("/__skiff/package-test/{index}"),
            },
            contract_operation_id: operation,
        });
    }
    Ok((operation_bindings, ingress))
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
    contract: ServiceContractRef,
    implementation: PackageArtifactRef,
    operation_bindings: Vec<ServiceDeploymentOperationInput>,
    package_bindings: Vec<PackageBinding>,
    service_selectors: Vec<ServiceSelectorBinding>,
    ingress: Vec<DeploymentIngressBinding>,
    owner: Option<&ServiceDeployment>,
    config_literals: Vec<ConfigLiteralBinding>,
) -> ServiceDeploymentInput {
    let revision = implementation
        .package_build_id
        .as_str()
        .rsplit(':')
        .next()
        .unwrap_or("overlay");
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new(format!("test-{revision}")),
        implementation,
        operation_bindings,
        package_bindings,
        service_selectors,
        ingress,
        config_literals,
        secret_refs: owner
            .map(|value| value.secret_refs.clone())
            .unwrap_or_default(),
        state_bindings: owner
            .map(|value| value.state_bindings.clone())
            .unwrap_or_default(),
        resource_bindings: owner
            .map(|value| value.resource_bindings.clone())
            .unwrap_or_default(),
        runtime_capability_bindings: owner
            .map(|value| value.runtime_capability_bindings.clone())
            .unwrap_or_default(),
        policy: DeploymentPolicy {
            timeout_ms: 30_000,
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

fn package_test_config_literals(
    packages: &[PackageArtifact],
    production: &PackageArtifactRef,
    overlay: &PackageArtifactRef,
    owner: Option<&ServiceDeployment>,
    supplied: &[PackageTestConfigLiteral],
) -> Result<Vec<ConfigLiteralBinding>, CanonicalFixtureError> {
    let package_by_ref = packages
        .iter()
        .map(|package| {
            package_artifact_ref(package)
                .map(|reference| (reference, package))
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut supplied_by_key = BTreeMap::new();
    for literal in supplied {
        let effective_package = if &literal.package == production {
            overlay.clone()
        } else {
            literal.package.clone()
        };
        let exact_key = (effective_package.clone(), literal.key.clone());
        if supplied_by_key.insert(exact_key, literal).is_some() {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test config literal repeats exact package requirement {} {}",
                effective_package.package_build_id, literal.key
            )));
        }
        let package = package_by_ref.get(&effective_package).ok_or_else(|| {
            CanonicalFixtureError::InvalidInput(format!(
                "test config literal names package {} outside the exact deployment closure",
                literal.package.package_build_id
            ))
        })?;
        let requirement = package
            .runtime_requirements
            .config
            .iter()
            .find(|requirement| requirement.path == literal.key)
            .ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "test config literal names unknown requirement {} for package {}",
                    literal.key, effective_package.package_build_id
                ))
            })?;
        validate_test_literal_type(requirement.value_type.as_str(), &literal.value).map_err(
            |actual| {
                CanonicalFixtureError::InvalidInput(format!(
                    "test config literal {} for package {} must be {}, got {actual}",
                    literal.key, effective_package.package_build_id, requirement.value_type
                ))
            },
        )?;
    }

    let mut projected = owner
        .map(|deployment| deployment.config_literals.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|literal| (literal.path.clone(), literal))
        .collect::<BTreeMap<_, _>>();
    let inherited_secret_paths = owner
        .into_iter()
        .flat_map(|deployment| &deployment.secret_refs)
        .map(|binding| binding.path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for literal in supplied {
        if inherited_secret_paths.contains(literal.key.as_str()) {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test config literal {} conflicts with the exact base-assembly secret binding",
                literal.key
            )));
        }
        match projected.get(&literal.key) {
            Some(inherited) if inherited.value != literal.value => {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "test config literal {} conflicts with the exact base-assembly binding",
                    literal.key
                )));
            }
            Some(_) => {}
            None => {
                projected.insert(
                    literal.key.clone(),
                    ConfigLiteralBinding {
                        path: literal.key.clone(),
                        value: literal.value.clone(),
                    },
                );
            }
        }
    }
    for (package_ref, package) in &package_by_ref {
        for requirement in &package.runtime_requirements.config {
            if requirement.required
                && !projected.contains_key(&requirement.path)
                && !inherited_secret_paths.contains(requirement.path.as_str())
            {
                return Err(CanonicalFixtureError::InvalidInput(format!(
                    "required test config literal {} for package {} is missing",
                    requirement.path, package_ref.package_build_id
                )));
            }
        }
    }
    Ok(projected.into_values().collect())
}

fn validate_test_literal_type(value_type: &str, value: &MetadataValue) -> Result<(), &'static str> {
    let actual = match value {
        MetadataValue::String(_) => "string",
        MetadataValue::Number(_) => "number",
        MetadataValue::Bool(_) => "bool",
        MetadataValue::Array(_) => "json",
        MetadataValue::Object(_) => "jsonObject",
        MetadataValue::Null => "json",
    };
    let valid = match value_type {
        "string" => matches!(value, MetadataValue::String(_)),
        "number" => matches!(value, MetadataValue::Number(_)),
        "bool" => matches!(value, MetadataValue::Bool(_)),
        "json" => true,
        "jsonObject" => matches!(value, MetadataValue::Object(_)),
        _ => false,
    };
    valid.then_some(()).ok_or(actual)
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
