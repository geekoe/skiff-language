use skiff_artifact_model::{ContractOperationId, GatewayEntryKey};

use crate::{FunctionIndex, LinkedCallableSignature};

/// Unverified external operation entry facts. The verifier must independently
/// compare the operation contract, concrete signature and referenced function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedOperationEntry {
    contract_operation_id: ContractOperationId,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

impl LinkedOperationEntry {
    pub fn new(
        contract_operation_id: ContractOperationId,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            contract_operation_id,
            function,
            signature,
        }
    }

    pub const fn contract_operation_id(&self) -> &ContractOperationId {
        &self.contract_operation_id
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

/// Unverified gateway entry facts. The verifier must independently compare
/// the gateway adapter contract, concrete signature and referenced function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedGatewayEntry {
    gateway_entry_key: GatewayEntryKey,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

impl LinkedGatewayEntry {
    pub fn new(
        gateway_entry_key: GatewayEntryKey,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            gateway_entry_key,
            function,
            signature,
        }
    }

    pub const fn gateway_entry_key(&self) -> &GatewayEntryKey {
        &self.gateway_entry_key
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}
