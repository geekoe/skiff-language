use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    ActivationTemplate, BoundaryOperationDescriptor, ContractOperationId, GatewayEntryKey,
    OperationTargetRef, PackageBuildId, PackageCallableId, RuntimeAssembly, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, ServiceIngressKey, ServiceProtocolIdentity,
    ServiceRequirementKey,
};
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, AssemblyExecutionImage, PackageCodeSlotIndex,
    SharedPackageLinkedImage,
};
use skiff_runtime_loader::ServiceContractStore;

use super::LinkedGatewayEntry;

/// An implementation operation linked to immutable package code.
///
/// The canonical boundary descriptor deliberately remains in [`ServiceContractStore`]; this
/// value only retains the provider-local callable and executable target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedContractOperation {
    pub(super) contract_operation_id: ContractOperationId,
    pub(super) package_callable_id: PackageCallableId,
    pub(super) target: OperationTargetRef,
}

impl LinkedContractOperation {
    pub fn operation_id(&self) -> &ContractOperationId {
        &self.contract_operation_id
    }

    pub fn package_callable_id(&self) -> &PackageCallableId {
        &self.package_callable_id
    }

    pub fn target(&self) -> &OperationTargetRef {
        &self.target
    }
}

/// One activation-relative service binding.
///
/// There is intentionally no provider package, code slot or executable target here. Runtime
/// dispatch first resolves this value against the caller activation, crosses the service
/// boundary, and only then consults the provider activation's operation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedServiceBindingTemplate {
    pub(super) key: ServiceRequirementKey,
    pub(super) contract: ServiceContractRef,
    pub(super) provider: ServiceDeploymentRef,
    pub(super) used_operations: BTreeSet<ContractOperationId>,
}

impl LinkedServiceBindingTemplate {
    pub fn key(&self) -> &ServiceRequirementKey {
        &self.key
    }

    pub fn contract(&self) -> &ServiceContractRef {
        &self.contract
    }

    pub fn provider(&self) -> &ServiceDeploymentRef {
        &self.provider
    }

    pub fn used_operations(&self) -> &BTreeSet<ContractOperationId> {
        &self.used_operations
    }
}

/// Immutable input for creating one Phase 04 activation owner.
///
/// No `ActivationContext`, callback table, state handle or other mutable owner is created while
/// linking. Two entries may point at the same shared code slot while retaining distinct source
/// templates and service-binding maps.
#[derive(Debug)]
pub struct LinkedActivationTemplate {
    pub(super) source: ActivationTemplate,
    pub(super) deployment: Arc<ServiceDeployment>,
    pub(super) implementation_code_slot: PackageCodeSlotIndex,
    pub(super) operations: BTreeMap<ContractOperationId, LinkedContractOperation>,
    pub(super) service_bindings: BTreeMap<ServiceRequirementKey, LinkedServiceBindingTemplate>,
}

impl LinkedActivationTemplate {
    pub fn source(&self) -> &ActivationTemplate {
        &self.source
    }

    pub fn deployment_ref(&self) -> &ServiceDeploymentRef {
        &self.source.deployment
    }

    pub fn deployment(&self) -> &Arc<ServiceDeployment> {
        &self.deployment
    }

    pub fn contract(&self) -> &ServiceContractRef {
        &self.deployment.contract
    }

    pub fn implementation_package_build_id(&self) -> &PackageBuildId {
        &self.source.implementation_package_build_id
    }

    pub fn implementation_code_slot(&self) -> PackageCodeSlotIndex {
        self.implementation_code_slot
    }

    pub fn operations(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ContractOperationId, &LinkedContractOperation)> {
        self.operations.iter()
    }

    pub fn operation(&self, operation: &ContractOperationId) -> Option<&LinkedContractOperation> {
        self.operations.get(operation)
    }

    pub fn service_bindings(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ServiceRequirementKey, &LinkedServiceBindingTemplate)>
    {
        self.service_bindings.iter()
    }

    pub fn service_binding(
        &self,
        caller_package_build_id: &PackageBuildId,
        service_requirement_slot: u32,
    ) -> Option<&LinkedServiceBindingTemplate> {
        self.service_bindings.get(&ServiceRequirementKey {
            caller_package_build_id: caller_package_build_id.clone(),
            service_requirement_slot,
        })
    }
}

/// Immutable result of linking one fully hydrated runtime assembly.
#[derive(Debug)]
pub struct AssemblyLinkedCandidate {
    pub(super) assembly: Arc<RuntimeAssembly>,
    pub(super) shared_image: Arc<SharedPackageLinkedImage>,
    pub(super) execution_image: Arc<AssemblyExecutionImage>,
    pub(super) contracts: Arc<ServiceContractStore>,
    pub(super) activations: BTreeMap<ServiceDeploymentRef, LinkedActivationTemplate>,
    pub(super) gateway_entries:
        BTreeMap<(ServiceDeploymentRef, GatewayEntryKey), Arc<LinkedGatewayEntry>>,
    pub(super) ingress: BTreeMap<ServiceIngressKey, Arc<LinkedGatewayEntry>>,
}

impl AssemblyLinkedCandidate {
    pub fn assembly(&self) -> &Arc<RuntimeAssembly> {
        &self.assembly
    }

    pub fn shared_image(&self) -> &Arc<SharedPackageLinkedImage> {
        &self.shared_image
    }

    pub fn execution_image(&self) -> &Arc<AssemblyExecutionImage> {
        &self.execution_image
    }

    pub fn contract_store(&self) -> &Arc<ServiceContractStore> {
        &self.contracts
    }

    /// Typed canonical descriptor/value-plan lookup retained through admission.
    pub fn operation_descriptor(
        &self,
        contract: &ServiceContractRef,
        operation: &ContractOperationId,
    ) -> Option<&BoundaryOperationDescriptor> {
        self.contracts.operation_descriptor(contract, operation)
    }

    pub fn activations(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ServiceDeploymentRef, &LinkedActivationTemplate)> {
        self.activations.iter()
    }

    pub fn activation(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Option<&LinkedActivationTemplate> {
        self.activations.get(deployment)
    }

    pub fn gateway_entries(
        &self,
    ) -> impl ExactSizeIterator<
        Item = (
            &(ServiceDeploymentRef, GatewayEntryKey),
            &Arc<LinkedGatewayEntry>,
        ),
    > {
        self.gateway_entries.iter()
    }

    pub fn gateway_entry(
        &self,
        owner: &ServiceDeploymentRef,
        key: &GatewayEntryKey,
    ) -> Option<&Arc<LinkedGatewayEntry>> {
        self.gateway_entries.get(&(owner.clone(), key.clone()))
    }

    pub fn ingress_bindings(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ServiceIngressKey, &Arc<LinkedGatewayEntry>)> {
        self.ingress.iter()
    }

    pub fn ingress(&self, key: &ServiceIngressKey) -> Option<&Arc<LinkedGatewayEntry>> {
        self.ingress.get(key)
    }

    pub fn is_empty(&self) -> bool {
        self.shared_image.is_empty()
            && self.activations.is_empty()
            && self.gateway_entries.is_empty()
            && self.ingress.is_empty()
            && self.contracts.is_empty()
    }

    /// Resolves only the caller-relative service binding. Provider code remains unresolved until
    /// the runtime crosses into `binding.provider()` and consults that activation's operation.
    pub fn resolve_activation_relative_service_call(
        &self,
        activation: &ServiceDeploymentRef,
        call: &ActivationRelativeServiceCall,
    ) -> Result<&LinkedServiceBindingTemplate, AssemblyServiceCallError> {
        let activation_template = self.activations.get(activation).ok_or_else(|| {
            AssemblyServiceCallError::MissingActivation {
                activation: Box::new(activation.clone()),
            }
        })?;
        let key = ServiceRequirementKey {
            caller_package_build_id: call.caller_package_build_id().clone(),
            service_requirement_slot: call.service_requirement_slot(),
        };
        let binding = activation_template
            .service_bindings
            .get(&key)
            .ok_or_else(|| AssemblyServiceCallError::MissingBinding {
                activation: Box::new(activation.clone()),
                key: key.clone(),
            })?;
        if binding.contract.service_protocol_identity != *call.expected_protocol_identity() {
            return Err(AssemblyServiceCallError::ProtocolMismatch {
                activation: Box::new(activation.clone()),
                key,
                expected: call.expected_protocol_identity().clone(),
                actual: binding.contract.service_protocol_identity.clone(),
            });
        }
        if !binding.used_operations.contains(call.operation_id()) {
            return Err(AssemblyServiceCallError::OperationNotBound {
                activation: Box::new(activation.clone()),
                key,
                operation: call.operation_id().clone(),
            });
        }
        if self
            .contracts
            .operation_descriptor(&binding.contract, call.operation_id())
            .is_none()
        {
            return Err(AssemblyServiceCallError::MissingContractOperation {
                contract: binding.contract.clone(),
                operation: call.operation_id().clone(),
            });
        }
        Ok(binding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssemblyServiceCallError {
    #[error("activation {activation:?} is not part of the linked assembly")]
    MissingActivation {
        activation: Box<ServiceDeploymentRef>,
    },
    #[error("activation {activation:?} has no service binding for {key:?}")]
    MissingBinding {
        activation: Box<ServiceDeploymentRef>,
        key: ServiceRequirementKey,
    },
    #[error("activation {activation:?} service binding {key:?} protocol mismatch")]
    ProtocolMismatch {
        activation: Box<ServiceDeploymentRef>,
        key: ServiceRequirementKey,
        expected: ServiceProtocolIdentity,
        actual: ServiceProtocolIdentity,
    },
    #[error("activation {activation:?} service binding {key:?} does not use {operation}")]
    OperationNotBound {
        activation: Box<ServiceDeploymentRef>,
        key: ServiceRequirementKey,
        operation: ContractOperationId,
    },
    #[error("contract {contract:?} has no operation {operation}")]
    MissingContractOperation {
        contract: ServiceContractRef,
        operation: ContractOperationId,
    },
}
