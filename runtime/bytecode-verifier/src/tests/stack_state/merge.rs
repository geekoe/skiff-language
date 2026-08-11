use skiff_artifact_model::{Opcode, ParamModeIr, TypeRefIr};
use skiff_runtime_linked_bytecode::FunctionIndex;

use super::{
    assert_instruction_violation, branch_fixture, hint, live, moved, prove, spec, uninitialized,
    WithHints,
};
use crate::{
    tests::fixtures::stack_state::{branch, fixture, plain, slot_instruction},
    VerificationError, VerificationLocation, VerificationObligation,
};

#[test]
fn merge_rejects_different_stack_heights() {
    let fixture = branch_fixture(
        vec![TypeRefIr::builtin("bool"), TypeRefIr::builtin("string")],
        vec![0, 1],
        vec![
            slot_instruction(Opcode::TakeSlot, 0),
            branch(Opcode::JumpIfTrue, 4),
            slot_instruction(Opcode::LoadSlot, 1),
            branch(Opcode::Jump, 5),
            branch(Opcode::Jump, 5),
            plain(Opcode::Return),
        ],
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 5);
}

#[test]
fn merge_rejects_different_stack_classes() {
    let fixture = branch_fixture(
        vec![
            TypeRefIr::builtin("bool"),
            TypeRefIr::builtin("string"),
            TypeRefIr::builtin("bytes"),
        ],
        vec![0, 1, 2],
        vec![
            slot_instruction(Opcode::TakeSlot, 0),
            branch(Opcode::JumpIfTrue, 4),
            slot_instruction(Opcode::LoadSlot, 1),
            branch(Opcode::Jump, 5),
            slot_instruction(Opcode::LoadSlot, 2),
            plain(Opcode::Pop),
            plain(Opcode::Return),
        ],
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 5);
}

#[test]
fn merge_rejects_different_slot_liveness() {
    let fixture = branch_fixture(
        vec![TypeRefIr::builtin("bool"), TypeRefIr::builtin("string")],
        vec![0, 1],
        vec![
            slot_instruction(Opcode::TakeSlot, 0),
            branch(Opcode::JumpIfTrue, 5),
            slot_instruction(Opcode::TakeSlot, 1),
            plain(Opcode::Pop),
            branch(Opcode::Jump, 6),
            branch(Opcode::Jump, 6),
            plain(Opcode::Return),
        ],
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 6);
}

#[test]
fn merge_accepts_uninitialized_and_live_before_converging_write() {
    let fixture = fixture(
        vec![
            TypeRefIr::builtin("bool"),
            TypeRefIr::builtin("string"),
            TypeRefIr::builtin("string"),
        ],
        spec(
            vec![0, 1, 2],
            vec![(0, ParamModeIr::Value), (2, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                branch(Opcode::JumpIfTrue, 5),
                slot_instruction(Opcode::LoadSlot, 2),
                slot_instruction(Opcode::StoreSlot, 1),
                branch(Opcode::Jump, 6),
                branch(Opcode::Jump, 6),
                slot_instruction(Opcode::LoadSlot, 2),
                slot_instruction(Opcode::StoreSlot, 1),
                plain(Opcode::Return),
            ],
            1,
        )
        .with_hints(vec![
            hint(&[], &[live(0), uninitialized(), live(2)]),
            hint(&[0], &[moved(), uninitialized(), live(2)]),
            hint(&[], &[moved(), uninitialized(), live(2)]),
            hint(&[2], &[moved(), uninitialized(), live(2)]),
            hint(&[], &[moved(), live(1), live(2)]),
            hint(&[], &[moved(), uninitialized(), live(2)]),
            hint(&[], &[moved(), uninitialized(), live(2)]),
            hint(&[2], &[moved(), uninitialized(), live(2)]),
            hint(&[], &[moved(), live(1), live(2)]),
        ]),
    );
    prove(&fixture)
        .expect("uninitialized entry and live backedge must merge to uninitialized before write");
}

#[test]
fn merge_accepts_distinct_type_indices_in_one_semantic_class() {
    let fixture = fixture(
        vec![
            TypeRefIr::builtin("bool"),
            TypeRefIr::builtin("string"),
            TypeRefIr::builtin("string"),
        ],
        spec(
            vec![0, 1, 2],
            vec![
                (0, ParamModeIr::Value),
                (1, ParamModeIr::Value),
                (2, ParamModeIr::Value),
            ],
            Vec::new(),
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                branch(Opcode::JumpIfTrue, 4),
                slot_instruction(Opcode::LoadSlot, 1),
                branch(Opcode::Jump, 5),
                slot_instruction(Opcode::LoadSlot, 2),
                plain(Opcode::Pop),
                plain(Opcode::Return),
            ],
            1,
        )
        .with_hints(vec![
            hint(&[], &[live(0), live(1), live(2)]),
            hint(&[0], &[moved(), live(1), live(2)]),
            hint(&[], &[moved(), live(1), live(2)]),
            hint(&[1], &[moved(), live(1), live(2)]),
            hint(&[], &[moved(), live(1), live(2)]),
            hint(&[2], &[moved(), live(1), live(2)]),
            hint(&[], &[moved(), live(1), live(2)]),
        ]),
    );
    prove(&fixture).expect("semantic-class merge canonicalizes through P2");
}

#[test]
fn independently_computed_depth_rejects_a_low_declaration() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::LoadSlot, 0),
                plain(Opcode::Pop),
                plain(Opcode::Return),
            ],
            0,
        ),
    );
    assert!(matches!(
        prove(&fixture),
        Err(VerificationError::SemanticViolation {
            obligation: VerificationObligation::StackAndSlotState,
            location: VerificationLocation::Function { function },
            ..
        }) if function == FunctionIndex::new(0)
    ));
}

#[test]
fn forged_stack_map_hint_is_only_a_post_fixpoint_check() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                plain(Opcode::Pop),
                plain(Opcode::Return),
            ],
            1,
        )
        .with_hints(vec![
            hint(&[], &[live(0)]),
            hint(&[], &[live(0)]),
            hint(&[], &[moved()]),
        ]),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 1);
}
