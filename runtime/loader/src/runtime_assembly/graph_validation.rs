use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    BoundaryCallableProjection, PackageBuildId, RuntimeAssembly, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef,
};

use super::{HydratedPackageCodeSlot, ServiceContractStore};

pub(super) fn validate_hydrated_graph(
    assembly: &RuntimeAssembly,
    deployments: &BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: &ServiceContractStore,
    code_slots: &[HydratedPackageCodeSlot],
) -> anyhow::Result<()> {
    let packages = code_slots
        .iter()
        .map(|slot| (slot.reference.package_build_id.clone(), slot))
        .collect::<BTreeMap<_, _>>();
    validate_package_links(assembly, &packages)?;
    validate_contract_closure(assembly, deployments, contracts)?;
    validate_deployment_contents(assembly, deployments, contracts, &packages)?;
    validate_service_templates(assembly, deployments, contracts, &packages)?;
    validate_activation_templates(assembly, deployments)?;
    validate_ingress(assembly, deployments)?;
    validate_reachable_closure(assembly, &packages)
}

fn validate_package_links(
    assembly: &RuntimeAssembly,
    packages: &BTreeMap<PackageBuildId, &HydratedPackageCodeSlot>,
) -> anyhow::Result<()> {
    let links = assembly
        .package_link_plan
        .package_links
        .iter()
        .map(|binding| (binding.key.clone(), &binding.package))
        .collect::<BTreeMap<_, _>>();
    let mut expected = BTreeSet::new();
    for (build_id, package) in packages {
        for requirement in &package.artifact.package_requirements {
            let key = skiff_artifact_model::PackageRequirementKey {
                caller_package_build_id: build_id.clone(),
                package_requirement_alias: requirement.alias.clone(),
            };
            expected.insert(key.clone());
            let provider = links.get(&key).ok_or_else(|| {
                anyhow::anyhow!(
                    "package {} requirement {} has no canonical link-plan target",
                    build_id,
                    requirement.alias
                )
            })?;
            if provider.package_id != requirement.package_id
                || provider.package_version != requirement.exact_version
                || provider.package_local_abi_identity != requirement.expected_local_abi
                || requirement
                    .expected_package_build
                    .as_ref()
                    .is_some_and(|expected| expected != &provider.package_build_id)
            {
                anyhow::bail!(
                    "package {} requirement {} link-plan target mismatches coordinate/local ABI",
                    build_id,
                    requirement.alias
                );
            }
        }
    }
    if links.keys().cloned().collect::<BTreeSet<_>>() != expected {
        anyhow::bail!("canonical package link plan contains a link without a package requirement");
    }
    Ok(())
}

fn validate_contract_closure(
    assembly: &RuntimeAssembly,
    deployments: &BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: &ServiceContractStore,
) -> anyhow::Result<()> {
    let mut expected = deployments
        .values()
        .map(|deployment| deployment.contract.clone())
        .collect::<BTreeSet<_>>();
    expected.extend(
        assembly
            .service_binding_templates
            .iter()
            .flat_map(|template| template.bindings.iter())
            .map(|binding| binding.contract.clone()),
    );
    let declared = assembly
        .resolved_contracts
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if declared != expected {
        anyhow::bail!("resolvedContracts does not exactly match the hydrated contract closure");
    }
    if contracts.contracts.keys().cloned().collect::<BTreeSet<_>>() != expected {
        anyhow::bail!("canonical contract store does not exactly match resolvedContracts");
    }
    Ok(())
}

fn validate_deployment_contents(
    assembly: &RuntimeAssembly,
    deployments: &BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: &ServiceContractStore,
    packages: &BTreeMap<PackageBuildId, &HydratedPackageCodeSlot>,
) -> anyhow::Result<()> {
    let global_package_links = assembly
        .package_link_plan
        .package_links
        .iter()
        .map(|binding| (binding.key.clone(), &binding.package))
        .collect::<BTreeMap<_, _>>();
    for (reference, deployment) in deployments {
        let contract = contracts.contract(&deployment.contract).ok_or_else(|| {
            anyhow::anyhow!("deployment {reference:?} targets a missing contract")
        })?;
        let implementation = packages
            .get(&deployment.implementation.package_build_id)
            .ok_or_else(|| anyhow::anyhow!("deployment {reference:?} targets a missing package"))?;
        if implementation.reference != deployment.implementation {
            anyhow::bail!("deployment {reference:?} implementation ref is not exact");
        }
        for binding in &deployment.package_bindings {
            let package = packages
                .get(&binding.package.package_build_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("deployment {reference:?} has a missing package binding")
                })?;
            if package.reference != binding.package {
                anyhow::bail!("deployment {reference:?} package binding ref is not exact");
            }
            if global_package_links.get(&binding.key) != Some(&&binding.package) {
                anyhow::bail!(
                    "deployment {reference:?} package binding {:?} mismatches canonical link plan",
                    binding.key
                );
            }
        }
        activation_package_closure(reference, deployment, packages)?;

        let bound_operations = deployment
            .operation_bindings
            .iter()
            .map(|binding| binding.contract_operation_id.clone())
            .collect::<BTreeSet<_>>();
        if bound_operations != contract.operations.keys().cloned().collect::<BTreeSet<_>>() {
            anyhow::bail!(
                "deployment {reference:?} operation bindings do not exactly match its contract"
            );
        }
        for binding in &deployment.operation_bindings {
            let descriptor = contract
                .operations
                .get(&binding.contract_operation_id)
                .expect("operation key set was checked");
            let projection = implementation
                .artifact
                .boundary_projections
                .get(&binding.package_callable_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "deployment {reference:?} operation {} targets missing callable {}",
                        binding.contract_operation_id,
                        binding.package_callable_id
                    )
                })?;
            match projection {
                BoundaryCallableProjection::Available {
                    operation_contract, ..
                } if operation_contract == &descriptor.contract => {}
                BoundaryCallableProjection::Available { .. } => anyhow::bail!(
                    "deployment {reference:?} operation {} descriptor mismatches provider callable",
                    binding.contract_operation_id
                ),
                BoundaryCallableProjection::Unavailable { .. } => anyhow::bail!(
                    "deployment {reference:?} operation {} targets boundary-unavailable callable",
                    binding.contract_operation_id
                ),
            }
        }
    }
    Ok(())
}

fn validate_service_templates(
    assembly: &RuntimeAssembly,
    deployments: &BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: &ServiceContractStore,
    packages: &BTreeMap<PackageBuildId, &HydratedPackageCodeSlot>,
) -> anyhow::Result<()> {
    for template in &assembly.service_binding_templates {
        let deployment = deployments
            .get(&template.activation)
            .expect("assembly surface validated activation ref");
        let declared = deployment
            .service_selectors
            .iter()
            .map(|binding| (binding.key.clone(), binding.contract.clone()))
            .collect::<BTreeMap<_, _>>();
        let resolved = template
            .bindings
            .iter()
            .map(|binding| (binding.key.clone(), binding.contract.clone()))
            .collect::<BTreeMap<_, _>>();
        if declared != resolved {
            anyhow::bail!(
                "service binding template {:?} does not exactly match deployment selectors",
                template.activation
            );
        }
        let activation_packages =
            activation_package_closure(&template.activation, deployment, packages)?;
        let mut required_bindings = BTreeMap::new();
        for build_id in activation_packages {
            let package = packages
                .get(&build_id)
                .expect("activation package closure returns hydrated builds");
            for requirement in &package.artifact.service_requirements {
                let key = skiff_artifact_model::ServiceRequirementKey {
                    caller_package_build_id: build_id.clone(),
                    service_requirement_slot: requirement.service_binding_slot,
                };
                let contract = ServiceContractRef {
                    service_id: requirement.contract_requirement.service_id.clone(),
                    contract_version: requirement.contract_requirement.contract_version.clone(),
                    service_protocol_identity: requirement
                        .contract_requirement
                        .expected_protocol_identity
                        .clone(),
                };
                required_bindings.insert(key, (contract, requirement.used_operations.clone()));
            }
        }
        let hydrated_bindings = template
            .bindings
            .iter()
            .map(|binding| {
                (
                    binding.key.clone(),
                    (
                        binding.contract.clone(),
                        binding
                            .used_operations
                            .iter()
                            .cloned()
                            .collect::<BTreeSet<_>>(),
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if hydrated_bindings != required_bindings {
            anyhow::bail!(
                "service binding template {:?} does not exactly match package service requirements",
                template.activation
            );
        }
        for binding in &template.bindings {
            let caller = packages
                .get(&binding.key.caller_package_build_id)
                .expect("assembly surface validated caller build");
            let requirement = caller
                .artifact
                .service_requirements
                .iter()
                .find(|requirement| {
                    requirement.service_binding_slot == binding.key.service_requirement_slot
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "service binding {:?} has no typed package requirement",
                        binding.key
                    )
                })?;
            let requirement_contract = ServiceContractRef {
                service_id: requirement.contract_requirement.service_id.clone(),
                contract_version: requirement.contract_requirement.contract_version.clone(),
                service_protocol_identity: requirement
                    .contract_requirement
                    .expected_protocol_identity
                    .clone(),
            };
            if requirement_contract != binding.contract {
                anyhow::bail!("service binding {:?} contract ref mismatch", binding.key);
            }
            if requirement.used_operations
                != binding
                    .used_operations
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
            {
                anyhow::bail!("service binding {:?} used operations mismatch", binding.key);
            }
            let provider = deployments
                .get(&binding.provider)
                .expect("assembly surface validated provider deployment");
            if provider.contract != binding.contract {
                anyhow::bail!(
                    "service binding {:?} provider contract mismatch",
                    binding.key
                );
            }
            for operation in &binding.used_operations {
                if contracts.operation(&binding.contract, operation).is_none() {
                    anyhow::bail!(
                        "service binding {:?} references missing contract operation {}",
                        binding.key,
                        operation
                    );
                }
            }
        }
    }
    Ok(())
}

fn activation_package_closure(
    activation: &ServiceDeploymentRef,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, &HydratedPackageCodeSlot>,
) -> anyhow::Result<BTreeSet<PackageBuildId>> {
    let bindings = deployment
        .package_bindings
        .iter()
        .map(|binding| (binding.key.clone(), &binding.package))
        .collect::<BTreeMap<_, _>>();
    let mut used_bindings = BTreeSet::new();
    let mut closure = BTreeSet::new();
    let mut pending = vec![deployment.implementation.package_build_id.clone()];
    while let Some(build_id) = pending.pop() {
        if !closure.insert(build_id.clone()) {
            continue;
        }
        let package = packages.get(&build_id).ok_or_else(|| {
            anyhow::anyhow!("activation {activation:?} targets missing package build {build_id}")
        })?;
        for requirement in &package.artifact.package_requirements {
            let key = skiff_artifact_model::PackageRequirementKey {
                caller_package_build_id: build_id.clone(),
                package_requirement_alias: requirement.alias.clone(),
            };
            let provider = bindings.get(&key).ok_or_else(|| {
                anyhow::anyhow!(
                    "activation {activation:?} package requirement {key:?} has no binding"
                )
            })?;
            if provider.package_id != requirement.package_id
                || provider.package_version != requirement.exact_version
                || provider.package_local_abi_identity != requirement.expected_local_abi
                || requirement
                    .expected_package_build
                    .as_ref()
                    .is_some_and(|expected| expected != &provider.package_build_id)
            {
                anyhow::bail!(
                    "activation {activation:?} package requirement {key:?} binding mismatches coordinate/local ABI"
                );
            }
            used_bindings.insert(key);
            pending.push(provider.package_build_id.clone());
        }
    }
    if used_bindings != bindings.keys().cloned().collect::<BTreeSet<_>>() {
        anyhow::bail!("activation {activation:?} contains an unused package binding");
    }
    Ok(closure)
}

fn validate_activation_templates(
    assembly: &RuntimeAssembly,
    deployments: &BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
) -> anyhow::Result<()> {
    for template in &assembly.activation_templates {
        let deployment = deployments
            .get(&template.deployment)
            .expect("assembly surface validated activation deployment");
        let config = template
            .config_literals
            .iter()
            .map(|binding| (binding.path.as_str(), &binding.value))
            .collect::<BTreeMap<_, _>>();
        let deployment_config = deployment
            .config_literals
            .iter()
            .map(|binding| (binding.path.as_str(), &binding.value))
            .collect::<BTreeMap<_, _>>();
        let secrets = template
            .secret_refs
            .iter()
            .map(|binding| (binding.path.as_str(), binding.secret_ref.as_str()))
            .collect::<BTreeMap<_, _>>();
        let deployment_secrets = deployment
            .secret_refs
            .iter()
            .map(|binding| (binding.path.as_str(), binding.secret_ref.as_str()))
            .collect::<BTreeMap<_, _>>();
        let states = template
            .state_bindings
            .iter()
            .map(|binding| (binding.requirement_key.as_str(), binding))
            .collect::<BTreeMap<_, _>>();
        let deployment_states = deployment
            .state_bindings
            .iter()
            .map(|binding| (binding.requirement_key.as_str(), binding))
            .collect::<BTreeMap<_, _>>();
        let resources = template
            .resource_bindings
            .iter()
            .map(|binding| (binding.requirement_key.as_str(), binding))
            .collect::<BTreeMap<_, _>>();
        let deployment_resources = deployment
            .resource_bindings
            .iter()
            .map(|binding| (binding.requirement_key.as_str(), binding))
            .collect::<BTreeMap<_, _>>();
        if template.implementation_package_build_id != deployment.implementation.package_build_id
            || config != deployment_config
            || secrets != deployment_secrets
            || states != deployment_states
            || resources != deployment_resources
            || template.policy != deployment.policy
        {
            anyhow::bail!(
                "activation template {:?} does not exactly match immutable deployment inputs",
                template.deployment
            );
        }
    }
    Ok(())
}

fn validate_ingress(
    assembly: &RuntimeAssembly,
    deployments: &BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
) -> anyhow::Result<()> {
    if !assembly.global_ingress.is_empty() {
        anyhow::bail!(
            "legacy RuntimeAssembly globalIngress is not accepted before deployment gateway entries are linked"
        );
    }
    if deployments
        .values()
        .any(|deployment| !deployment.gateway_entries.is_empty() || !deployment.ingress.is_empty())
    {
        anyhow::bail!("deployment gateway ingress is not yet linked into RuntimeAssembly");
    }
    Ok(())
}

fn validate_reachable_closure(
    assembly: &RuntimeAssembly,
    packages: &BTreeMap<PackageBuildId, &HydratedPackageCodeSlot>,
) -> anyhow::Result<()> {
    let service_templates = assembly
        .service_binding_templates
        .iter()
        .map(|template| (template.activation.clone(), template))
        .collect::<BTreeMap<_, _>>();
    let mut reachable_deployments = assembly.roots.iter().cloned().collect::<BTreeSet<_>>();
    let mut pending_deployments = assembly.roots.clone();
    while let Some(deployment) = pending_deployments.pop() {
        if let Some(template) = service_templates.get(&deployment) {
            for binding in &template.bindings {
                if reachable_deployments.insert(binding.provider.clone()) {
                    pending_deployments.push(binding.provider.clone());
                }
            }
        }
    }
    if reachable_deployments
        != assembly
            .resolved_deployments
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
    {
        anyhow::bail!("resolvedDeployments is not the exact root-reachable service closure");
    }

    let links = assembly
        .package_link_plan
        .package_links
        .iter()
        .map(|binding| (binding.key.clone(), &binding.package))
        .collect::<BTreeMap<_, _>>();
    let activation_packages = assembly
        .activation_templates
        .iter()
        .filter(|template| reachable_deployments.contains(&template.deployment))
        .map(|template| template.implementation_package_build_id.clone())
        .collect::<BTreeSet<_>>();
    let mut reachable_packages = activation_packages.clone();
    let mut pending_packages = activation_packages.into_iter().collect::<Vec<_>>();
    while let Some(build_id) = pending_packages.pop() {
        let package = packages
            .get(&build_id)
            .expect("assembly surface validated activation package");
        for requirement in &package.artifact.package_requirements {
            let key = skiff_artifact_model::PackageRequirementKey {
                caller_package_build_id: build_id.clone(),
                package_requirement_alias: requirement.alias.clone(),
            };
            let provider = links
                .get(&key)
                .expect("package link validation checked requirement");
            if reachable_packages.insert(provider.package_build_id.clone()) {
                pending_packages.push(provider.package_build_id.clone());
            }
        }
    }
    if reachable_packages != packages.keys().cloned().collect::<BTreeSet<_>>() {
        anyhow::bail!("resolvedPackages is not the exact activation-reachable package closure");
    }
    Ok(())
}
