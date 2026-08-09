use std::{fmt, sync::Arc};

use skiff_artifact_model::{ContractOperationId, GatewayEntryIdentity, GatewayEntryKey};
use skiff_runtime_deployment_image::{
    DeploymentOwnerIdentity, DeploymentProgramEntry, DeploymentProgramFacts, ServiceDependencySlot,
};
use skiff_runtime_linked_bytecode::{
    ConstantIndex, FunctionIndex, LinkedBytecodeCandidate, LinkedCallableSignature, LinkedFunction,
    LinkedGatewayCallableRole,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;
use skiff_runtime_model::vm_value::ValueSlot;

use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// Opaque proof token stored in every verified image.
///
/// The type and the image fields are private to this module, which also owns
/// [`verify`]. No sibling module, feature, or downstream crate can mint it.
#[derive(Debug)]
struct VerificationSeal;

#[derive(Debug)]
struct SealedDeploymentFacts {
    owner: DeploymentOwnerIdentity,
    dependency_slots: Box<[ServiceDependencySlot]>,
    constant_heap: VerifiedConstantHeap,
    seal: VerificationSeal,
}

/// A linked candidate sealed against one exact, opaque deployment hydration.
///
/// Fields are private and there is no `Default`, unchecked/test-support
/// constructor, `From<LinkedBytecodeCandidate>`, mutable candidate accessor,
/// or `DerefMut` implementation. The exact owner is always derived from the
/// consumed hydration; [`verify`] has no caller-supplied owner parameter.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
/// use skiff_runtime_loader::HydratedDeploymentBytecode;
///
/// fn extract_unverified_hydration(
///     image: &VerifiedLinkedBytecodeImage,
/// ) -> &HydratedDeploymentBytecode {
///     &image._hydrated
/// }
/// ```
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
/// use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
///
/// fn bypass(candidate: LinkedBytecodeCandidate) -> VerifiedLinkedBytecodeImage {
///     candidate.into()
/// }
/// ```
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
///
/// let _ = VerifiedLinkedBytecodeImage::default();
/// ```
#[derive(Debug)]
pub struct VerifiedLinkedBytecodeImage {
    _hydrated: HydratedDeploymentBytecode,
    candidate: LinkedBytecodeCandidate,
    owner: DeploymentOwnerIdentity,
    dependency_slots: Box<[ServiceDependencySlot]>,
    constant_heap: VerifiedConstantHeap,
    _seal: VerificationSeal,
}

impl VerifiedLinkedBytecodeImage {
    /// Returns the exact deployment owner derived from the hydration.
    pub const fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    /// Returns the complete verified candidate as a shared, read-only view.
    pub const fn candidate(&self) -> &LinkedBytecodeCandidate {
        &self.candidate
    }

    /// Returns the verified concrete functions as a shared, read-only view.
    pub fn functions(&self) -> &[LinkedFunction] {
        self.candidate.functions()
    }

    /// Returns canonical symbolic service facts derived from the hydration.
    pub fn dependency_slots(&self) -> &[ServiceDependencySlot] {
        &self.dependency_slots
    }

    /// Returns the immutable constant heap materialized by this verifier.
    pub const fn constant_heap(&self) -> &VerifiedConstantHeap {
        &self.constant_heap
    }

    /// Resolves a verified service operation entry while pinning this exact
    /// program allocation.
    pub fn operation_entry(
        self: &Arc<Self>,
        operation: &ContractOperationId,
    ) -> Result<VerifiedCodeEntry, CodeEntryLookupError> {
        let entry = self
            .candidate
            .operation_entries()
            .binary_search_by(|entry| entry.contract_operation_id().cmp(operation))
            .map(|index| &self.candidate.operation_entries()[index])
            .map_err(|_| CodeEntryLookupError::OperationNotFound {
                contract_operation_id: operation.clone(),
            })?;
        let function = entry.function();
        let signature = entry.signature().clone();

        Ok(VerifiedCodeEntry {
            program: Arc::clone(self),
            kind: VerifiedCodeEntryKind::Operation {
                contract_operation_id: operation.clone(),
            },
            function,
            signature,
        })
    }

    /// Resolves one typed callable role by its owner-local gateway key and pins
    /// this program allocation. The returned kind carries the identity proved
    /// from the hydration; callers never supply that fact.
    pub fn gateway_entry(
        self: &Arc<Self>,
        key: &GatewayEntryKey,
        role: LinkedGatewayCallableRole,
    ) -> Result<VerifiedCodeEntry, CodeEntryLookupError> {
        let entry = self
            .candidate
            .gateway_entries()
            .binary_search_by(|entry| entry.gateway_entry_key().cmp(key))
            .map(|index| &self.candidate.gateway_entries()[index])
            .map_err(|_| CodeEntryLookupError::GatewayNotFound {
                gateway_entry_key: key.clone(),
            })?;
        let callable =
            entry
                .callable(role)
                .ok_or_else(|| CodeEntryLookupError::GatewayCallableNotFound {
                    gateway_entry_key: key.clone(),
                    gateway_entry_identity: entry.gateway_entry_identity().clone(),
                    role,
                })?;
        let function = callable.function();
        let signature = callable.signature().clone();

        Ok(VerifiedCodeEntry {
            program: Arc::clone(self),
            kind: VerifiedCodeEntryKind::Gateway {
                gateway_entry_key: key.clone(),
                gateway_entry_identity: entry.gateway_entry_identity().clone(),
                role,
            },
            function,
            signature,
        })
    }
}

impl DeploymentProgramFacts for VerifiedLinkedBytecodeImage {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    fn dependency_slots(&self) -> &[ServiceDependencySlot] {
        &self.dependency_slots
    }
}

/// Immutable values materialized from the verified frozen constant graph.
///
/// Fields and construction are private to the verifier. An aggregate value is
/// represented by a [`ValueSlot`] of kind `ConstRef`; that handle is meaningful
/// only together with the same pinned [`VerifiedLinkedBytecodeImage`]. Scalar
/// immediates may be returned directly. This type never accepts values or
/// handles supplied by a caller.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedConstantHeap;
/// use skiff_runtime_model::vm_value::ValueSlot;
///
/// fn extract_values(heap: &VerifiedConstantHeap) -> &[ValueSlot] {
///     &heap.values
/// }
/// ```
pub struct VerifiedConstantHeap {
    values: Box<[ValueSlot]>,
    _seal: VerifiedConstantHeapSeal,
}

#[derive(Debug)]
struct VerifiedConstantHeapSeal;

impl fmt::Debug for VerifiedConstantHeap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedConstantHeap")
            .field("len", &self.values.len())
            .finish_non_exhaustive()
    }
}

impl VerifiedConstantHeap {
    /// Returns one verified constant value by its image-local index.
    pub fn get(&self, index: ConstantIndex) -> Option<ValueSlot> {
        let index = usize::try_from(index.get()).ok()?;
        self.values.get(index).copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

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
    program: Arc<VerifiedLinkedBytecodeImage>,
    kind: VerifiedCodeEntryKind,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
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

/// Independently verifies and seals one exact deployment hydration and linked
/// candidate as a single immutable program.
///
/// Both inputs are consumed so a successful result owns the exact facts that
/// were cross-checked. At the C2 interface checkpoint the complete hydration
/// correspondence and semantic proofs are intentionally unavailable, so every
/// input returns [`VerificationError::ProofUnavailable`] and no seal is minted.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::{verify, VerificationLimits};
/// use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
///
/// fn candidate_only_bypass(
///     candidate: LinkedBytecodeCandidate,
///     limits: &VerificationLimits,
/// ) {
///     let _ = verify(candidate, limits);
/// }
/// ```
pub fn verify(
    hydrated: HydratedDeploymentBytecode,
    candidate: LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<VerifiedLinkedBytecodeImage, VerificationError> {
    let facts = establish_verification_seal(&hydrated, &candidate, limits)?;
    Ok(VerifiedLinkedBytecodeImage {
        _hydrated: hydrated,
        candidate,
        owner: facts.owner,
        dependency_slots: facts.dependency_slots,
        constant_heap: facts.constant_heap,
        _seal: facts.seal,
    })
}

fn establish_verification_seal(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<SealedDeploymentFacts, VerificationError> {
    prove_exact_hydration_binding(hydrated, candidate)?;
    prove_candidate_semantics(candidate, limits)?;
    let constant_heap = build_verified_constant_heap(candidate, limits)?;

    Ok(SealedDeploymentFacts {
        owner: DeploymentOwnerIdentity::new(hydrated.reference().clone()),
        dependency_slots: dependency_slots_from_hydration(hydrated)?,
        constant_heap,
        seal: VerificationSeal,
    })
}

fn prove_exact_hydration_binding(
    _hydrated: &HydratedDeploymentBytecode,
    _candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ExactHydrationBinding,
        location: VerificationLocation::Image,
    })
}

pub(super) fn prove_candidate_semantics(
    _candidate: &LinkedBytecodeCandidate,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ControlFlow,
        location: VerificationLocation::Image,
    })
}

pub(super) fn build_verified_constant_heap(
    _candidate: &LinkedBytecodeCandidate,
    _limits: &VerificationLimits,
) -> Result<VerifiedConstantHeap, VerificationError> {
    // Materialization must independently prove every immediate and ConstRef,
    // including same-image handle ownership. Returning an empty heap here for
    // an arbitrary candidate would create a second seal bypass, so this seam
    // remains explicitly unavailable until that proof lands.
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::FrozenConstantSafety,
        location: VerificationLocation::Image,
    })
}

fn dependency_slots_from_hydration(
    hydrated: &HydratedDeploymentBytecode,
) -> Result<Box<[ServiceDependencySlot]>, VerificationError> {
    hydrated
        .service_dependencies()
        .values()
        .map(|dependency| {
            ServiceDependencySlot::try_new(
                dependency.key().clone(),
                dependency.contract().clone(),
                dependency.used_operations().iter().cloned(),
            )
            .map_err(|error| VerificationError::SemanticViolation {
                obligation: VerificationObligation::ExactHydrationBinding,
                location: VerificationLocation::Image,
                detail: error.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}
