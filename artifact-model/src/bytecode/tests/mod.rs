//! Shared test helpers: hand-built canonical fixture (not encoder-generated),
//! mutation helpers and assertion utilities.

pub(crate) use super::*;

mod corpus;
mod limits;
mod roundtrip;
mod schema_snapshot;

use std::collections::BTreeMap;

use crate::bytecode::dto::{
    BytecodeArtifact, BytecodeImage, BytecodePoolEntry, BytecodePools, BytecodeRelocation,
    CallbackCaptureDecl, CallbackCaptureLayout, CatchMatcher, DebugBinding, DebugTable,
    ExceptionRegion, FrameLayout, FrozenConstantGraph, FrozenConstantNode, ParameterSlotDecl,
    RelocatableBytecodeFunction, ResumeDescriptor, ShapeDeclaration, SourceMapEntry,
    StatementEntry, SwitchTable, ValueTransferPlan, ValueTransferPlanKind, BYTECODE_ISA_VERSION,
    BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use crate::bytecode::opcodes::opcode_table_fingerprint;
use crate::refs::SourcePosition;
use crate::types::TypeRefIr;

pub(crate) fn plan(kind: ValueTransferPlanKind) -> ValueTransferPlan {
    ValueTransferPlan { kind }
}

pub(crate) fn snapshot_share() -> ValueTransferPlan {
    plan(ValueTransferPlanKind::SnapshotShare)
}

pub(crate) fn string_type() -> TypeRefIr {
    TypeRefIr::builtin("string")
}

pub(crate) fn number_type() -> TypeRefIr {
    TypeRefIr::builtin("number")
}

/// Hand-written wordcode for `module::main` (not encoder-produced). Layout:
///
/// ```text
/// pc   instruction
/// 0    const pool[0]
/// 2    store_slot slot0
/// 4    jump_if_true -> 6
/// 6    call_local reloc[0], argCount 0
/// 9    budget_checkpoint
/// 10   switch_tag table[0], default -> 13
/// 13   jump -> 15
/// 15   enter_region region[0]
/// 17   budget_checkpoint
/// 18   leave_region region[0]
/// 20   call_service reloc[2], argCount 0, resume pool[0]
/// 24   jump_if_false -> 6
/// 26   return
/// ```
pub(crate) fn main_function_words() -> Vec<u32> {
    vec![
        0x00,
        0,
        0x03,
        0,
        0x11,
        0,
        0x20,
        0,
        0,
        0x14,
        0x13,
        0,
        0,
        0x10,
        0,
        0x72,
        0,
        0x14,
        0x73,
        0,
        0x22,
        2,
        0,
        0,
        0x11,
        0xFFFF_FFEC,
        0x25,
    ]
}

pub(crate) fn main_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: "module::main".to_string(),
        type_parameters: Vec::new(),
        words: main_function_words(),
        relocations: vec![
            BytecodeRelocation::LocalExecutableRef {
                function_key: "module::helper".to_string(),
            },
            BytecodeRelocation::InterfaceRequirementRef {
                interface_identity: "interface:reader".to_string(),
            },
            BytecodeRelocation::ServiceOperationRef {
                operation_abi_id: "operation:svc:call".to_string(),
            },
        ],
        frame_layout: FrameLayout {
            slot_count: 4,
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                plan: snapshot_share(),
            }],
            result_count: 1,
            result_plans: vec![snapshot_share()],
            slot_plans: vec![
                snapshot_share(),
                snapshot_share(),
                plan(ValueTransferPlanKind::MoveOnly),
                plan(ValueTransferPlanKind::AffineResource),
            ],
        },
        max_operand_depth: 8,
        effect_summary_ref: "operation:module:main".to_string(),
        exception_regions: vec![ExceptionRegion {
            start_pc: 15,
            end_pc: 20,
            handler_pc: 26,
            handler_stack_height: 0,
            catch_matchers: vec![CatchMatcher::TypeRef { type_ref: 0 }],
            catch_slot: 1,
            cleanup_depth: 0,
        }],
        switch_tables: vec![SwitchTable {
            tag_pool_index: 0,
            targets: vec![4, 13],
        }],
        statement_entries: vec![
            StatementEntry {
                pc: 0,
                statement_id: "s:main:0".to_string(),
            },
            StatementEntry {
                pc: 9,
                statement_id: "s:main:1".to_string(),
            },
            StatementEntry {
                pc: 24,
                statement_id: "s:main:2".to_string(),
            },
        ],
        source_map: vec![
            SourceMapEntry {
                start: 0,
                end: 6,
                source_id: 0,
                start_position: SourcePosition::new(1, 1),
                end_position: SourcePosition::new(3, 1),
            },
            SourceMapEntry {
                start: 7,
                end: 27,
                source_id: 0,
                start_position: SourcePosition::new(3, 1),
                end_position: SourcePosition::new(9, 1),
            },
        ],
    }
}

pub(crate) fn helper_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: "module::helper".to_string(),
        type_parameters: Vec::new(),
        words: vec![0x14, 0x25],
        relocations: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 2,
            parameter_slots: Vec::new(),
            result_count: 0,
            result_plans: Vec::new(),
            slot_plans: vec![snapshot_share(), plan(ValueTransferPlanKind::MoveOnly)],
        },
        max_operand_depth: 2,
        effect_summary_ref: "operation:module:helper".to_string(),
        exception_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

pub(crate) fn canonical_pools() -> BytecodePools {
    BytecodePools {
        constants: vec![BytecodePoolEntry::FrozenConstantRef { node_index: 0 }],
        types: vec![
            BytecodePoolEntry::TypeRef { ty: string_type() },
            BytecodePoolEntry::TypeRef { ty: number_type() },
        ],
        shapes: vec![BytecodePoolEntry::ShapeRef {
            shape: ShapeDeclaration {
                field_count: 1,
                field_types: vec![0],
            },
        }],
        effects: vec![BytecodePoolEntry::HostEffectRef {
            effect_ref: "effect:llm".to_string(),
        }],
        resume: vec![BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            result_type_ref: 1,
            expected_stack_height: 2,
            result_plan: snapshot_share(),
        })],
        callback_capture: vec![BytecodePoolEntry::CallbackCaptureLayout(
            CallbackCaptureLayout {
                function_key: "module::helper".to_string(),
                captures: vec![
                    CallbackCaptureDecl {
                        slot: 0,
                        plan: snapshot_share(),
                    },
                    CallbackCaptureDecl {
                        slot: 1,
                        plan: plan(ValueTransferPlanKind::MoveOnly),
                    },
                ],
            },
        )],
    }
}

pub(crate) fn canonical_constant_graph() -> FrozenConstantGraph {
    FrozenConstantGraph {
        nodes: vec![
            FrozenConstantNode::Literal {
                literal: crate::types::LiteralIr::Number {
                    value: serde_json::Number::from(42),
                },
            },
            FrozenConstantNode::Array { children: vec![0] },
            FrozenConstantNode::Record {
                shape_index: 0,
                children: vec![0],
            },
            FrozenConstantNode::TypeRef { type_ref: 0 },
            FrozenConstantNode::Behavior {
                function_key: "module::helper".to_string(),
            },
        ],
    }
}

pub(crate) fn canonical_debug_table() -> DebugTable {
    DebugTable {
        bindings: vec![
            DebugBinding {
                function_key: "module::main".to_string(),
                pc: 0,
                name: "x".to_string(),
                slot: 0,
            },
            DebugBinding {
                function_key: "module::helper".to_string(),
                pc: 0,
                name: "y".to_string(),
                slot: 1,
            },
        ],
    }
}

/// Hand-built canonical artifact: every pool category, table, relocation and
/// constant graph node kind appears at least once. This fixture must pass C1–C8.
pub(crate) fn canonical_artifact() -> BytecodeArtifact {
    let mut functions = BTreeMap::new();
    functions.insert("module::main".to_string(), main_function());
    functions.insert("module::helper".to_string(), helper_function());
    BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        bytecode_identity: "skiff-bytecode-image-v1:sha256:fixture".to_string(),
        image: BytecodeImage {
            functions,
            pools: canonical_pools(),
            frozen_constant_graph: canonical_constant_graph(),
            debug_table: Some(canonical_debug_table()),
        },
    }
}

/// Asserts the artifact passes C1–C8.
pub(crate) fn assert_validates(artifact: &BytecodeArtifact) {
    structurally_validate(artifact)
        .unwrap_or_else(|error| panic!("fixture must validate: {error}"));
}

/// Asserts the artifact is rejected and returns the error.
pub(crate) fn assert_rejected(artifact: &BytecodeArtifact) -> StructuralValidationError {
    structurally_validate(artifact).expect_err("corrupt fixture must be rejected")
}
