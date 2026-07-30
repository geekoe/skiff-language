use std::{collections::BTreeMap, fmt, sync::Arc};

use skiff_artifact_model::{
    ActivationTemplate, AssemblyIdentity, ConfigLiteralBinding, ContractOperationId,
    DeploymentPolicy, GatewayEntryIdentity, GatewayEntryKey, IngressSelector, PackageBuildId,
    SecretRefBinding, ServiceBindingTemplate, ServiceContractRef, ServiceDeploymentRef,
    ServiceProtocolIdentity, ServiceRequirementKey, StateBinding, WebSocketEntryId,
};

use crate::capability::CallbackCapabilityTable;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActivationId(String);

impl ActivationId {
    pub fn from_identity(identity: &ActivationIdentity) -> Self {
        let deployment = &identity.deployment;
        Self(frame_identity_parts(&[
            identity.assembly_identity.as_str(),
            &identity.assembly_generation.to_string(),
            &identity.runtime_replica_id,
            &deployment.service_id,
            &deployment.contract_version,
            deployment.deployment_revision.as_str(),
            deployment.deployment_artifact_identity.as_str(),
        ]))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn frame_identity_parts(parts: &[&str]) -> String {
    let mut framed = String::from("activation-v1");
    for part in parts {
        framed.push(':');
        framed.push_str(&part.len().to_string());
        framed.push(':');
        framed.push_str(part);
    }
    framed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIdentity {
    pub assembly_identity: AssemblyIdentity,
    pub assembly_generation: u64,
    pub runtime_replica_id: String,
    pub deployment: ServiceDeploymentRef,
}

impl ActivationIdentity {
    pub fn activation_id(&self) -> ActivationId {
        ActivationId::from_identity(self)
    }
}

#[derive(Clone, PartialEq)]
pub struct ActivationOwnedBindings {
    pub config_literals: Vec<ConfigLiteralBinding>,
    pub secret_refs: Vec<SecretRefBinding>,
    pub state_bindings: Vec<StateBinding>,
    pub policy: DeploymentPolicy,
}

impl fmt::Debug for ActivationOwnedBindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationOwnedBindings")
            .field(
                "config_literal_paths",
                &self
                    .config_literals
                    .iter()
                    .map(|binding| binding.path.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("secret_refs", &self.secret_refs)
            .field("state_bindings", &self.state_bindings)
            .field("policy", &self.policy)
            .finish()
    }
}

/// The exact compiler-owned WebSocket entry admitted for one service activation.
///
/// A service may have no such entry, but it can never have more than one. Keeping the
/// selector and both identities together prevents request or native callers from filling
/// missing entry facts with defaults or caller-provided strings.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivationWebSocketEntry {
    selector: IngressSelector,
    gateway_entry_key: GatewayEntryKey,
    gateway_entry_identity: GatewayEntryIdentity,
    websocket_entry_id: WebSocketEntryId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationServiceBinding {
    key: ServiceRequirementKey,
    provider_activation_id: ActivationId,
    contract: ServiceContractRef,
    used_operations: Vec<ContractOperationId>,
}

impl ActivationServiceBinding {
    pub fn new(
        key: ServiceRequirementKey,
        provider_activation_id: ActivationId,
        contract: ServiceContractRef,
        mut used_operations: Vec<ContractOperationId>,
    ) -> Result<Self, ActivationContextError> {
        used_operations.sort();
        if used_operations.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ActivationContextError::DuplicateUsedOperation { key });
        }
        Ok(Self {
            key,
            provider_activation_id,
            contract,
            used_operations,
        })
    }

    pub fn key(&self) -> &ServiceRequirementKey {
        &self.key
    }

    pub fn provider_activation_id(&self) -> &ActivationId {
        &self.provider_activation_id
    }

    pub fn contract(&self) -> &ServiceContractRef {
        &self.contract
    }

    pub fn used_operations(&self) -> &[ContractOperationId] {
        &self.used_operations
    }

    fn uses_operation(&self, operation: &ContractOperationId) -> bool {
        self.used_operations.binary_search(operation).is_ok()
    }
}

#[derive(Debug)]
pub struct ActivationContext {
    identity: ActivationIdentity,
    activation_id: ActivationId,
    implementation_package_build_id: PackageBuildId,
    owned_bindings: ActivationOwnedBindings,
    websocket_entry: Option<ActivationWebSocketEntry>,
    service_bindings: BTreeMap<ServiceRequirementKey, ActivationServiceBinding>,
    callback_capabilities: CallbackCapabilityTable,
}

impl ActivationContext {
    pub fn new(
        identity: ActivationIdentity,
        implementation_package_build_id: PackageBuildId,
        owned_bindings: ActivationOwnedBindings,
        service_bindings: Vec<ActivationServiceBinding>,
    ) -> Result<Arc<Self>, ActivationContextError> {
        Self::new_with_websocket_entry(
            identity,
            implementation_package_build_id,
            owned_bindings,
            None,
            service_bindings,
        )
    }

    pub fn new_with_websocket_entry(
        identity: ActivationIdentity,
        implementation_package_build_id: PackageBuildId,
        owned_bindings: ActivationOwnedBindings,
        websocket_entry: Option<(
            IngressSelector,
            GatewayEntryKey,
            GatewayEntryIdentity,
            WebSocketEntryId,
        )>,
        service_bindings: Vec<ActivationServiceBinding>,
    ) -> Result<Arc<Self>, ActivationContextError> {
        let activation_id = identity.activation_id();
        let mut bindings_by_key = BTreeMap::new();
        for binding in service_bindings {
            let key = binding.key.clone();
            if bindings_by_key.insert(key.clone(), binding).is_some() {
                return Err(ActivationContextError::DuplicateServiceBinding { key });
            }
        }
        let callback_capabilities = CallbackCapabilityTable::new(
            identity.runtime_replica_id.clone(),
            activation_id.as_str().to_string(),
        );
        Ok(Arc::new(Self {
            identity,
            activation_id,
            implementation_package_build_id,
            owned_bindings,
            websocket_entry: websocket_entry.map(
                |(selector, gateway_entry_key, gateway_entry_identity, websocket_entry_id)| {
                    ActivationWebSocketEntry {
                        selector,
                        gateway_entry_key,
                        gateway_entry_identity,
                        websocket_entry_id,
                    }
                },
            ),
            service_bindings: bindings_by_key,
            callback_capabilities,
        }))
    }

    pub fn from_assembly_templates(
        assembly_identity: AssemblyIdentity,
        assembly_generation: u64,
        runtime_replica_id: impl Into<String>,
        activation_template: &ActivationTemplate,
        service_binding_template: &ServiceBindingTemplate,
    ) -> Result<Arc<Self>, ActivationContextError> {
        Self::from_assembly_templates_with_websocket_entry(
            assembly_identity,
            assembly_generation,
            runtime_replica_id,
            activation_template,
            service_binding_template,
            None,
        )
    }

    pub fn from_assembly_templates_with_websocket_entry(
        assembly_identity: AssemblyIdentity,
        assembly_generation: u64,
        runtime_replica_id: impl Into<String>,
        activation_template: &ActivationTemplate,
        service_binding_template: &ServiceBindingTemplate,
        websocket_entry: Option<(
            IngressSelector,
            GatewayEntryKey,
            GatewayEntryIdentity,
            WebSocketEntryId,
        )>,
    ) -> Result<Arc<Self>, ActivationContextError> {
        Self::from_assembly_templates_with_config_literals(
            assembly_identity,
            assembly_generation,
            runtime_replica_id,
            activation_template,
            service_binding_template,
            activation_template.config_literals.clone(),
            websocket_entry,
        )
    }

    pub fn from_assembly_templates_with_resolved_secrets_and_websocket_entry(
        assembly_identity: AssemblyIdentity,
        assembly_generation: u64,
        runtime_replica_id: impl Into<String>,
        activation_template: &ActivationTemplate,
        service_binding_template: &ServiceBindingTemplate,
        resolved_secrets: &[ConfigLiteralBinding],
        websocket_entry: Option<(
            IngressSelector,
            GatewayEntryKey,
            GatewayEntryIdentity,
            WebSocketEntryId,
        )>,
    ) -> Result<Arc<Self>, ActivationContextError> {
        if activation_template.deployment != service_binding_template.activation {
            return Err(ActivationContextError::TemplateDeploymentMismatch {
                activation: activation_template.deployment.clone(),
                bindings: service_binding_template.activation.clone(),
            });
        }
        let expected_secret_paths = activation_template
            .secret_refs
            .iter()
            .map(|binding| binding.path.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut resolved_secret_paths = std::collections::BTreeSet::new();
        for binding in resolved_secrets {
            if !expected_secret_paths.contains(binding.path.as_str()) {
                return Err(ActivationContextError::UnexpectedResolvedSecret {
                    path: binding.path.clone(),
                });
            }
            if !resolved_secret_paths.insert(binding.path.as_str()) {
                return Err(ActivationContextError::DuplicateResolvedSecret {
                    path: binding.path.clone(),
                });
            }
        }
        if let Some(path) = expected_secret_paths
            .iter()
            .find(|path| !resolved_secret_paths.contains(**path))
        {
            return Err(ActivationContextError::MissingResolvedSecret {
                path: (*path).to_string(),
            });
        }
        let mut config_literals = activation_template.config_literals.clone();
        config_literals.extend_from_slice(resolved_secrets);
        Self::from_assembly_templates_with_config_literals(
            assembly_identity,
            assembly_generation,
            runtime_replica_id,
            activation_template,
            service_binding_template,
            config_literals,
            websocket_entry,
        )
    }

    fn from_assembly_templates_with_config_literals(
        assembly_identity: AssemblyIdentity,
        assembly_generation: u64,
        runtime_replica_id: impl Into<String>,
        activation_template: &ActivationTemplate,
        service_binding_template: &ServiceBindingTemplate,
        config_literals: Vec<ConfigLiteralBinding>,
        websocket_entry: Option<(
            IngressSelector,
            GatewayEntryKey,
            GatewayEntryIdentity,
            WebSocketEntryId,
        )>,
    ) -> Result<Arc<Self>, ActivationContextError> {
        if activation_template.deployment != service_binding_template.activation {
            return Err(ActivationContextError::TemplateDeploymentMismatch {
                activation: activation_template.deployment.clone(),
                bindings: service_binding_template.activation.clone(),
            });
        }
        let runtime_replica_id = runtime_replica_id.into();
        let identity = ActivationIdentity {
            assembly_identity: assembly_identity.clone(),
            assembly_generation,
            runtime_replica_id: runtime_replica_id.clone(),
            deployment: activation_template.deployment.clone(),
        };
        let service_bindings = service_binding_template
            .bindings
            .iter()
            .map(|binding| {
                let provider_identity = ActivationIdentity {
                    assembly_identity: assembly_identity.clone(),
                    assembly_generation,
                    runtime_replica_id: runtime_replica_id.clone(),
                    deployment: binding.provider.clone(),
                };
                ActivationServiceBinding::new(
                    binding.key.clone(),
                    provider_identity.activation_id(),
                    binding.contract.clone(),
                    binding.used_operations.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new_with_websocket_entry(
            identity,
            activation_template.implementation_package_build_id.clone(),
            ActivationOwnedBindings {
                config_literals,
                secret_refs: activation_template.secret_refs.clone(),
                state_bindings: activation_template.state_bindings.clone(),
                policy: activation_template.policy.clone(),
            },
            websocket_entry,
            service_bindings,
        )
    }

    pub fn identity(&self) -> &ActivationIdentity {
        &self.identity
    }

    pub fn activation_id(&self) -> &ActivationId {
        &self.activation_id
    }

    pub fn implementation_package_build_id(&self) -> &PackageBuildId {
        &self.implementation_package_build_id
    }

    pub fn owned_bindings(&self) -> &ActivationOwnedBindings {
        &self.owned_bindings
    }

    pub fn websocket_entry_id(&self) -> Option<&WebSocketEntryId> {
        self.websocket_entry
            .as_ref()
            .map(|entry| &entry.websocket_entry_id)
    }

    pub fn websocket_entry_matches(
        &self,
        selector: &IngressSelector,
        gateway_entry_key: &GatewayEntryKey,
        gateway_entry_identity: &GatewayEntryIdentity,
        websocket_entry_id: &WebSocketEntryId,
    ) -> bool {
        self.websocket_entry.as_ref().is_some_and(|entry| {
            entry.selector == *selector
                && entry.gateway_entry_key == *gateway_entry_key
                && entry.gateway_entry_identity == *gateway_entry_identity
                && entry.websocket_entry_id == *websocket_entry_id
        })
    }

    pub fn callback_capabilities(&self) -> &CallbackCapabilityTable {
        &self.callback_capabilities
    }

    pub fn resolve_service_binding(
        &self,
        caller_package_build_id: &PackageBuildId,
        service_requirement_slot: u32,
        expected_protocol_identity: &ServiceProtocolIdentity,
        operation: &ContractOperationId,
    ) -> Result<&ActivationServiceBinding, ActivationContextError> {
        let key = ServiceRequirementKey {
            caller_package_build_id: caller_package_build_id.clone(),
            service_requirement_slot,
        };
        let binding = self
            .service_bindings
            .get(&key)
            .ok_or_else(|| ActivationContextError::MissingServiceBinding { key: key.clone() })?;
        if &binding.contract.service_protocol_identity != expected_protocol_identity {
            return Err(ActivationContextError::ServiceProtocolMismatch {
                key,
                expected: expected_protocol_identity.clone(),
                actual: binding.contract.service_protocol_identity.clone(),
            });
        }
        if !binding.uses_operation(operation) {
            return Err(ActivationContextError::OperationNotUsed {
                key,
                operation: operation.clone(),
            });
        }
        Ok(binding)
    }

    pub fn mark_owner_unavailable(&self) {
        self.callback_capabilities.mark_owner_unavailable();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivationContextError {
    #[error("activation and service-binding templates target different deployments")]
    TemplateDeploymentMismatch {
        activation: ServiceDeploymentRef,
        bindings: ServiceDeploymentRef,
    },
    #[error("activation resolved an undeclared secret path {path}")]
    UnexpectedResolvedSecret { path: String },
    #[error("activation resolved secret path {path} more than once")]
    DuplicateResolvedSecret { path: String },
    #[error("activation did not resolve required secret path {path}")]
    MissingResolvedSecret { path: String },
    #[error("activation service binding {key:?} is duplicated")]
    DuplicateServiceBinding { key: ServiceRequirementKey },
    #[error("activation service binding {key:?} repeats a used operation")]
    DuplicateUsedOperation { key: ServiceRequirementKey },
    #[error("activation service binding {key:?} is missing")]
    MissingServiceBinding { key: ServiceRequirementKey },
    #[error("activation service binding {key:?} has a different protocol identity")]
    ServiceProtocolMismatch {
        key: ServiceRequirementKey,
        expected: ServiceProtocolIdentity,
        actual: ServiceProtocolIdentity,
    },
    #[error("activation service binding {key:?} does not authorize operation {operation}")]
    OperationNotUsed {
        key: ServiceRequirementKey,
        operation: ContractOperationId,
    },
    #[error("activation switch crosses assembly identity, generation, or runtime replica")]
    CrossAssemblyActivationSwitch,
    #[error("request generation space is exhausted")]
    RequestGenerationExhausted,
}
