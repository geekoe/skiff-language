use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, ContractOperationId,
    PackageBuildId, ServiceProtocolIdentity, ServiceRequirementKey, ServiceSymbolRef,
};

use crate::{
    ActorCreateIndex, ActorMethodIndex, FunctionIndex, LinkedCallableSignature,
    ServiceOperationIndex, SpecializationKey,
};

/// Exact concrete local or package-direct target. The key and function remain
/// visible so the verifier can independently compare specialization and code.
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
    expected_protocol_identity: ServiceProtocolIdentity,
    signature: LinkedCallableSignature,
}

impl LinkedServiceOperationTarget {
    pub fn new(
        index: ServiceOperationIndex,
        service_requirement_key: ServiceRequirementKey,
        contract_operation_id: ContractOperationId,
        expected_protocol_identity: ServiceProtocolIdentity,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            index,
            service_requirement_key,
            contract_operation_id,
            expected_protocol_identity,
            signature,
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

    pub const fn expected_protocol_identity(&self) -> &ServiceProtocolIdentity {
        &self.expected_protocol_identity
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

/// Actor entry target inside the exact owner image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedActorMethodTarget {
    index: ActorMethodIndex,
    owner_package_build_id: PackageBuildId,
    actor: ServiceSymbolRef,
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: ActorImplementationIdentity,
    method_identity: ActorMethodIdentity,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

impl LinkedActorMethodTarget {
    pub fn new(
        index: ActorMethodIndex,
        owner_package_build_id: PackageBuildId,
        actor: ServiceSymbolRef,
        actor_abi_identity: ActorAbiIdentity,
        actor_implementation_identity: ActorImplementationIdentity,
        method_identity: ActorMethodIdentity,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            index,
            owner_package_build_id,
            actor,
            actor_abi_identity,
            actor_implementation_identity,
            method_identity,
            function,
            signature,
        }
    }

    pub const fn index(&self) -> ActorMethodIndex {
        self.index
    }

    pub const fn owner_package_build_id(&self) -> &PackageBuildId {
        &self.owner_package_build_id
    }

    pub const fn actor(&self) -> &ServiceSymbolRef {
        &self.actor
    }

    pub const fn actor_abi_identity(&self) -> &ActorAbiIdentity {
        &self.actor_abi_identity
    }

    pub const fn actor_implementation_identity(&self) -> &ActorImplementationIdentity {
        &self.actor_implementation_identity
    }

    pub const fn method_identity(&self) -> &ActorMethodIdentity {
        &self.method_identity
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

/// Exact actor create target inside the owning package build. Create remains
/// a distinct typed table from public methods so its role is never inferred
/// from textual or ABI identity shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedActorCreateTarget {
    index: ActorCreateIndex,
    owner_package_build_id: PackageBuildId,
    actor: ServiceSymbolRef,
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: ActorImplementationIdentity,
    create_identity: ActorMethodIdentity,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

impl LinkedActorCreateTarget {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: ActorCreateIndex,
        owner_package_build_id: PackageBuildId,
        actor: ServiceSymbolRef,
        actor_abi_identity: ActorAbiIdentity,
        actor_implementation_identity: ActorImplementationIdentity,
        create_identity: ActorMethodIdentity,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            index,
            owner_package_build_id,
            actor,
            actor_abi_identity,
            actor_implementation_identity,
            create_identity,
            function,
            signature,
        }
    }

    pub const fn index(&self) -> ActorCreateIndex {
        self.index
    }

    pub const fn owner_package_build_id(&self) -> &PackageBuildId {
        &self.owner_package_build_id
    }

    pub const fn actor(&self) -> &ServiceSymbolRef {
        &self.actor
    }

    pub const fn actor_abi_identity(&self) -> &ActorAbiIdentity {
        &self.actor_abi_identity
    }

    pub const fn actor_implementation_identity(&self) -> &ActorImplementationIdentity {
        &self.actor_implementation_identity
    }

    pub const fn create_identity(&self) -> &ActorMethodIdentity {
        &self.create_identity
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}
