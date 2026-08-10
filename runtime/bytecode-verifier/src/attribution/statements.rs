use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Statement rows remain untrusted metadata until a canonical, fingerprinted
/// charge contract can be recomputed independently from the linked program.
pub(crate) fn prove_statement_attribution(
    _candidate: &LinkedBytecodeCandidate,
) -> Result<(), VerificationError> {
    Err(statement_proof_unavailable())
}

fn statement_proof_unavailable() -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::SourceAndStatementAttribution,
        location: VerificationLocation::Image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_canonical_statement_authority_is_always_fail_closed() {
        assert_eq!(
            statement_proof_unavailable(),
            VerificationError::ProofUnavailable {
                obligation: VerificationObligation::SourceAndStatementAttribution,
                location: VerificationLocation::Image,
            }
        );
    }
}
