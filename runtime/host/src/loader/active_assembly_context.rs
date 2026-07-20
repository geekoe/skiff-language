use std::{collections::BTreeMap, sync::Arc};

use anyhow::Context;
use skiff_artifact_model::{
    ContractOperationId, OperationTargetRef, ServiceContract, ServiceContractRef,
    ServiceDeploymentRef,
};
use skiff_runtime_activation::{ActivationContext, ActivationId};
use skiff_runtime_eval::RuntimeAssemblyEvalResolver;
use skiff_runtime_linker::AssemblyLinkedCandidate;

/// Immutable activation owners and canonical target facts published with one assembly generation.
#[derive(Debug)]
pub(crate) struct ActiveAssemblyContextSet {
    activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    activations_by_deployment: BTreeMap<ServiceDeploymentRef, Arc<ActivationContext>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    operation_targets: BTreeMap<(ActivationId, ContractOperationId), OperationTargetRef>,
}

impl ActiveAssemblyContextSet {
    pub(crate) fn from_candidate(
        candidate: &AssemblyLinkedCandidate,
        generation: u64,
        runtime_replica_id: &str,
    ) -> anyhow::Result<Self> {
        if runtime_replica_id.trim().is_empty() {
            anyhow::bail!("runtime replica id must be non-empty for activation construction");
        }
        let mut activations = BTreeMap::new();
        let mut activations_by_deployment = BTreeMap::new();
        let mut operation_targets = BTreeMap::new();
        for (deployment, linked) in candidate.activations() {
            let binding_template = candidate
                .assembly()
                .service_binding_templates
                .iter()
                .find(|template| &template.activation == deployment)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "admitted activation {:?} has no service binding template",
                        deployment
                    )
                })?;
            let activation = ActivationContext::from_assembly_templates(
                candidate.assembly().assembly_identity.clone(),
                generation,
                runtime_replica_id,
                linked.source(),
                binding_template,
            )
            .with_context(|| {
                format!(
                    "failed to construct activation context for deployment {:?}",
                    deployment
                )
            })?;
            for (operation, linked_operation) in linked.operations() {
                if operation_targets
                    .insert(
                        (activation.activation_id().clone(), operation.clone()),
                        linked_operation.target().clone(),
                    )
                    .is_some()
                {
                    anyhow::bail!(
                        "activation {} repeats canonical operation {}",
                        activation.activation_id().as_str(),
                        operation
                    );
                }
            }
            if activations
                .insert(activation.activation_id().clone(), Arc::clone(&activation))
                .is_some()
            {
                anyhow::bail!(
                    "activation id {} is duplicated within generation {generation}",
                    activation.activation_id().as_str()
                );
            }
            if activations_by_deployment
                .insert(deployment.clone(), activation)
                .is_some()
            {
                anyhow::bail!(
                    "deployment {:?} has duplicate activation contexts",
                    deployment
                );
            }
        }
        if activations.len() != candidate.activations().len() {
            anyhow::bail!("activation context set does not exactly match linked activations");
        }
        let contracts = candidate
            .contract_store()
            .contracts()
            .map(|(reference, contract)| (reference.clone(), Arc::clone(contract)))
            .collect();
        Ok(Self {
            activations,
            activations_by_deployment,
            contracts,
            operation_targets,
        })
    }

    pub(crate) fn activation_for_deployment(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Option<Arc<ActivationContext>> {
        self.activations_by_deployment.get(deployment).cloned()
    }
}

impl RuntimeAssemblyEvalResolver for ActiveAssemblyContextSet {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        self.activations.get(activation_id).cloned()
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        self.activations
            .values()
            .find(|activation| activation.activation_id().as_str() == activation_id)
            .cloned()
    }

    fn contract(&self, contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        self.contracts.get(contract).cloned()
    }

    fn operation_target(
        &self,
        activation_id: &ActivationId,
        operation: &ContractOperationId,
    ) -> Option<OperationTargetRef> {
        self.operation_targets
            .get(&(activation_id.clone(), operation.clone()))
            .cloned()
    }
}
