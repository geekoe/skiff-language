use skiff_artifact_model::AbiInterfaceId;

use crate::{InterfaceTableIndex, LinkedCallableSignature};

/// Canonical dynamic-interface method signature. It has no executable target:
/// carrier selection and exact local/remote/callback dispatch remain runtime
/// behavior proved by the verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedInterfaceMethod {
    method_slot: u32,
    signature: LinkedCallableSignature,
}

impl LinkedInterfaceMethod {
    pub fn new(method_slot: u32, signature: LinkedCallableSignature) -> Self {
        Self {
            method_slot,
            signature,
        }
    }

    pub const fn method_slot(&self) -> u32 {
        self.method_slot
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

/// Symbolic interface requirement table. No row can carry an exact executable
/// address, so a dynamic requirement cannot be mistaken for a local target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedInterfaceTable {
    index: InterfaceTableIndex,
    interface_identity: AbiInterfaceId,
    methods: Box<[LinkedInterfaceMethod]>,
}

impl LinkedInterfaceTable {
    pub fn new(
        index: InterfaceTableIndex,
        interface_identity: AbiInterfaceId,
        methods: Box<[LinkedInterfaceMethod]>,
    ) -> Self {
        Self {
            index,
            interface_identity,
            methods,
        }
    }

    pub const fn index(&self) -> InterfaceTableIndex {
        self.index
    }

    pub const fn interface_identity(&self) -> &AbiInterfaceId {
        &self.interface_identity
    }

    pub fn methods(&self) -> &[LinkedInterfaceMethod] {
        &self.methods
    }
}
