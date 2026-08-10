use skiff_artifact_model::{Opcode, TypeRefIr};

use super::{assert_instruction_violation, instruction_location, prove, spec};
use crate::tests::fixtures::stack_state::{
    call_argument_mismatch_fixture, call_result_fixture, fixture, plain,
    tail_argument_mismatch_fixture, tail_copy_live_fixture, tail_live_cleanup_fixture,
    tail_residue_fixture, tail_result_fixture,
};
use crate::{VerificationError, VerificationObligation};

#[test]
fn assumed_bound_local_call_argument_class_mismatch_is_rejected() {
    assert_instruction_violation(prove(&call_argument_mismatch_fixture()).unwrap_err(), 1);
}

#[test]
fn assumed_bound_local_call_results_propagate_into_caller_return() {
    prove(&call_result_fixture())
        .expect("exact call result types propagate by semantic class into Return");
}

#[test]
fn exact_tail_moves_arguments_and_cleans_up_live_caller_slots() {
    prove(&tail_live_cleanup_fixture())
        .expect("tail replacement must accept an exact live-slot drop plan");
    prove(&tail_copy_live_fixture())
        .expect("a shared argument may leave its exact caller slot live for cleanup");
}

#[test]
fn exact_tail_accepts_equivalent_result_coordinates_and_plans() {
    prove(&tail_result_fixture())
        .expect("equivalent concrete result coordinates have one exact return plan");
}

#[test]
fn exact_tail_rejects_argument_class_mismatch() {
    assert_instruction_violation(prove(&tail_argument_mismatch_fixture()).unwrap_err(), 1);
}

#[test]
fn exact_tail_rejects_stack_residue_below_arguments() {
    let error = prove(&tail_residue_fixture()).unwrap_err();
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::TailCall,
            location,
            detail,
        } if location == instruction_location(2) && detail.contains("residual values")
    ));
}

#[test]
fn return_requires_exact_result_arity() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            Vec::new(),
            Vec::new(),
            vec![0],
            vec![plain(Opcode::Return)],
            0,
        ),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 0);
}
