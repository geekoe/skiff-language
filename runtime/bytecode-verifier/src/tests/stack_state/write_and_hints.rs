use skiff_artifact_model::{Opcode, ParamModeIr, TypeRefIr};

use super::{
    assert_instruction_violation, hint, live, moved, prove, spec, uninitialized, WithHints,
};
use crate::tests::fixtures::stack_state::{fixture, plain, slot_instruction, FunctionSpec};

#[test]
fn forged_hint_cannot_seed_an_underflowing_entry() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![plain(Opcode::Pop), plain(Opcode::Return)],
            1,
        )
        .with_hints(vec![hint(&[0], &[]), hint(&[], &[])]),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 0);
}

#[test]
fn immutable_local_allows_its_first_initialization() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0, 0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                slot_instruction(Opcode::StoreSlot, 1),
                plain(Opcode::Return),
            ],
            1,
        )
        .with_hints(vec![
            hint(&[], &[live(0), uninitialized()]),
            hint(&[0], &[moved(), uninitialized()]),
            hint(&[], &[moved(), live(0)]),
        ]),
    );
    prove(&fixture).expect("an uninitialized immutable local accepts one exact write");
}

#[test]
fn parameter_overwrite_is_rejected() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::StoreSlot, 0),
                plain(Opcode::Return),
            ],
            1,
        ),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 1);
}

#[test]
fn immutable_local_rewrite_is_rejected() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0, 0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::StoreSlot, 1),
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::StoreSlot, 1),
                plain(Opcode::Return),
            ],
            1,
        ),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 3);
}

#[test]
fn immutable_moved_local_reinitialization_is_rejected() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0, 0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::StoreSlot, 1),
                slot_instruction(Opcode::TakeSlot, 1),
                slot_instruction(Opcode::StoreSlot, 1),
                plain(Opcode::Return),
            ],
            1,
        ),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 3);
}

#[test]
fn writable_local_rewrite_is_accepted() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        FunctionSpec {
            slots: vec![0, 0],
            parameters: vec![(0, ParamModeIr::Value)],
            writable: vec![1],
            results: Vec::new(),
            instructions: vec![
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::StoreSlot, 1),
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::StoreSlot, 1),
                plain(Opcode::Return),
            ],
            declared_max: 1,
            hints: Some(vec![
                hint(&[], &[live(0), uninitialized()]),
                hint(&[0], &[live(0), uninitialized()]),
                hint(&[], &[live(0), live(0)]),
                hint(&[0], &[live(0), live(0)]),
                hint(&[], &[live(0), live(0)]),
            ]),
        },
    );
    prove(&fixture).expect("writable-local authority permits exact rewrite");
}

#[test]
fn return_rejects_residual_values_below_zero_results() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![slot_instruction(Opcode::TakeSlot, 0), plain(Opcode::Return)],
            1,
        ),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 1);
}

#[test]
fn conservative_depth_overdeclaration_is_accepted() {
    let fixture = fixture(
        Vec::new(),
        spec(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![plain(Opcode::Return)],
            5,
        )
        .with_hints(vec![hint(&[], &[])]),
    );
    prove(&fixture).expect("declared max is an upper bound, not an equality claim");
}
