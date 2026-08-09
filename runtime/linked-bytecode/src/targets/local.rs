use skiff_artifact_model::{
    ActorAbiIdentity, ActorMethodIdentity, ContractOperationId, ServiceRequirementKey,
};

use crate::{ActorMethodIndex, FunctionIndex, ServiceOperationIndex, SpecializationKey};

/// Exact concrete local target. The key and function remain visible so the
/// verifier can independently compare target specialization and code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedExactLocalTarget {
    key: SpecializationKey,
    function: FunctionIndex,
}

impl LinkedExactLocalTarget {
    pub fn new(key: SpecializationKey, function: FunctionIndex) -> Self {
        Self { key, function }
    }

    pub const fn key(&self) -> &SpecializationKey {
        &self.key
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }
}

/// Activation-relative service target. It intentionally contains no provider
/// deployment, build identity, executable address, or function index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedServiceOperationTarget {
    index: ServiceOperationIndex,
    service_requirement_key: ServiceRequirementKey,
    contract_operation_id: ContractOperationId,
}

impl LinkedServiceOperationTarget {
    pub fn new(
        index: ServiceOperationIndex,
        service_requirement_key: ServiceRequirementKey,
        contract_operation_id: ContractOperationId,
    ) -> Self {
        Self {
            index,
            service_requirement_key,
            contract_operation_id,
        }
    }

    pub const fn index(&self) -> ServiceOperationIndex {
        self.index
    }

    pub const fn service_requirement_key(&self) -> &ServiceRequirementKey {
        &self.service_requirement_key
    }

    pub const fn contract_operation_id(&self) -> &ContractOperationId {
        &self.contract_operation_id
    }
}

/// Actor entry target inside the exact owner image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedActorMethodTarget {
    index: ActorMethodIndex,
    actor_abi_identity: ActorAbiIdentity,
    method_identity: ActorMethodIdentity,
    function: FunctionIndex,
}

impl LinkedActorMethodTarget {
    pub fn new(
        index: ActorMethodIndex,
        actor_abi_identity: ActorAbiIdentity,
        method_identity: ActorMethodIdentity,
        function: FunctionIndex,
    ) -> Self {
        Self {
            index,
            actor_abi_identity,
            method_identity,
            function,
        }
    }

    pub const fn index(&self) -> ActorMethodIndex {
        self.index
    }

    pub const fn actor_abi_identity(&self) -> &ActorAbiIdentity {
        &self.actor_abi_identity
    }

    pub const fn method_identity(&self) -> &ActorMethodIdentity {
        &self.method_identity
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }
}
