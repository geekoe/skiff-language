use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ActivationTemplate, GatewayEntryIdentity, GatewayEntryKey, GatewayIngressBinding,
    PackageArtifactRef, RuntimeAssembly, ServiceBindingTemplate, ServiceContractRef,
    ServiceDeploymentRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};

use crate::{
    deployment::{validate_contract_ref, validate_deployment_ref_shape, validate_package_ref},
    Result,
};

use super::invalid_assembly;

pub fn validate_runtime_assembly_surface(assembly: &RuntimeAssembly) -> Result<()> {
    if assembly.schema_version != RUNTIME_ASSEMBLY_SCHEMA_VERSION {
        return invalid_assembly(format!(
            "schemaVersion must be {RUNTIME_ASSEMBLY_SCHEMA_VERSION}, got {}",
            assembly.schema_version
        ));
    }

    let deployments = validate_deployments(&assembly.resolved_deployments)?;
    let contracts = validate_contracts(&assembly.resolved_contracts)?;
    let packages = validate_packages(&assembly.resolved_packages)?;
    validate_roots(&assembly.roots, &deployments)?;
    validate_link_plan(assembly, &packages)?;
    validate_service_templates(assembly, &deployments, &contracts, &packages)?;
    validate_activation_templates(assembly, &deployments, &packages)?;
    validate_gateway_ingress(&assembly.gateway_ingress, &deployments)?;
    validate_empty_closure(assembly)
}

fn validate_deployments(
    references: &[ServiceDeploymentRef],
) -> Result<BTreeSet<ServiceDeploymentRef>> {
    let mut exact = BTreeSet::new();
    let mut coordinates = BTreeMap::new();
    for reference in references {
        validate_deployment_ref_shape(reference, "resolved deployment")?;
        if !exact.insert(reference.clone()) {
            return invalid_assembly(format!("duplicate resolved deployment {reference:?}"));
        }
        let coordinate = (
            reference.service_id.as_str(),
            reference.contract_version.as_str(),
            reference.deployment_revision.as_str(),
        );
        if let Some(existing) = coordinates.insert(coordinate, reference) {
            if existing != reference {
                return invalid_assembly(format!(
                    "deployment coordinate {coordinate:?} resolves to multiple identities"
                ));
            }
        }
    }
    Ok(exact)
}

fn validate_contracts(references: &[ServiceContractRef]) -> Result<BTreeSet<ServiceContractRef>> {
    let mut exact = BTreeSet::new();
    let mut coordinates = BTreeMap::new();
    for reference in references {
        validate_contract_ref(reference, "resolved contract")?;
        if !exact.insert(reference.clone()) {
            return invalid_assembly(format!("duplicate resolved contract {reference:?}"));
        }
        let coordinate = (
            reference.service_id.as_str(),
            reference.contract_version.as_str(),
        );
        if let Some(existing) = coordinates.insert(coordinate, reference) {
            if existing != reference {
                return invalid_assembly(format!(
                    "contract coordinate {coordinate:?} resolves to multiple protocol identities"
                ));
            }
        }
    }
    Ok(exact)
}

fn validate_packages(references: &[PackageArtifactRef]) -> Result<BTreeSet<PackageArtifactRef>> {
    let mut exact = BTreeSet::new();
    let mut builds = BTreeMap::new();
    for reference in references {
        validate_package_ref(reference, "resolved package")?;
        if !exact.insert(reference.clone()) {
            return invalid_assembly(format!("duplicate resolved package {reference:?}"));
        }
        if let Some(existing) = builds.insert(reference.package_build_id.clone(), reference) {
            if existing != reference {
                return invalid_assembly(format!(
                    "package build {} has mismatched coordinate or local ABI",
                    reference.package_build_id
                ));
            }
        }
    }
    Ok(exact)
}

fn validate_roots(
    roots: &[ServiceDeploymentRef],
    deployments: &BTreeSet<ServiceDeploymentRef>,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for root in roots {
        validate_deployment_ref_shape(root, "root deployment")?;
        if !seen.insert(root) {
            return invalid_assembly(format!("duplicate root deployment {root:?}"));
        }
        if !deployments.contains(root) {
            return invalid_assembly(format!("root deployment {root:?} is dangling"));
        }
    }
    Ok(())
}

fn validate_link_plan(
    assembly: &RuntimeAssembly,
    packages: &BTreeSet<PackageArtifactRef>,
) -> Result<()> {
    let mut code_slots = BTreeSet::new();
    for slot in &assembly.package_link_plan.code_slots {
        validate_package_ref(&slot.package, "package code slot")?;
        if !packages.contains(&slot.package) {
            return invalid_assembly(format!("code slot {:?} is dangling", slot.package));
        }
        if !code_slots.insert(slot.package.clone()) {
            return invalid_assembly(format!("duplicate package code slot {:?}", slot.package));
        }
    }
    if &code_slots != packages {
        return invalid_assembly("package code slots must exactly match resolvedPackages");
    }

    let known_builds = packages
        .iter()
        .map(|package| package.package_build_id.clone())
        .collect::<BTreeSet<_>>();
    let mut link_keys = BTreeSet::new();
    for link in &assembly.package_link_plan.package_links {
        skiff_artifact_model::validate_dependency_collection_name_mapping(
            &link.collection_name_mapping,
        )
        .map_err(|message| crate::ArtifactIdentityError::InvalidRuntimeAssembly { message })?;
        if !known_builds.contains(&link.key.caller_package_build_id) {
            return invalid_assembly(format!(
                "package link caller {} is dangling",
                link.key.caller_package_build_id
            ));
        }
        if link.key.package_requirement_alias.trim().is_empty() {
            return invalid_assembly("package link requirement alias must not be empty");
        }
        if !packages.contains(&link.package) {
            return invalid_assembly(format!(
                "package link provider {:?} is dangling",
                link.package
            ));
        }
        if !link_keys.insert(link.key.clone()) {
            return invalid_assembly(format!("duplicate package link key {:?}", link.key));
        }
    }
    Ok(())
}

fn validate_service_templates(
    assembly: &RuntimeAssembly,
    deployments: &BTreeSet<ServiceDeploymentRef>,
    contracts: &BTreeSet<ServiceContractRef>,
    packages: &BTreeSet<PackageArtifactRef>,
) -> Result<()> {
    let known_builds = packages
        .iter()
        .map(|package| package.package_build_id.clone())
        .collect::<BTreeSet<_>>();
    let mut activations = BTreeSet::new();
    for template in &assembly.service_binding_templates {
        validate_service_template(template, deployments, contracts, &known_builds)?;
        if !activations.insert(template.activation.clone()) {
            return invalid_assembly(format!(
                "duplicate service binding template for {:?}",
                template.activation
            ));
        }
    }
    if &activations != deployments {
        return invalid_assembly(
            "serviceBindingTemplates must contain exactly one entry per resolved deployment",
        );
    }
    Ok(())
}

fn validate_service_template(
    template: &ServiceBindingTemplate,
    deployments: &BTreeSet<ServiceDeploymentRef>,
    contracts: &BTreeSet<ServiceContractRef>,
    known_builds: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<()> {
    if !deployments.contains(&template.activation) {
        return invalid_assembly(format!(
            "service binding activation {:?} is dangling",
            template.activation
        ));
    }
    let mut keys = BTreeSet::new();
    for binding in &template.bindings {
        if !known_builds.contains(&binding.key.caller_package_build_id) {
            return invalid_assembly(format!(
                "service binding caller {} is dangling",
                binding.key.caller_package_build_id
            ));
        }
        if !keys.insert(binding.key.clone()) {
            return invalid_assembly(format!("duplicate service binding key {:?}", binding.key));
        }
        if !contracts.contains(&binding.contract) {
            return invalid_assembly(format!(
                "service binding contract {:?} is dangling",
                binding.contract
            ));
        }
        if !deployments.contains(&binding.provider) {
            return invalid_assembly(format!(
                "service binding provider {:?} is dangling",
                binding.provider
            ));
        }
        if binding.provider.service_id != binding.contract.service_id
            || binding.provider.contract_version != binding.contract.contract_version
        {
            return invalid_assembly(
                "service binding provider coordinate mismatches contract selector",
            );
        }
        let mut operations = BTreeSet::new();
        for operation in &binding.used_operations {
            if operation.as_str().trim().is_empty() {
                return invalid_assembly("service binding operation id must not be empty");
            }
            if !operations.insert(operation) {
                return invalid_assembly(format!("duplicate used operation {operation}"));
            }
        }
        if operations.is_empty() {
            return invalid_assembly("resolved service binding must use at least one operation");
        }
    }
    Ok(())
}

fn validate_activation_templates(
    assembly: &RuntimeAssembly,
    deployments: &BTreeSet<ServiceDeploymentRef>,
    packages: &BTreeSet<PackageArtifactRef>,
) -> Result<()> {
    let known_builds = packages
        .iter()
        .map(|package| package.package_build_id.clone())
        .collect::<BTreeSet<_>>();
    let mut activations = BTreeSet::new();
    for template in &assembly.activation_templates {
        validate_activation_template(template, deployments, &known_builds)?;
        if !activations.insert(template.deployment.clone()) {
            return invalid_assembly(format!(
                "duplicate activation template for {:?}",
                template.deployment
            ));
        }
    }
    if &activations != deployments {
        return invalid_assembly(
            "activationTemplates must contain exactly one entry per resolved deployment",
        );
    }
    Ok(())
}

fn validate_activation_template(
    template: &ActivationTemplate,
    deployments: &BTreeSet<ServiceDeploymentRef>,
    known_builds: &BTreeSet<skiff_artifact_model::PackageBuildId>,
) -> Result<()> {
    if !deployments.contains(&template.deployment) {
        return invalid_assembly(format!("activation {:?} is dangling", template.deployment));
    }
    if !known_builds.contains(&template.implementation_package_build_id) {
        return invalid_assembly(format!(
            "activation implementation package {} is dangling",
            template.implementation_package_build_id
        ));
    }
    validate_unique_paths(
        template
            .config_literals
            .iter()
            .map(|binding| binding.path.as_str()),
        template
            .secret_refs
            .iter()
            .map(|binding| binding.path.as_str()),
    )?;
    validate_unique_names(
        template
            .state_bindings
            .iter()
            .map(|binding| binding.requirement_key.as_str()),
        "activation state binding",
    )?;
    validate_unique_names(
        template
            .resource_bindings
            .iter()
            .map(|binding| binding.requirement_key.as_str()),
        "activation resource binding",
    )
}

fn validate_gateway_ingress(
    bindings: &[GatewayIngressBinding],
    deployments: &BTreeSet<ServiceDeploymentRef>,
) -> Result<()> {
    let mut selectors = BTreeSet::new();
    for binding in bindings {
        if !selectors.insert(binding.selector.clone()) {
            return invalid_assembly(format!(
                "gateway ingress collision for {:?}",
                binding.selector
            ));
        }
        if !deployments.contains(&binding.deployment) {
            return invalid_assembly(format!(
                "gateway ingress deployment {:?} is dangling",
                binding.deployment
            ));
        }
        GatewayEntryKey::parse(binding.gateway_entry_key.as_str()).map_err(|error| {
            crate::ArtifactIdentityError::InvalidRuntimeAssembly {
                message: format!("gateway ingress key is invalid: {error}"),
            }
        })?;
        GatewayEntryIdentity::parse(binding.gateway_entry_identity.as_str()).map_err(|error| {
            crate::ArtifactIdentityError::InvalidRuntimeAssembly {
                message: format!("gateway ingress identity is invalid: {error}"),
            }
        })?;
    }
    Ok(())
}

fn validate_empty_closure(assembly: &RuntimeAssembly) -> Result<()> {
    if !assembly.roots.is_empty() {
        return Ok(());
    }
    if assembly.resolved_deployments.is_empty()
        && assembly.resolved_contracts.is_empty()
        && assembly.resolved_packages.is_empty()
        && assembly.package_link_plan.code_slots.is_empty()
        && assembly.package_link_plan.package_links.is_empty()
        && assembly.service_binding_templates.is_empty()
        && assembly.activation_templates.is_empty()
        && assembly.gateway_ingress.is_empty()
    {
        Ok(())
    } else {
        invalid_assembly("an assembly with no roots must have an empty resolved closure")
    }
}

fn validate_unique_paths<'a>(
    config: impl IntoIterator<Item = &'a str>,
    secrets: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let mut paths = BTreeSet::new();
    for path in config.into_iter().chain(secrets) {
        if path.trim().is_empty() {
            return invalid_assembly("activation config/secret path must not be empty");
        }
        if !paths.insert(path) {
            return invalid_assembly(format!("duplicate activation config/secret path {path}"));
        }
    }
    Ok(())
}

fn validate_unique_names<'a>(names: impl IntoIterator<Item = &'a str>, label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        if name.trim().is_empty() {
            return invalid_assembly(format!("{label} key must not be empty"));
        }
        if !seen.insert(name) {
            return invalid_assembly(format!("duplicate {label} {name}"));
        }
    }
    Ok(())
}
