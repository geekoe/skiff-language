use skiff_runtime_linked_bytecode::{CandidateTable, LinkedBytecodeCandidate, LinkedResumeSite};

use super::{ControlFlowFacts, ExactTargetAndCallFacts};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits,
    VerificationLocation, VerificationObligation,
};

/// Checks every pending-capable site against its unique resume descriptor.
pub(super) fn prove_resume_sites(
    candidate: &LinkedBytecodeCandidate,
    _concrete_values: &ConcreteValueFacts,
    _targets: &ExactTargetAndCallFacts,
    _control_flow: &ControlFlowFacts,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    prove_resume_table_is_empty(candidate.resume_sites())
}

fn prove_resume_table_is_empty(resume_sites: &[LinkedResumeSite]) -> Result<(), VerificationError> {
    let Some(resume) = resume_sites.first() else {
        return Ok(());
    };
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ResumeSite,
        location: VerificationLocation::Table {
            table: CandidateTable::ResumeSites,
            row: resume.index().get(),
        },
    })
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::ResumeErrorMode;
    use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, ResumeSiteIndex};

    use super::*;

    #[test]
    fn empty_resume_table_is_a_complete_zero_site_proof() {
        assert_eq!(prove_resume_table_is_empty(&[]), Ok(()));
    }

    #[test]
    fn first_nonempty_resume_row_remains_precisely_fail_closed() {
        let resume = LinkedResumeSite::new(
            ResumeSiteIndex::new(7),
            FunctionIndex::new(0),
            InstructionIndex::new(0),
            InstructionIndex::new(1),
            0,
            Box::new([]),
            Box::new([]),
            ResumeErrorMode::RaiseAtSite,
        )
        .unwrap();

        assert_eq!(
            prove_resume_table_is_empty(&[resume]),
            Err(VerificationError::ProofUnavailable {
                obligation: VerificationObligation::ResumeSite,
                location: VerificationLocation::Table {
                    table: CandidateTable::ResumeSites,
                    row: 7,
                },
            })
        );
    }
}
