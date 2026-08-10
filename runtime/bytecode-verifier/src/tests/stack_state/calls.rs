use skiff_artifact_model::{Opcode, TypeRefIr};

use super::{assert_instruction_violation, prove, spec};
use crate::tests::fixtures::stack_state::{
    call_argument_mismatch_fixture, call_result_fixture, fixture, plain,
};

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
