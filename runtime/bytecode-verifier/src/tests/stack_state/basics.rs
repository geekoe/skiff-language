use skiff_artifact_model::{Opcode, ParamModeIr, TypeRefIr};
use skiff_runtime_linked_bytecode::FunctionIndex;

use super::{
    assert_instruction_violation, hint, live, moved, prove, spec, uninitialized, WithHints,
};
use crate::{
    tests::fixtures::stack_state::{
        constant_fixture, fixture, plain, slot_instruction, stream_of_string, two_slot_instruction,
        FunctionSpec,
    },
    VerificationError, VerificationLocation, VerificationObligation,
};

#[test]
fn straight_return_and_exact_bool_not_are_proven() {
    let straight = fixture(
        Vec::new(),
        spec(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![plain(Opcode::Return)],
            0,
        )
        .with_hints(vec![hint(&[], &[])]),
    );
    prove(&straight).expect("straight empty return has a complete stack proof");

    let boolean = fixture(
        vec![TypeRefIr::builtin("bool")],
        spec(
            vec![0],
            vec![(0, ParamModeIr::Value)],
            vec![0],
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                plain(Opcode::Not),
                plain(Opcode::Return),
            ],
            1,
        )
        .with_hints(vec![
            hint(&[], &[live(0)]),
            hint(&[0], &[moved()]),
            hint(&[0], &[moved()]),
        ]),
    );
    prove(&boolean).expect("Not consumes and produces the independently proved bool class");
}

#[test]
fn canonical_slot_and_stack_operations_share_one_transfer_engine() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        FunctionSpec {
            slots: vec![0, 0, 0],
            parameters: vec![(0, ParamModeIr::Value)],
            writable: vec![1],
            results: Vec::new(),
            instructions: vec![
                two_slot_instruction(Opcode::CopySlot, 0, 1),
                two_slot_instruction(Opcode::MoveSlot, 1, 2),
                slot_instruction(Opcode::LoadSlot, 0),
                plain(Opcode::Dup),
                slot_instruction(Opcode::StoreSlot, 1),
                plain(Opcode::Pop),
                slot_instruction(Opcode::Drop, 2),
                plain(Opcode::Return),
            ],
            declared_max: 2,
            hints: Some(vec![
                hint(&[], &[live(0), uninitialized(), uninitialized()]),
                hint(&[], &[live(0), live(0), uninitialized()]),
                hint(&[], &[live(0), moved(), live(0)]),
                hint(&[0], &[live(0), moved(), live(0)]),
                hint(&[0, 0], &[live(0), moved(), live(0)]),
                hint(&[0], &[live(0), live(0), live(0)]),
                hint(&[], &[live(0), live(0), live(0)]),
                hint(&[], &[live(0), live(0), moved()]),
            ]),
        },
    );
    prove(&fixture).expect("canonical basic slot and stack contracts are supported");
}

#[test]
fn isolated_const_lifecycle_gate_accepts_string_and_rejects_stream() {
    prove(&constant_fixture(TypeRefIr::builtin("string")))
        .expect("assumed-bound Ordinary SnapshotShare constant is materializable");
    assert_instruction_violation(prove(&constant_fixture(stream_of_string())).unwrap_err(), 0);
}

#[test]
fn uninitialized_and_moved_slot_reads_are_rejected() {
    let uninitialized_fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0],
            Vec::new(),
            Vec::new(),
            vec![slot_instruction(Opcode::LoadSlot, 0), plain(Opcode::Return)],
            1,
        ),
    );
    assert_instruction_violation(prove(&uninitialized_fixture).unwrap_err(), 0);

    let moved_fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                plain(Opcode::Pop),
                slot_instruction(Opcode::LoadSlot, 0),
                plain(Opcode::Return),
            ],
            1,
        ),
    );
    assert_instruction_violation(prove(&moved_fixture).unwrap_err(), 2);
}

#[test]
fn slot_write_requires_the_exact_semantic_class() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("bytes")],
        spec(
            vec![0, 1],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                slot_instruction(Opcode::StoreSlot, 1),
                plain(Opcode::Return),
            ],
            1,
        ),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 1);
}

#[test]
fn copy_of_non_shareable_value_is_rejected() {
    let fixture = fixture(
        vec![stream_of_string()],
        spec(
            vec![0, 0],
            vec![(0, ParamModeIr::Value)],
            Vec::new(),
            vec![
                two_slot_instruction(Opcode::CopySlot, 0, 1),
                plain(Opcode::Return),
            ],
            0,
        ),
    );
    assert_instruction_violation(prove(&fixture).unwrap_err(), 0);
}

#[test]
fn comparison_pair_is_proven_when_bool_output_is_concrete() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("bool")],
        spec(
            vec![0, 0],
            vec![(0, ParamModeIr::Value), (1, ParamModeIr::Value)],
            vec![1],
            vec![
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::LoadSlot, 1),
                plain(Opcode::Equal),
                plain(Opcode::Return),
            ],
            2,
        )
        .with_hints(vec![
            hint(&[], &[live(0), live(0)]),
            hint(&[0], &[live(0), live(0)]),
            hint(&[0, 0], &[live(0), live(0)]),
            hint(&[1], &[live(0), live(0)]),
        ]),
    );
    prove(&fixture).expect("ComparablePair and bool output are supported");
}

#[test]
fn inout_frame_entry_remains_unavailable_without_loan_proof() {
    let fixture = fixture(
        vec![TypeRefIr::builtin("string")],
        spec(
            vec![0],
            vec![(0, ParamModeIr::InOut)],
            Vec::new(),
            vec![plain(Opcode::Return)],
            0,
        ),
    );
    assert_eq!(
        prove(&fixture).unwrap_err(),
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::StackAndSlotState,
            location: VerificationLocation::Function {
                function: FunctionIndex::new(0),
            },
        }
    );
}
