use std::{fmt, sync::Arc};

use skiff_artifact_model::{ContractOperationId, GatewayEntryIdentity, GatewayEntryKey};
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, DeploymentProgramEntry};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedCallableSignature, LinkedGatewayCallableRole,
};

use super::VerifiedLinkedBytecodeImage;

/// Typed identity of one verified code entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifiedCodeEntryKind {
    Operation {
        contract_operation_id: ContractOperationId,
    },
    Gateway {
        gateway_entry_key: GatewayEntryKey,
        gateway_entry_identity: GatewayEntryIdentity,
        role: LinkedGatewayCallableRole,
    },
}

/// An unforgeable code entry pinned to the exact verified program allocation.
///
/// Construction is private to [`VerifiedLinkedBytecodeImage::operation_entry`]
/// and [`VerifiedLinkedBytecodeImage::gateway_entry`]. A raw function index or
/// equal-but-distinct program allocation cannot be turned into this type.
///
/// ```compile_fail
/// use std::sync::Arc;
/// use skiff_runtime_bytecode_verifier::{
///     VerifiedCodeEntry, VerifiedLinkedBytecodeImage,
/// };
/// use skiff_runtime_linked_bytecode::FunctionIndex;
///
/// fn forge(image: Arc<VerifiedLinkedBytecodeImage>) -> VerifiedCodeEntry {
///     VerifiedCodeEntry::new(image, FunctionIndex::new(0))
/// }
/// ```
#[derive(Debug)]
pub struct VerifiedCodeEntry {
    pub(super) program: Arc<VerifiedLinkedBytecodeImage>,
    pub(super) kind: VerifiedCodeEntryKind,
    pub(super) function: FunctionIndex,
    pub(super) signature: LinkedCallableSignature,
}

impl VerifiedCodeEntry {
    /// Returns the exact program allocation pinned by this entry.
    pub const fn image(&self) -> &Arc<VerifiedLinkedBytecodeImage> {
        &self.program
    }

    pub const fn kind(&self) -> &VerifiedCodeEntryKind {
        &self.kind
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

impl DeploymentProgramEntry<VerifiedLinkedBytecodeImage> for VerifiedCodeEntry {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        self.program.owner()
    }

    fn program(&self) -> &Arc<VerifiedLinkedBytecodeImage> {
        &self.program
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeEntryLookupError {
    OperationNotFound {
        contract_operation_id: ContractOperationId,
    },
    GatewayNotFound {
        gateway_entry_key: GatewayEntryKey,
    },
    GatewayCallableNotFound {
        gateway_entry_key: GatewayEntryKey,
        gateway_entry_identity: GatewayEntryIdentity,
        role: LinkedGatewayCallableRole,
    },
}

impl fmt::Display for CodeEntryLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OperationNotFound {
                contract_operation_id,
            } => write!(
                formatter,
                "verified operation entry {contract_operation_id} does not exist"
            ),
            Self::GatewayNotFound { gateway_entry_key } => write!(
                formatter,
                "verified gateway entry {gateway_entry_key} does not exist"
            ),
            Self::GatewayCallableNotFound {
                gateway_entry_key,
                gateway_entry_identity,
                role,
            } => write!(
                formatter,
                "verified gateway entry {gateway_entry_key}/{gateway_entry_identity} has no {role:?} callable"
            ),
        }
    }
}

impl std::error::Error for CodeEntryLookupError {}
