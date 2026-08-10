use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedBytecodeCandidate};

use super::{ExactTargetAndCallFacts, ExactTargetCoordinate};
use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Opaque authority that one exact local tail site completed the terminal
/// stack, frame-cleanup, region and loan proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VerifiedTailCallProof {
    function: FunctionIndex,
    instruction: InstructionIndex,
    target: FunctionIndex,
}

impl VerifiedTailCallProof {
    pub(in crate::control_flow) const fn new(
        function: FunctionIndex,
        instruction: InstructionIndex,
        target: FunctionIndex,
    ) -> Self {
        Self {
            function,
            instruction,
            target,
        }
    }

    pub(super) const fn target(&self) -> FunctionIndex {
        self.target
    }
}

/// Dense tail-call authorities. A token can only be minted by the independent
/// terminal transfer, never by reading the candidate stack-map sidecar.
#[derive(Debug)]
pub(super) struct VerifiedTailCallFacts {
    rows: Box<[Box<[Option<VerifiedTailCallProof>]>]>,
}

impl VerifiedTailCallFacts {
    pub(in crate::control_flow) fn try_from_dense(
        candidate: &LinkedBytecodeCandidate,
        targets: &ExactTargetAndCallFacts,
        rows: Vec<Vec<Option<VerifiedTailCallProof>>>,
    ) -> Result<Self, VerificationError> {
        if rows.len() != candidate.functions().len() {
            return Err(violation(
                VerificationLocation::Image,
                "tail-call facts are not dense with candidate functions",
            ));
        }

        for (function, row) in candidate.functions().iter().zip(&rows) {
            let function_location = VerificationLocation::Function {
                function: function.index(),
            };
            if row.len() != function.instructions().len() {
                return Err(violation(
                    function_location,
                    "tail-call facts are not dense with function instructions",
                ));
            }

            for (ordinal, (instruction, proof)) in
                function.instructions().iter().zip(row).enumerate()
            {
                let instruction_index =
                    u32::try_from(ordinal)
                        .map(InstructionIndex::new)
                        .map_err(|_| {
                            violation(
                                function_location,
                                "tail-call instruction ordinal does not fit u32",
                            )
                        })?;
                let location = VerificationLocation::Instruction {
                    function: function.index(),
                    instruction: instruction_index,
                };
                if instruction.opcode() != Opcode::TailCallLocal {
                    if proof.is_some() {
                        return Err(violation(
                            location,
                            "non-tail instruction carries a tail-call proof",
                        ));
                    }
                    continue;
                }

                let proof = proof.as_ref().ok_or_else(|| unavailable(location))?;
                let plan = targets
                    .call_plan(function.index(), instruction_index)
                    .ok_or_else(|| unavailable(location))?;
                let ExactTargetCoordinate::LocalFunction(target) = plan.target() else {
                    return Err(unavailable(location));
                };
                if proof.function != function.index()
                    || proof.instruction != instruction_index
                    || proof.target != target
                    || plan.call_site().function() != function.index()
                    || plan.call_site().instruction() != instruction_index
                {
                    return Err(violation(
                        location,
                        "tail-call proof disagrees with its exact call plan",
                    ));
                }
            }
        }

        Ok(Self {
            rows: rows
                .into_iter()
                .map(Vec::into_boxed_slice)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    }

    pub(super) fn proof(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Option<&VerifiedTailCallProof> {
        self.rows
            .get(function.get() as usize)
            .and_then(|row| row.get(instruction.get() as usize))
            .and_then(Option::as_ref)
    }
}

fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::TailCall,
        location,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::TailCall,
        location,
        detail: detail.into(),
    }
}
