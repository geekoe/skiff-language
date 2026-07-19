use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ConfigLiteralBinding, DeploymentIngressBinding, DeploymentPolicy, DeploymentRevision,
    PackageArtifactRef, PackageBinding, ResourceBinding, RuntimeCapabilityBinding,
    SecretRefBinding, ServiceContractRef, ServiceDeployment, ServiceDeploymentInput,
    ServiceDeploymentRef, ServiceSelectorBinding, StateBinding,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

use crate::{ArtifactIdentityError, Result};

/// Validate the path-free typed input before projection resolves public paths.
pub fn validate_service_deployment_input(input: &ServiceDeploymentInput) -> Result<()> {
    if input.schema_version != SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION {
        return invalid_deployment(format!(
            "schemaVersion must be {SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION}, got {}",
            input.schema_version
        ));
    }
    validate_contract_ref(&input.contract, "contract")?;
    validate_revision(&input.deployment_revision)?;
    validate_package_ref(&input.implementation, "implementation")?;
    let mut operations = BTreeSet::new();
    for binding in &input.operation_bindings {
        require_non_empty(
            "operation contractOperationId",
            binding.contract_operation_id.as_str(),
        )?;
        require_non_empty("operation packagePublicPath", &binding.package_public_path)?;
        if !operations.insert(binding.contract_operation_id.clone()) {
            return invalid_deployment(format!(
                "duplicate operation binding {}",
                binding.contract_operation_id
            ));
        }
    }
    if operations.is_empty() {
        return invalid_deployment("operationBindings must not be empty");
    }
    validate_shared_bindings(
        &input.implementation,
        &input.package_bindings,
        &input.service_selectors,
        &input.ingress,
        &input.config_literals,
        &input.secret_refs,
        &input.state_bindings,
        &input.resource_bindings,
        &input.runtime_capability_bindings,
        &input.policy,
        &operations,
    )
}

/// Validate the canonical deployment shape without trusting its declared identity.
pub fn validate_service_deployment_surface(deployment: &ServiceDeployment) -> Result<()> {
    if deployment.schema_version != SERVICE_DEPLOYMENT_SCHEMA_VERSION {
        return invalid_deployment(format!(
            "schemaVersion must be {SERVICE_DEPLOYMENT_SCHEMA_VERSION}, got {}",
            deployment.schema_version
        ));
    }
    validate_contract_ref(&deployment.contract, "contract")?;
    validate_revision(&deployment.deployment_revision)?;
    validate_package_ref(&deployment.implementation, "implementation")?;
    let mut operations = BTreeSet::new();
    for binding in &deployment.operation_bindings {
        require_non_empty(
            "operation contractOperationId",
            binding.contract_operation_id.as_str(),
        )?;
        require_non_empty(
            "operation packageCallableId",
            binding.package_callable_id.as_str(),
        )?;
        if !operations.insert(binding.contract_operation_id.clone()) {
            return invalid_deployment(format!(
                "duplicate operation binding {}",
                binding.contract_operation_id
            ));
        }
    }
    if operations.is_empty() {
        return invalid_deployment("operationBindings must not be empty");
    }
    validate_shared_bindings(
        &deployment.implementation,
        &deployment.package_bindings,
        &deployment.service_selectors,
        &deployment.ingress,
        &deployment.config_literals,
        &deployment.secret_refs,
        &deployment.state_bindings,
        &deployment.resource_bindings,
        &deployment.runtime_capability_bindings,
        &deployment.policy,
        &operations,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_shared_bindings(
    implementation: &PackageArtifactRef,
    package_bindings: &[PackageBinding],
    service_selectors: &[ServiceSelectorBinding],
    ingress: &[DeploymentIngressBinding],
    config_literals: &[ConfigLiteralBinding],
    secret_refs: &[SecretRefBinding],
    state_bindings: &[StateBinding],
    resource_bindings: &[ResourceBinding],
    runtime_capability_bindings: &[RuntimeCapabilityBinding],
    policy: &DeploymentPolicy,
    operation_ids: &BTreeSet<skiff_artifact_model::ContractOperationId>,
) -> Result<()> {
    let packages = validate_package_bindings(implementation, package_bindings)?;
    validate_service_selectors(service_selectors, &packages)?;
    validate_ingress_bindings(ingress, operation_ids)?;
    validate_activation_inputs(
        config_literals,
        secret_refs,
        state_bindings,
        resource_bindings,
        runtime_capability_bindings,
    )?;
    validate_policy(policy)
}

fn validate_package_bindings<'a>(
    implementation: &'a PackageArtifactRef,
    bindings: &'a [PackageBinding],
) -> Result<BTreeMap<skiff_artifact_model::PackageBuildId, &'a PackageArtifactRef>> {
    let mut packages = BTreeMap::new();
    insert_package_coordinate(&mut packages, implementation, "implementation")?;
    for binding in bindings {
        validate_package_ref(&binding.package, "package binding provider")?;
        insert_package_coordinate(&mut packages, &binding.package, "package binding provider")?;
    }
    let mut keys = BTreeSet::new();
    for binding in bindings {
        require_non_empty(
            "packageRequirementAlias",
            &binding.key.package_requirement_alias,
        )?;
        if !packages.contains_key(&binding.key.caller_package_build_id) {
            return invalid_deployment(format!(
                "package binding caller {} is not in the deployment package closure",
                binding.key.caller_package_build_id
            ));
        }
        if !keys.insert(binding.key.clone()) {
            return invalid_deployment(format!(
                "duplicate package binding key {}:{}",
                binding.key.caller_package_build_id, binding.key.package_requirement_alias
            ));
        }
    }
    Ok(packages)
}

fn validate_service_selectors(
    selectors: &[ServiceSelectorBinding],
    packages: &BTreeMap<skiff_artifact_model::PackageBuildId, &PackageArtifactRef>,
) -> Result<()> {
    let mut keys = BTreeSet::new();
    for selector in selectors {
        validate_contract_ref(&selector.contract, "service selector contract")?;
        if !packages.contains_key(&selector.key.caller_package_build_id) {
            return invalid_deployment(format!(
                "service selector caller {} is not in the deployment package closure",
                selector.key.caller_package_build_id
            ));
        }
        if !keys.insert(selector.key.clone()) {
            return invalid_deployment(format!(
                "duplicate service selector key {}:{}",
                selector.key.caller_package_build_id, selector.key.service_requirement_slot
            ));
        }
    }
    Ok(())
}

fn validate_ingress_bindings(
    ingress: &[DeploymentIngressBinding],
    operation_ids: &BTreeSet<skiff_artifact_model::ContractOperationId>,
) -> Result<()> {
    let mut selectors = BTreeSet::new();
    for binding in ingress {
        validate_ingress(binding)?;
        if !operation_ids.contains(&binding.contract_operation_id) {
            return invalid_deployment(format!(
                "ingress selector references unbound operation {}",
                binding.contract_operation_id
            ));
        }
        if !selectors.insert(binding.selector.clone()) {
            return invalid_deployment(format!(
                "duplicate ingress selector {:?}",
                binding.selector
            ));
        }
    }
    Ok(())
}

fn validate_activation_inputs(
    config_literals: &[ConfigLiteralBinding],
    secret_refs: &[SecretRefBinding],
    state_bindings: &[StateBinding],
    resource_bindings: &[ResourceBinding],
    runtime_capability_bindings: &[RuntimeCapabilityBinding],
) -> Result<()> {
    let mut config_paths = BTreeSet::new();
    for binding in config_literals {
        require_non_empty("config literal path", &binding.path)?;
        if !config_paths.insert(binding.path.as_str()) {
            return invalid_deployment(format!("duplicate config literal path {}", binding.path));
        }
    }
    for binding in secret_refs {
        require_non_empty("secret path", &binding.path)?;
        require_non_empty("secretRef", &binding.secret_ref)?;
        if !config_paths.insert(binding.path.as_str()) {
            return invalid_deployment(format!(
                "config/secret path {} has more than one binding",
                binding.path
            ));
        }
    }
    validate_unique_named(
        state_bindings
            .iter()
            .map(|binding| binding.requirement_key.as_str()),
        "state requirement",
    )?;
    for binding in state_bindings {
        require_non_empty("state namespace", &binding.namespace)?;
    }
    validate_unique_named(
        resource_bindings
            .iter()
            .map(|binding| binding.requirement_key.as_str()),
        "resource requirement",
    )?;
    for binding in resource_bindings {
        require_non_empty("resource capability", &binding.capability)?;
        require_non_empty("resourceRef", &binding.resource_ref)?;
    }
    validate_unique_named(
        runtime_capability_bindings
            .iter()
            .map(|binding| binding.capability.as_str()),
        "runtime capability",
    )?;
    for binding in runtime_capability_bindings {
        require_non_empty("runtime capability version", &binding.version)?;
    }
    Ok(())
}

pub(crate) fn validate_contract_ref(reference: &ServiceContractRef, label: &str) -> Result<()> {
    require_non_empty(&format!("{label}.serviceId"), &reference.service_id)?;
    require_non_empty(
        &format!("{label}.contractVersion"),
        &reference.contract_version,
    )?;
    require_non_empty(
        &format!("{label}.serviceProtocolIdentity"),
        reference.service_protocol_identity.as_str(),
    )
}

pub(crate) fn validate_package_ref(reference: &PackageArtifactRef, label: &str) -> Result<()> {
    require_non_empty(&format!("{label}.packageId"), &reference.package_id)?;
    require_non_empty(
        &format!("{label}.packageVersion"),
        &reference.package_version,
    )?;
    require_non_empty(
        &format!("{label}.packageBuildId"),
        reference.package_build_id.as_str(),
    )?;
    require_non_empty(
        &format!("{label}.packageLocalAbiIdentity"),
        reference.package_local_abi_identity.as_str(),
    )
}

pub(crate) fn validate_deployment_ref_shape(
    reference: &ServiceDeploymentRef,
    label: &str,
) -> Result<()> {
    require_non_empty(&format!("{label}.serviceId"), &reference.service_id)?;
    require_non_empty(
        &format!("{label}.contractVersion"),
        &reference.contract_version,
    )?;
    require_non_empty(
        &format!("{label}.deploymentRevision"),
        reference.deployment_revision.as_str(),
    )?;
    require_non_empty(
        &format!("{label}.deploymentArtifactIdentity"),
        reference.deployment_artifact_identity.as_str(),
    )
}

fn validate_revision(revision: &DeploymentRevision) -> Result<()> {
    require_non_empty("deploymentRevision", revision.as_str())
}

fn insert_package_coordinate<'a>(
    packages: &mut BTreeMap<skiff_artifact_model::PackageBuildId, &'a PackageArtifactRef>,
    reference: &'a PackageArtifactRef,
    label: &str,
) -> Result<()> {
    if let Some(existing) = packages.insert(reference.package_build_id.clone(), reference) {
        if existing != reference {
            return invalid_deployment(format!(
                "{label} reuses package build {} with mismatched coordinate or local ABI",
                reference.package_build_id
            ));
        }
    }
    Ok(())
}

fn validate_ingress(binding: &DeploymentIngressBinding) -> Result<()> {
    require_non_empty("ingress host", &binding.selector.host)?;
    require_non_empty("ingress path", &binding.selector.path)?;
    if !binding.selector.path.starts_with('/') {
        return invalid_deployment(format!(
            "ingress path {} must start with /",
            binding.selector.path
        ));
    }
    match binding.selector.protocol {
        skiff_artifact_model::IngressProtocol::Http => {
            let Some(method) = &binding.selector.method else {
                return invalid_deployment("HTTP ingress requires method");
            };
            require_non_empty("HTTP ingress method", method)?;
        }
        skiff_artifact_model::IngressProtocol::WebSocket => {
            if binding.selector.method.is_some() {
                return invalid_deployment("WebSocket ingress must not declare method");
            }
        }
    }
    Ok(())
}

fn validate_policy(policy: &DeploymentPolicy) -> Result<()> {
    if policy.timeout_ms == 0 {
        return invalid_deployment("policy.timeoutMs must be greater than zero");
    }
    if policy.resources.cpu_millis == 0 || policy.resources.memory_bytes == 0 {
        return invalid_deployment("resource policy limits must be greater than zero");
    }
    if policy.activation.max_concurrency == 0 {
        return invalid_deployment("activation maxConcurrency must be greater than zero");
    }
    require_non_empty("policy.principal", &policy.principal)
}

fn validate_unique_named<'a>(values: impl IntoIterator<Item = &'a str>, label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        require_non_empty(label, value)?;
        if !seen.insert(value) {
            return invalid_deployment(format!("duplicate {label} {value}"));
        }
    }
    Ok(())
}

fn require_non_empty(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid_deployment(format!("{label} must be a non-empty string"));
    }
    Ok(())
}

fn invalid_deployment<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidServiceDeployment {
        message: message.into(),
    })
}
