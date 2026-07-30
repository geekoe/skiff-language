use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use skiff_artifact_model::{
    ActivationTemplate, ContractOperationId, PackageBuildId, PackageRequirementKey,
    RuntimeAssembly, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    ServiceRequirementKey,
};
use skiff_runtime_linked_program::{
    HydratedPackageCode, LoadedPublicationResource, PublicationResourceTable,
    SharedPackageLinkedImage,
};
use skiff_runtime_loader::{HydratedRuntimeAssembly, ServiceContractStore};

mod candidate;
mod gateway;

pub use candidate::{
    AssemblyLinkedCandidate, AssemblyServiceCallError, LinkedActivationTemplate,
    LinkedContractOperation, LinkedServiceBindingTemplate,
};
use gateway::link_gateway_ingress;
pub use gateway::{LinkedGatewayCallable, LinkedGatewayEntry};

/// Links the exact canonical package plan once and retains only typed immutable assembly facts.
///
/// The input can only be constructed by the typed runtime-assembly loader. Candidate fields are
/// assembled in locals and published atomically after every package, activation, binding,
/// operation, contract and ingress check succeeds.
pub fn link_runtime_assembly(
    hydrated: HydratedRuntimeAssembly,
) -> anyhow::Result<AssemblyLinkedCandidate> {
    let assembly = Arc::clone(hydrated.assembly());
    let contracts = Arc::clone(hydrated.contract_store());
    let packages = hydrated
        .code_slots()
        .iter()
        .map(hydrated_package_code)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let shared_image = Arc::new(
        SharedPackageLinkedImage::from_runtime_assembly(&assembly, packages)
            .context("failed to link the canonical package image")?,
    );
    if shared_image.assembly_identity() != &assembly.assembly_identity {
        anyhow::bail!("shared package image identity does not match RuntimeAssembly");
    }
    let execution_image =
        crate::assembly_execution::link_assembly_execution_image(Arc::clone(&shared_image))
            .context("failed to link the canonical assembly execution image")?;

    validate_contract_store(&assembly, &contracts)?;
    let activations = link_activation_templates(&hydrated, &shared_image, &contracts)?;
    let (gateway_entries, ingress) = link_gateway_ingress(&hydrated, &activations, &shared_image)?;

    Ok(AssemblyLinkedCandidate {
        assembly,
        shared_image,
        execution_image,
        contracts,
        activations,
        gateway_entries,
        ingress,
    })
}

fn hydrated_package_code(
    slot: &skiff_runtime_loader::HydratedPackageCodeSlot,
) -> anyhow::Result<HydratedPackageCode> {
    let mut resources = PublicationResourceTable::default();
    for resource in slot.resources() {
        let reference = resource.reference();
        if resources
            .insert(
                reference.path.clone(),
                LoadedPublicationResource {
                    meta: reference.clone(),
                    bytes: Arc::clone(resource.bytes()),
                },
            )
            .is_some()
        {
            anyhow::bail!(
                "package {} repeats hydrated resource {}",
                slot.reference().package_build_id,
                reference.path
            );
        }
    }
    Ok(HydratedPackageCode::new(
        Arc::clone(slot.artifact()),
        slot.files().to_vec(),
        resources,
    )
    .with_schema_index(Arc::clone(slot.schema_index()))
    .with_schema_records(slot.schema_records().clone()))
}

fn validate_contract_store(
    assembly: &RuntimeAssembly,
    contracts: &ServiceContractStore,
) -> anyhow::Result<()> {
    let declared = assembly
        .resolved_contracts
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let hydrated = contracts
        .contracts()
        .map(|(reference, _)| reference.clone())
        .collect::<BTreeSet<_>>();
    if declared.len() != assembly.resolved_contracts.len() || declared != hydrated {
        anyhow::bail!("canonical contract store does not exactly match resolvedContracts");
    }
    Ok(())
}

fn link_activation_templates(
    hydrated: &HydratedRuntimeAssembly,
    image: &SharedPackageLinkedImage,
    contracts: &ServiceContractStore,
) -> anyhow::Result<BTreeMap<ServiceDeploymentRef, LinkedActivationTemplate>> {
    let assembly = hydrated.assembly();
    let mut service_templates = BTreeMap::new();
    for template in &assembly.service_binding_templates {
        if service_templates
            .insert(template.activation.clone(), template)
            .is_some()
        {
            anyhow::bail!(
                "duplicate service binding template for {:?}",
                template.activation
            );
        }
    }

    let mut linked = BTreeMap::new();
    for source in &assembly.activation_templates {
        let deployment = hydrated
            .deployment(&source.deployment)
            .ok_or_else(|| anyhow::anyhow!("activation {:?} is dangling", source.deployment))?;
        validate_activation_source(source, deployment)?;

        let implementation = image
            .code_by_build(&source.implementation_package_build_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "activation {:?} implementation package {} is not linked",
                    source.deployment,
                    source.implementation_package_build_id
                )
            })?;
        if implementation.artifact_ref() != &deployment.implementation {
            anyhow::bail!(
                "activation {:?} implementation package ref is not exact",
                source.deployment
            );
        }
        let operations = link_contract_operations(deployment, implementation, contracts)?;
        let package_closure = activation_package_closure(deployment, image)?;
        let service_template = service_templates
            .remove(&source.deployment)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "activation {:?} has no service binding template",
                    source.deployment
                )
            })?;
        let service_bindings = link_service_bindings(
            hydrated,
            service_template,
            &package_closure,
            image,
            contracts,
        )?;
        let candidate = LinkedActivationTemplate {
            source: source.clone(),
            deployment: Arc::clone(deployment),
            implementation_code_slot: implementation.code_slot(),
            operations,
            service_bindings,
        };
        if linked
            .insert(source.deployment.clone(), candidate)
            .is_some()
        {
            anyhow::bail!("duplicate activation template for {:?}", source.deployment);
        }
    }

    if let Some((activation, _)) = service_templates.into_iter().next() {
        anyhow::bail!("service binding template {activation:?} has no activation template");
    }
    let deployments = hydrated
        .deployments()
        .map(|(reference, _)| reference.clone())
        .collect::<BTreeSet<_>>();
    if linked.keys().cloned().collect::<BTreeSet<_>>() != deployments {
        anyhow::bail!("linked activation templates do not exactly match hydrated deployments");
    }
    for (activation, template) in &linked {
        for binding in template.service_bindings.values() {
            if !linked.contains_key(&binding.provider) {
                anyhow::bail!(
                    "activation {activation:?} service binding {:?} targets an unlinked provider",
                    binding.key
                );
            }
        }
    }
    Ok(linked)
}

fn validate_activation_source(
    source: &ActivationTemplate,
    deployment: &ServiceDeployment,
) -> anyhow::Result<()> {
    let expected = ActivationTemplate {
        deployment: source.deployment.clone(),
        implementation_package_build_id: deployment.implementation.package_build_id.clone(),
        config_literals: deployment.config_literals.clone(),
        secret_refs: deployment.secret_refs.clone(),
        state_bindings: deployment.state_bindings.clone(),
        resource_bindings: deployment.resource_bindings.clone(),
        policy: deployment.policy.clone(),
    };
    if source != &expected {
        anyhow::bail!(
            "activation template {:?} does not exactly match its deployment",
            source.deployment
        );
    }

    let mut paths = BTreeSet::new();
    for path in source
        .config_literals
        .iter()
        .map(|binding| binding.path.as_str())
        .chain(
            source
                .secret_refs
                .iter()
                .map(|binding| binding.path.as_str()),
        )
    {
        if path.is_empty() || !paths.insert(path) {
            anyhow::bail!(
                "activation template {:?} has a duplicate or empty config/secret path",
                source.deployment
            );
        }
    }
    validate_unique_requirement_keys(
        &source.deployment,
        "state",
        source
            .state_bindings
            .iter()
            .map(|binding| binding.requirement_key.as_str()),
    )?;
    validate_unique_requirement_keys(
        &source.deployment,
        "resource",
        source
            .resource_bindings
            .iter()
            .map(|binding| binding.requirement_key.as_str()),
    )
}

fn validate_unique_requirement_keys<'a>(
    activation: &ServiceDeploymentRef,
    kind: &str,
    keys: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<()> {
    let mut unique = BTreeSet::new();
    for key in keys {
        if key.is_empty() || !unique.insert(key) {
            anyhow::bail!(
                "activation template {activation:?} has a duplicate or empty {kind} requirement key"
            );
        }
    }
    Ok(())
}

fn link_contract_operations(
    deployment: &ServiceDeployment,
    implementation: &skiff_runtime_linked_program::SharedPackageCode,
    contracts: &ServiceContractStore,
) -> anyhow::Result<BTreeMap<ContractOperationId, LinkedContractOperation>> {
    let contract = contracts.contract(&deployment.contract).ok_or_else(|| {
        anyhow::anyhow!(
            "deployment contract {:?} is not hydrated",
            deployment.contract
        )
    })?;
    let mut operations = BTreeMap::new();
    for binding in &deployment.operation_bindings {
        let descriptor = contracts
            .operation_descriptor(&deployment.contract, &binding.contract_operation_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "deployment operation {} is absent from contract {:?}",
                    binding.contract_operation_id,
                    deployment.contract
                )
            })?;
        if descriptor.operation_id != binding.contract_operation_id {
            anyhow::bail!(
                "contract operation map key {} mismatches canonical descriptor",
                binding.contract_operation_id
            );
        }
        let target = implementation
            .callable_target(&binding.package_callable_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "deployment operation {} targets missing callable {} in package {}",
                    binding.contract_operation_id,
                    binding.package_callable_id,
                    implementation.package_build_id()
                )
            })?;
        let linked = LinkedContractOperation {
            contract_operation_id: binding.contract_operation_id.clone(),
            package_callable_id: binding.package_callable_id.clone(),
            target: target.clone(),
        };
        if operations
            .insert(binding.contract_operation_id.clone(), linked)
            .is_some()
        {
            anyhow::bail!(
                "deployment repeats operation binding {}",
                binding.contract_operation_id
            );
        }
    }
    if operations.keys().collect::<BTreeSet<_>>() != contract.operations.keys().collect() {
        anyhow::bail!("deployment operation bindings do not exactly match its contract");
    }
    Ok(operations)
}

fn activation_package_closure(
    deployment: &ServiceDeployment,
    image: &SharedPackageLinkedImage,
) -> anyhow::Result<BTreeSet<PackageBuildId>> {
    let mut bindings = BTreeMap::new();
    for binding in &deployment.package_bindings {
        if bindings.insert(binding.key.clone(), binding).is_some() {
            anyhow::bail!("deployment repeats package binding {:?}", binding.key);
        }
    }
    let global_links = image
        .package_link_plan()
        .package_links
        .iter()
        .map(|binding| (binding.key.clone(), binding))
        .collect::<BTreeMap<_, _>>();
    let mut used = BTreeSet::new();
    let mut closure = BTreeSet::new();
    let mut pending = vec![deployment.implementation.package_build_id.clone()];
    while let Some(build_id) = pending.pop() {
        if !closure.insert(build_id.clone()) {
            continue;
        }
        let code = image.code_by_build(&build_id).ok_or_else(|| {
            anyhow::anyhow!("activation package closure targets missing build {build_id}")
        })?;
        for requirement in &code.artifact().package_requirements {
            let key = PackageRequirementKey {
                caller_package_build_id: build_id.clone(),
                package_requirement_alias: requirement.alias.clone(),
            };
            let provider = bindings.get(&key).ok_or_else(|| {
                anyhow::anyhow!("activation package requirement {key:?} has no binding")
            })?;
            if global_links.get(&key) != Some(provider)
                || provider.package.package_id != requirement.package_id
                || provider.package.package_version != requirement.exact_version
                || provider.package.package_local_abi_identity != requirement.expected_local_abi
                || provider.collection_name_mapping != requirement.collection_name_mapping
                || requirement
                    .expected_package_build
                    .as_ref()
                    .is_some_and(|expected| expected != &provider.package.package_build_id)
            {
                anyhow::bail!(
                    "activation package requirement {key:?} does not match the canonical exact link"
                );
            }
            let provider_code = image
                .code_by_build(&provider.package.package_build_id)
                .ok_or_else(|| anyhow::anyhow!("package binding {key:?} provider is not linked"))?;
            if provider_code.artifact_ref() != &provider.package {
                anyhow::bail!("package binding {key:?} provider ref is not exact");
            }
            used.insert(key);
            pending.push(provider.package.package_build_id.clone());
        }
    }
    if used != bindings.keys().cloned().collect::<BTreeSet<_>>() {
        anyhow::bail!("activation contains a package binding outside its reachable closure");
    }
    Ok(closure)
}

fn link_service_bindings(
    hydrated: &HydratedRuntimeAssembly,
    template: &skiff_artifact_model::ServiceBindingTemplate,
    package_closure: &BTreeSet<PackageBuildId>,
    image: &SharedPackageLinkedImage,
    contracts: &ServiceContractStore,
) -> anyhow::Result<BTreeMap<ServiceRequirementKey, LinkedServiceBindingTemplate>> {
    let mut expected = BTreeMap::new();
    for build_id in package_closure {
        let code = image
            .code_by_build(build_id)
            .expect("activation closure contains only linked package builds");
        for requirement in &code.artifact().service_requirements {
            let key = ServiceRequirementKey {
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
            if expected
                .insert(key.clone(), (contract, requirement.used_operations.clone()))
                .is_some()
            {
                anyhow::bail!("activation repeats package service requirement {key:?}");
            }
        }
    }

    let mut linked = BTreeMap::new();
    for binding in &template.bindings {
        let (expected_contract, expected_operations) =
            expected.remove(&binding.key).ok_or_else(|| {
                anyhow::anyhow!(
                    "service binding {:?} does not identify an activation package requirement",
                    binding.key
                )
            })?;
        let used_operations = binding
            .used_operations
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if used_operations.len() != binding.used_operations.len()
            || binding.contract != expected_contract
            || used_operations != expected_operations
        {
            anyhow::bail!(
                "service binding {:?} mismatches its caller-relative requirement",
                binding.key
            );
        }
        let provider = hydrated.deployment(&binding.provider).ok_or_else(|| {
            anyhow::anyhow!("service binding {:?} provider is dangling", binding.key)
        })?;
        if provider.contract != binding.contract {
            anyhow::bail!(
                "service binding {:?} provider protocol mismatches",
                binding.key
            );
        }
        for operation in &used_operations {
            if contracts
                .operation_descriptor(&binding.contract, operation)
                .is_none()
            {
                anyhow::bail!(
                    "service binding {:?} targets missing contract operation {}",
                    binding.key,
                    operation
                );
            }
        }
        let linked_binding = LinkedServiceBindingTemplate {
            key: binding.key.clone(),
            contract: binding.contract.clone(),
            provider: binding.provider.clone(),
            used_operations,
        };
        if linked.insert(binding.key.clone(), linked_binding).is_some() {
            anyhow::bail!("duplicate service binding key {:?}", binding.key);
        }
    }
    if let Some((key, _)) = expected.into_iter().next() {
        anyhow::bail!("activation is missing service binding {key:?}");
    }
    Ok(linked)
}

#[cfg(test)]
mod tests;
