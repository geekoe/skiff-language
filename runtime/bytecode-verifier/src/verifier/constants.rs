use std::fmt;

use skiff_runtime_linked_bytecode::{CandidateTable, ConstantIndex, LinkedBytecodeCandidate};
use skiff_runtime_loader::HydratedDeploymentBytecode;
use skiff_runtime_model::vm_value::ValueSlot;

use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Immutable values materialized from the verified frozen constant graph.
///
/// Fields and construction are private to the verifier. Every future non-empty
/// value is represented by a [`ValueSlot`] of kind `ConstRef`; that handle is
/// meaningful only together with the same pinned
/// [`crate::VerifiedLinkedBytecodeImage`]. The current conservative checkpoint can
/// construct only an empty heap. This type never accepts values or handles
/// supplied by a caller.
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
    pub(super) values: Box<[ValueSlot]>,
    pub(super) _seal: VerifiedConstantHeapSeal,
}

#[derive(Debug)]
pub(super) struct VerifiedConstantHeapSeal;

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

pub(crate) fn prove_and_build_empty_constant_heap(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
) -> Result<VerifiedConstantHeap, VerificationError> {
    let source_has_constant_authority = hydrated.packages().values().any(|package| {
        let view = package.bytecode().view();
        !view.pools().constants.is_empty()
            || !view.constant_roots().is_empty()
            || !view.frozen_constant_graph().nodes.is_empty()
    });
    let candidate_location = first_candidate_constant_location(candidate);

    match (source_has_constant_authority, candidate_location) {
        (false, None) => Ok(VerifiedConstantHeap {
            values: Box::new([]),
            _seal: VerifiedConstantHeapSeal,
        }),
        (true, Some(location)) => Err(VerificationError::ProofUnavailable {
            obligation: VerificationObligation::FrozenConstantSafety,
            location,
        }),
        (true, None) => Err(frozen_constant_violation(
            VerificationLocation::Image,
            "candidate erased non-empty frozen constant authority from the exact hydration",
        )),
        (false, Some(location)) => Err(frozen_constant_violation(
            location,
            "candidate introduced frozen constant authority absent from the exact hydration",
        )),
    }
}

fn first_candidate_constant_location(
    candidate: &LinkedBytecodeCandidate,
) -> Option<VerificationLocation> {
    candidate
        .constants()
        .first()
        .map(|constant| VerificationLocation::Table {
            table: CandidateTable::Constants,
            row: constant.index().get(),
        })
        .or_else(|| {
            candidate
                .constant_roots()
                .first()
                .map(|_| VerificationLocation::Table {
                    table: CandidateTable::ConstantRoots,
                    row: 0,
                })
        })
        .or_else(|| {
            candidate
                .frozen_constant_nodes()
                .first()
                .map(|node| VerificationLocation::Table {
                    table: CandidateTable::FrozenConstantNodes,
                    row: node.index().get(),
                })
        })
}

fn frozen_constant_violation(
    location: VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::FrozenConstantSafety,
        location,
        detail: detail.into(),
    }
}
