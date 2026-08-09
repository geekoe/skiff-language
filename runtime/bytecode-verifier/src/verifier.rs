use skiff_runtime_linked_bytecode::{CandidateTable, LinkedBytecodeCandidate, LinkedFunction};

use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// Opaque proof token stored in every verified image.
///
/// The type and the image fields are private to this module, which also owns
/// [`verify`]. No sibling module, feature, or downstream crate can mint it.
#[derive(Debug)]
struct VerificationSeal;

/// A linked candidate sealed by this crate's independent semantic verifier.
///
/// Fields are private and there is no `Default`, unchecked/test-support
/// constructor, `From<LinkedBytecodeCandidate>`, mutable candidate accessor,
/// or `DerefMut` implementation.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
/// use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
///
/// fn forge(candidate: LinkedBytecodeCandidate) -> VerifiedLinkedBytecodeImage {
///     VerifiedLinkedBytecodeImage { candidate }
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
    candidate: LinkedBytecodeCandidate,
    _seal: VerificationSeal,
}

impl VerifiedLinkedBytecodeImage {
    /// Returns the complete verified candidate as a shared, read-only view.
    pub const fn candidate(&self) -> &LinkedBytecodeCandidate {
        &self.candidate
    }

    /// Returns the verified concrete functions as a shared, read-only view.
    pub fn functions(&self) -> &[LinkedFunction] {
        self.candidate.functions()
    }
}

/// Independently verifies and seals one linked bytecode candidate.
///
/// The candidate is consumed so a successful result owns the exact immutable
/// data that was proved. At the C2 interface checkpoint, the full semantic
/// proofs are intentionally not implemented. The only accepted candidate is
/// the independently trivial empty image; every non-empty table returns
/// [`VerificationError::ProofUnavailable`] and no seal is minted.
pub fn verify(
    candidate: LinkedBytecodeCandidate,
    _limits: &VerificationLimits,
) -> Result<VerifiedLinkedBytecodeImage, VerificationError> {
    if let Some((obligation, location)) = first_unproven_obligation(&candidate) {
        return Err(VerificationError::ProofUnavailable {
            obligation,
            location,
        });
    }

    Ok(VerifiedLinkedBytecodeImage {
        candidate,
        _seal: VerificationSeal,
    })
}

fn first_unproven_obligation(
    candidate: &LinkedBytecodeCandidate,
) -> Option<(VerificationObligation, VerificationLocation)> {
    if let Some(function) = candidate.functions().first() {
        return Some((
            VerificationObligation::ControlFlow,
            VerificationLocation::Function {
                function: function.index(),
            },
        ));
    }

    let tables = [
        (
            CandidateTable::ExactLocalTargets,
            candidate.exact_local_targets().len(),
            VerificationObligation::ExactTargetAndCallPlan,
        ),
        (
            CandidateTable::ServiceOperations,
            candidate.service_operations().len(),
            VerificationObligation::ExactTargetAndCallPlan,
        ),
        (
            CandidateTable::ActorMethods,
            candidate.actor_methods().len(),
            VerificationObligation::ExactTargetAndCallPlan,
        ),
        (
            CandidateTable::InterfaceTables,
            candidate.interface_tables().len(),
            VerificationObligation::InterfaceSignature,
        ),
        (
            CandidateTable::SyntheticCallbacks,
            candidate.synthetic_callbacks().len(),
            VerificationObligation::CallbackCaptureAndEscape,
        ),
        (
            CandidateTable::HostEffectAdapters,
            candidate.host_effect_adapters().len(),
            VerificationObligation::EffectAndNoPending,
        ),
        (
            CandidateTable::Types,
            candidate.types().len(),
            VerificationObligation::ConcreteTypeAndShape,
        ),
        (
            CandidateTable::Shapes,
            candidate.shapes().len(),
            VerificationObligation::ConcreteTypeAndShape,
        ),
        (
            CandidateTable::Constants,
            candidate.constants().len(),
            VerificationObligation::FrozenConstantSafety,
        ),
        (
            CandidateTable::ResumeSites,
            candidate.resume_sites().len(),
            VerificationObligation::ResumeSite,
        ),
    ];

    tables
        .into_iter()
        .find(|(_, len, _)| *len != 0)
        .map(|(table, _, obligation)| (obligation, VerificationLocation::Table { table, row: 0 }))
}
