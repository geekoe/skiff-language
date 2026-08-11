use skiff_artifact_model::{Opcode, ParamModeIr, TypeRefIr, ValueDropPlan, ValueTransferPlan};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedInstruction, LinkedInstructionTarget, LinkedResolvedOperand, TypeIndex,
};

use super::{
    assert_instruction_violation, hint, live, moved, prove, spec, uninitialized, WithHints,
};
use crate::{
    tests::fixtures::stack_state::{
        constant_fixture, fixture, plain, rich_fixture, rich_shape_decl, slot_instruction,
        stream_of_string, two_slot_instruction, FunctionSpec,
    },
    VerificationError, VerificationLocation, VerificationObligation,
};

fn type_instruction(opcode: Opcode, ty: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([ty]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Type(TypeIndex::new(ty)),
        )]),
        0,
    )
    .expect("type test instruction is valid")
}

fn two_type_instruction(opcode: Opcode, key: u32, value: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([key, value]),
        Box::new([
            LinkedResolvedOperand::new(0, LinkedInstructionTarget::Type(TypeIndex::new(key))),
            LinkedResolvedOperand::new(1, LinkedInstructionTarget::Type(TypeIndex::new(value))),
        ]),
        0,
    )
    .expect("two-type test instruction is valid")
}

fn shape_instruction(opcode: Opcode, shape: u32, field_count: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([shape, field_count]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Shape(skiff_runtime_linked_bytecode::ShapeIndex::new(shape)),
        )]),
        0,
    )
    .expect("shape test instruction is valid")
}

fn field_instruction(opcode: Opcode, shape: u32, ordinal: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([shape, ordinal]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Shape(skiff_runtime_linked_bytecode::ShapeIndex::new(shape)),
        )]),
        0,
    )
    .expect("field test instruction is valid")
}

#[test]
fn record_and_dense_field_stack_transfer_is_proven() {
    let name_type = TypeRefIr::builtin("string");
    let record_type = TypeRefIr::Record {
        fields: [("name".to_string(), name_type.clone())].into(),
    };
    let shapes = vec![rich_shape_decl(
        0,
        vec![(
            "name",
            1,
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            },
        )],
    )];
    let fixture = rich_fixture(
        vec![record_type.clone(), name_type.clone()],
        shapes,
        spec(
            vec![1],
            vec![(0, ParamModeIr::Value)],
            vec![1],
            vec![
                slot_instruction(Opcode::TakeSlot, 0),
                shape_instruction(Opcode::NewRecord, 0, 1),
                field_instruction(Opcode::GetDenseField, 0, 0),
                plain(Opcode::Return),
            ],
            2,
        )
        .with_hints(vec![
            hint(&[], &[live(1)]),
            hint(&[1], &[moved()]),
            hint(&[0], &[moved()]),
            hint(&[1], &[moved()]),
        ]),
    );
    prove(&fixture).expect("record and dense-field stack transfer is proven");
}

#[test]
fn array_builder_freeze_get_and_len_are_proven() {
    let integer = TypeRefIr::builtin("integer");
    let array = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![integer.clone()],
    };
    let fixture = rich_fixture(
        vec![integer.clone(), array.clone()],
        Vec::new(),
        spec(
            vec![0, 0],
            vec![(0, ParamModeIr::Value), (1, ParamModeIr::Value)],
            vec![0],
            vec![
                type_instruction(Opcode::NewArrayBuilder, 0),
                slot_instruction(Opcode::LoadSlot, 0),
                plain(Opcode::ArrayBuilderPush),
                plain(Opcode::FreezeArray),
                slot_instruction(Opcode::LoadSlot, 1),
                plain(Opcode::ArrayGet),
                plain(Opcode::Return),
            ],
            2,
        )
        .with_hints(vec![
            hint(&[], &[live(0), live(0)]),
            hint(&[1], &[live(0), live(0)]),
            hint(&[1, 0], &[live(0), live(0)]),
            hint(&[1], &[live(0), live(0)]),
            hint(&[1], &[live(0), live(0)]),
            hint(&[1, 0], &[live(0), live(0)]),
            hint(&[0], &[live(0), live(0)]),
        ]),
    );
    prove(&fixture).expect("array builder, freeze, get, and return transfer are proven");
}

#[test]
fn map_builder_freeze_get_and_entry_are_proven() {
    let string = TypeRefIr::builtin("string");
    let integer = TypeRefIr::builtin("integer");
    let map = TypeRefIr::Builtin {
        name: "Map".to_string(),
        args: vec![string.clone(), integer.clone()],
    };
    let fixture = rich_fixture(
        vec![string.clone(), integer.clone(), map.clone()],
        Vec::new(),
        spec(
            vec![0, 1, 0],
            vec![
                (0, ParamModeIr::Value),
                (1, ParamModeIr::Value),
                (2, ParamModeIr::Value),
            ],
            vec![1],
            vec![
                two_type_instruction(Opcode::NewMapBuilder, 0, 1),
                slot_instruction(Opcode::LoadSlot, 0),
                slot_instruction(Opcode::LoadSlot, 1),
                plain(Opcode::MapBuilderPut),
                plain(Opcode::FreezeMap),
                slot_instruction(Opcode::LoadSlot, 2),
                plain(Opcode::MapGet),
                plain(Opcode::Return),
            ],
            3,
        )
        .with_hints(vec![
            hint(&[], &[live(0), live(1), live(0)]),
            hint(&[2], &[live(0), live(1), live(0)]),
            hint(&[2, 0], &[live(0), live(1), live(0)]),
            hint(&[2, 0, 1], &[live(0), live(1), live(0)]),
            hint(&[2], &[live(0), live(1), live(0)]),
            hint(&[2], &[live(0), live(1), live(0)]),
            hint(&[2, 0], &[live(0), live(1), live(0)]),
            hint(&[1], &[live(0), live(1), live(0)]),
        ]),
    );
    prove(&fixture).expect("map builder, freeze, get, and return transfer are proven");
}

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
