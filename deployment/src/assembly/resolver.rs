use std::collections::{BTreeMap, BTreeSet, VecDeque};

use skiff_artifact_model::{
    ActivationTemplate, AssemblyIdentity, CanonicalPackageLinkPlan, GatewayIngressBinding,
    PackageArtifact, PackageArtifactRef, PackageBinding, PackageBuildId, PackageCodeSlot,
    PackageRequirement, PackageRequirementKey, ResolvedServiceBinding, RuntimeAssembly,
    ServiceBindingTemplate, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    ServiceIngressKey, ServiceRequirementKey, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};

use super::{AssemblyResolutionError, AssemblyResult, CandidateIndex};

pub(super) struct Resolver<'a, 'c> {
    candidates: &'c CandidateIndex<'a>,
    pending: VecDeque<ServiceDeploymentRef>,
    resolved_deployments: BTreeMap<ServiceDeploymentRef, &'a ServiceDeployment>,
    resolved_contracts: BTreeSet<ServiceContractRef>,
    resolved_packages: BTreeMap<PackageBuildId, PackageArtifactRef>,
    package_links: BTreeMap<PackageRequirementKey, PackageBinding>,
    service_templates: BTreeMap<ServiceDeploymentRef, ServiceBindingTemplate>,
    activation_templates: BTreeMap<ServiceDeploymentRef, ActivationTemplate>,
    gateway_ingress: BTreeMap<ServiceIngressKey, GatewayIngressBinding>,
}

impl<'a, 'c> Resolver<'a, 'c> {
    pub(super) fn new(
        candidates: &'c CandidateIndex<'a>,
        roots: impl IntoIterator<Item = ServiceDeploymentRef>,
    ) -> Self {
        Self {
            candidates,
            pending: roots.into_iter().collect(),
            resolved_deployments: BTreeMap::new(),
            resolved_contracts: BTreeSet::new(),
            resolved_packages: BTreeMap::new(),
            package_links: BTreeMap::new(),
            service_templates: BTreeMap::new(),
            activation_templates: BTreeMap::new(),
            gateway_ingress: BTreeMap::new(),
        }
    }

    pub(super) fn resolve(&mut self) -> AssemblyResult<()> {
        while let Some(reference) = self.pending.pop_front() {
            if self.resolved_deployments.contains_key(&reference) {
                continue;
            }
            self.resolve_activation(reference)?;
        }
        Ok(())
    }

    fn resolve_activation(&mut self, reference: ServiceDeploymentRef) -> AssemblyResult<()> {
        let deployment = self
            .candidates
            .deployment(&reference)
            .expect("providers and roots originate in the candidate index");
        self.resolved_deployments
            .insert(reference.clone(), deployment);

        self.candidates.contract(&deployment.contract)?;
        self.resolved_contracts.insert(deployment.contract.clone());

        let packages = self.resolve_activation_packages(&reference, deployment)?;
        let mut bindings = BTreeMap::new();
        let mut used_selector_keys = BTreeSet::new();
        for package in packages {
            for requirement in &package.service_requirements {
                let key = ServiceRequirementKey {
                    caller_package_build_id: package.package_build_id.clone(),
                    service_requirement_slot: requirement.service_binding_slot,
                };
                let selector = deployment
                    .service_selectors
                    .iter()
                    .find(|selector| selector.key == key)
                    .ok_or_else(|| AssemblyResolutionError::MissingServiceSelector {
                        activation: reference.clone(),
                        key: key.clone(),
                    })?;
                let expected = service_requirement_contract(requirement);
                if selector.contract != expected {
                    return Err(AssemblyResolutionError::ServiceSelectorMismatch {
                        activation: reference.clone(),
                        key,
                        expected,
                        selected: selector.contract.clone(),
                    });
                }

                let selected_contract = self.candidates.contract(&expected)?;
                for operation in &requirement.used_operations {
                    if !selected_contract.operations.contains_key(operation) {
                        return Err(AssemblyResolutionError::MissingServiceOperation {
                            activation: reference.clone(),
                            key: key.clone(),
                            contract: expected.clone(),
                            operation: operation.clone(),
                        });
                    }
                }
                self.resolved_contracts.insert(expected.clone());

                let provider = self.candidates.provider(&expected)?;
                self.pending.push_back(provider.clone());
                used_selector_keys.insert(key.clone());
                bindings.insert(
                    key.clone(),
                    ResolvedServiceBinding {
                        key,
                        contract: expected,
                        provider,
                        used_operations: requirement.used_operations.iter().cloned().collect(),
                    },
                );
            }
        }
        self.reject_unused_service_selectors(&reference, deployment, &used_selector_keys)?;
        self.insert_gateway_ingress(&reference, deployment)?;
        self.insert_templates(&reference, deployment, bindings);
        Ok(())
    }

    fn reject_unused_service_selectors(
        &self,
        activation: &ServiceDeploymentRef,
        deployment: &ServiceDeployment,
        used_keys: &BTreeSet<ServiceRequirementKey>,
    ) -> AssemblyResult<()> {
        for selector in &deployment.service_selectors {
            if !used_keys.contains(&selector.key) {
                return Err(AssemblyResolutionError::UnexpectedServiceSelector {
                    activation: activation.clone(),
                    key: selector.key.clone(),
                });
            }
        }
        Ok(())
    }

    fn insert_templates(
        &mut self,
        reference: &ServiceDeploymentRef,
        deployment: &ServiceDeployment,
        bindings: BTreeMap<ServiceRequirementKey, ResolvedServiceBinding>,
    ) {
        self.service_templates.insert(
            reference.clone(),
            ServiceBindingTemplate {
                activation: reference.clone(),
                bindings: bindings.into_values().collect(),
            },
        );
        let mut activation = ActivationTemplate {
            deployment: reference.clone(),
            implementation_package_build_id: deployment.implementation.package_build_id.clone(),
            config_literals: deployment.config_literals.clone(),
            secret_refs: deployment.secret_refs.clone(),
            state_bindings: deployment.state_bindings.clone(),
            resource_bindings: deployment.resource_bindings.clone(),
            policy: deployment.policy.clone(),
        };
        activation
            .config_literals
            .sort_by(|left, right| left.path.cmp(&right.path));
        activation
            .secret_refs
            .sort_by(|left, right| left.path.cmp(&right.path));
        activation
            .state_bindings
            .sort_by(|left, right| left.requirement_key.cmp(&right.requirement_key));
        activation
            .resource_bindings
            .sort_by(|left, right| left.requirement_key.cmp(&right.requirement_key));
        self.activation_templates
            .insert(reference.clone(), activation);
    }

    fn insert_gateway_ingress(
        &mut self,
        reference: &ServiceDeploymentRef,
        deployment: &ServiceDeployment,
    ) -> AssemblyResult<()> {
        for source in &deployment.ingress {
            let entry = deployment
                .gateway_entries
                .get(&source.gateway_entry_key)
                .ok_or_else(|| AssemblyResolutionError::MissingGatewayEntry {
                    activation: reference.clone(),
                    gateway_entry_key: source.gateway_entry_key.clone(),
                })?;
            let binding = GatewayIngressBinding {
                selector: source.selector.clone(),
                deployment: reference.clone(),
                gateway_entry_key: source.gateway_entry_key.clone(),
                gateway_entry_identity: entry.gateway_entry_identity.clone(),
            };
            let key = binding.service_ingress_key();
            if let Some(first) = self.gateway_ingress.insert(key.clone(), binding) {
                return Err(AssemblyResolutionError::GatewayIngressCollision {
                    key,
                    first: first.deployment,
                    second: reference.clone(),
                });
            }
        }
        Ok(())
    }

    fn resolve_activation_packages(
        &mut self,
        activation: &ServiceDeploymentRef,
        deployment: &'a ServiceDeployment,
    ) -> AssemblyResult<Vec<&'a PackageArtifact>> {
        let mut pending = VecDeque::from([deployment.implementation.clone()]);
        let mut visited = BTreeSet::new();
        let mut closure = BTreeMap::new();
        let mut used_binding_keys = BTreeSet::new();

        while let Some(reference) = pending.pop_front() {
            let package = self.candidates.package(&reference)?;
            if !visited.insert(package.package_build_id.clone()) {
                continue;
            }
            self.insert_resolved_package(reference.clone())?;
            closure.insert(package.package_build_id.clone(), package);

            for requirement in &package.package_requirements {
                let key = PackageRequirementKey {
                    caller_package_build_id: package.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                };
                let binding = deployment
                    .package_bindings
                    .iter()
                    .find(|binding| binding.key == key)
                    .ok_or_else(|| AssemblyResolutionError::MissingPackageBinding {
                        activation: activation.clone(),
                        key: key.clone(),
                    })?;
                if !package_requirement_matches(requirement, binding) {
                    return Err(AssemblyResolutionError::PackageRequirementMismatch {
                        activation: activation.clone(),
                        key,
                        requirement: requirement.clone(),
                        selected: binding.package.clone(),
                    });
                }
                self.insert_package_link(binding)?;
                used_binding_keys.insert(key);
                pending.push_back(binding.package.clone());
            }
        }

        for binding in &deployment.package_bindings {
            if !used_binding_keys.contains(&binding.key) {
                return Err(AssemblyResolutionError::UnexpectedPackageBinding {
                    activation: activation.clone(),
                    key: binding.key.clone(),
                });
            }
        }
        Ok(closure.into_values().collect())
    }

    fn insert_resolved_package(&mut self, reference: PackageArtifactRef) -> AssemblyResult<()> {
        if let Some(existing) = self
            .resolved_packages
            .insert(reference.package_build_id.clone(), reference.clone())
        {
            if existing != reference {
                return Err(AssemblyResolutionError::PackageReferenceMismatch {
                    expected: existing,
                    available: reference,
                });
            }
        }
        Ok(())
    }

    fn insert_package_link(&mut self, binding: &PackageBinding) -> AssemblyResult<()> {
        if let Some(existing) = self
            .package_links
            .insert(binding.key.clone(), binding.clone())
        {
            if existing != *binding {
                return Err(AssemblyResolutionError::ConflictingPackageLink {
                    key: binding.key.clone(),
                    first: existing.package,
                    second: binding.package.clone(),
                });
            }
        }
        Ok(())
    }

    pub(super) fn into_assembly(self, roots: BTreeSet<ServiceDeploymentRef>) -> RuntimeAssembly {
        let mut resolved_packages = self.resolved_packages.into_values().collect::<Vec<_>>();
        resolved_packages.sort();
        RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("unassigned"),
            roots: roots.into_iter().collect(),
            resolved_deployments: self.resolved_deployments.into_keys().collect(),
            resolved_contracts: self.resolved_contracts.into_iter().collect(),
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: resolved_packages
                    .iter()
                    .cloned()
                    .map(|package| PackageCodeSlot { package })
                    .collect(),
                package_links: self.package_links.into_values().collect(),
            },
            resolved_packages,
            service_binding_templates: self.service_templates.into_values().collect(),
            activation_templates: self.activation_templates.into_values().collect(),
            gateway_ingress: self.gateway_ingress.into_values().collect(),
        }
    }
}

fn service_requirement_contract(
    requirement: &skiff_artifact_model::ServiceRequirement,
) -> ServiceContractRef {
    ServiceContractRef {
        service_id: requirement.contract_requirement.service_id.clone(),
        contract_version: requirement.contract_requirement.contract_version.clone(),
        service_protocol_identity: requirement
            .contract_requirement
            .expected_protocol_identity
            .clone(),
    }
}

fn package_requirement_matches(
    requirement: &PackageRequirement,
    selected: &PackageBinding,
) -> bool {
    selected.collection_name_mapping == requirement.collection_name_mapping
        && selected.package.package_id == requirement.package_id
        && selected.package.package_version == requirement.exact_version
        && selected.package.package_local_abi_identity == requirement.expected_local_abi
        && requirement
            .expected_package_build
            .as_ref()
            .is_none_or(|expected| expected == &selected.package.package_build_id)
}
