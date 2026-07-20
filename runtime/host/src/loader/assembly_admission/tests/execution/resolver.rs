use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    ContractOperationId, FileIrRef, FileIrUnit, OperationTargetRef, PackageArtifact,
    PackageArtifactRef, PublicationResourceRef, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef,
};
use skiff_runtime_activation::{ActivationContext, ActivationId};
use skiff_runtime_eval::RuntimeAssemblyEvalResolver;

use super::super::super::*;

pub(super) struct TypedResolver {
    pub(super) deployments: Vec<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    pub(super) contracts: Vec<(ServiceContractRef, Arc<ServiceContract>)>,
    pub(super) packages: Vec<(PackageArtifactRef, Arc<PackageArtifact>)>,
    pub(super) files: Vec<(PackageArtifactRef, FileIrRef, Arc<FileIrUnit>)>,
}

impl RuntimeAssemblyContentResolver for TypedResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.deployments
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, deployment)| Arc::clone(deployment))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing deployment"))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.contracts
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, contract)| Arc::clone(contract))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing contract"))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, package)| Arc::clone(package))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing package"))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.files
            .iter()
            .find(|(candidate_package, candidate_file, _)| {
                candidate_package == package && candidate_file == reference
            })
            .map(|(_, _, file)| Arc::clone(file))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("typed execution fixture has no static resources")
    }
}

pub(super) struct AdmittedEvalResolver {
    pub(super) activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    operation_targets: BTreeMap<(ActivationId, ContractOperationId), OperationTargetRef>,
}

impl RuntimeAssemblyEvalResolver for AdmittedEvalResolver {
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

pub(super) fn admitted_eval_resolver(active: &ActiveAssembly) -> AdmittedEvalResolver {
    let mut activations = BTreeMap::new();
    let mut operation_targets = BTreeMap::new();
    for (deployment, linked) in active.candidate().activations() {
        let binding_template = active
            .candidate()
            .assembly()
            .service_binding_templates
            .iter()
            .find(|template| &template.activation == deployment)
            .expect("admitted activation should retain its typed binding template");
        let activation = ActivationContext::from_assembly_templates(
            active.identity().clone(),
            active.generation(),
            "typed-execution-fixture-replica",
            linked.source(),
            binding_template,
        )
        .expect("admitted templates should construct an ActivationContext");
        for (operation, linked_operation) in linked.operations() {
            operation_targets.insert(
                (activation.activation_id().clone(), operation.clone()),
                linked_operation.target().clone(),
            );
        }
        assert!(
            activations
                .insert(activation.activation_id().clone(), activation)
                .is_none(),
            "activation ids must be unique within one admitted generation"
        );
    }
    let contracts = active
        .contract_store()
        .contracts()
        .map(|(reference, contract)| (reference.clone(), Arc::clone(contract)))
        .collect();
    AdmittedEvalResolver {
        activations,
        contracts,
        operation_targets,
    }
}
