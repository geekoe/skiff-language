use skiff_artifact_model::StatementChargeKind;
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

use super::fixtures::{
    generous_limits, loader_backed_tail_case, TailMatrixCase, TailMatrixFixture,
};
use crate::{
    verify, VerificationError, VerificationLocation, VerificationObligation,
    VerifiedLinkedBytecodeImage,
};

#[test]
fn public_verify_accepts_live_cleanup_and_schedules_one_tail_hop() {
    let (image, tail_instruction) = verify_case(TailMatrixCase::LiveCleanup)
        .expect("an exact tail replaces the caller after dropping its remaining live slot");

    assert_one_tail_hop(&image, tail_instruction);
}

#[test]
fn public_verify_accepts_moved_and_uninitialized_slots_without_drop() {
    verify_case(TailMatrixCase::MovedAndUninitialized)
        .expect("moved and never-initialized caller slots require no tail cleanup");
}

#[test]
fn public_verify_rejects_tail_stack_residue_at_the_tail_site() {
    assert_semantic_failure(
        verify_case(TailMatrixCase::StackResidue).unwrap_err(),
        VerificationObligation::TailCall,
        instruction_location(2),
        "residual values",
    );
}

#[test]
fn public_verify_rejects_tail_argument_concrete_class_corruption() {
    assert_semantic_failure(
        verify_case(TailMatrixCase::ArgumentClassCorruption).unwrap_err(),
        VerificationObligation::StackAndSlotState,
        instruction_location(1),
        "local-call argument 0",
    );
}

#[test]
fn public_verify_rejects_argument_plan_corruption_at_its_first_p2_position() {
    assert_semantic_failure(
        verify_case(TailMatrixCase::ArgumentPlanCorruption).unwrap_err(),
        VerificationObligation::ValueTransferAndDrop,
        instruction_location(1),
        "stack-before value ordinal 0",
    );
}

#[test]
fn public_verify_rejects_result_plan_corruption_at_its_first_p2_position() {
    assert_semantic_failure(
        verify_case(TailMatrixCase::ResultPlanCorruption).unwrap_err(),
        VerificationObligation::ValueTransferAndDrop,
        VerificationLocation::Function {
            function: FunctionIndex::new(1),
        },
        "frame result ordinal 0",
    );
}

fn verify_case(
    case: TailMatrixCase,
) -> Result<(VerifiedLinkedBytecodeImage, InstructionIndex), VerificationError> {
    let TailMatrixFixture {
        hydrated,
        candidate,
        tail_instruction,
    } = loader_backed_tail_case(case);
    verify(hydrated, candidate, &generous_limits()).map(|image| (image, tail_instruction))
}

fn assert_one_tail_hop(image: &VerifiedLinkedBytecodeImage, instruction: InstructionIndex) {
    let events = image
        .statement_schedule()
        .events_at(FunctionIndex::new(0), instruction)
        .expect("the public verified image contains the dense tail statement site");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].charge_kind(), StatementChargeKind::TailHop);
}

fn assert_semantic_failure(
    error: VerificationError,
    obligation: VerificationObligation,
    location: VerificationLocation,
    detail: &str,
) {
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: actual_obligation,
            location: actual_location,
            detail: actual_detail,
        } if actual_obligation == obligation
            && actual_location == location
            && actual_detail.contains(detail)
    ));
}

const fn instruction_location(instruction: u32) -> VerificationLocation {
    VerificationLocation::Instruction {
        function: FunctionIndex::new(0),
        instruction: InstructionIndex::new(instruction),
    }
}
