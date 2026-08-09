//! Shared test helpers: hand-built canonical fixture (not encoder-generated),
//! mutation helpers and assertion utilities.

pub(crate) use super::*;

mod corpus;
mod limits;
mod manifests;
mod property;
mod roundtrip;
mod schema_snapshot;

use std::collections::BTreeMap;

use crate::bytecode::dto::{
    ActiveRegion, ActiveRegionKind, BytecodeArtifact, BytecodeConstantRef, BytecodeFunctionOrigin,
    BytecodeImage, BytecodePoolEntry, BytecodePools, BytecodeRelocation, BytecodeSpecialization,
    CallLoanBinding, CallLoanLayout, CallbackCaptureDecl, CallbackCaptureLayout, CatchMatcher,
    DebugBinding, DebugTable, ExceptionRegion, FrameLayout, FrozenBehaviorBinding,
    FrozenConstantGraph, FrozenConstantNode, HostEffectReference, HostEffectSignature,
    ParameterSlotDecl, RelocatableBytecodeFunction, ResourceDropPlan, ResumeDescriptor,
    ResumeErrorMode, ShapeDeclaration, ShapeFieldDeclaration, SourceMapEntry, StatementChargeKind,
    StatementEntry, SwitchCase, SwitchTable, ValueDropPlan, ValueTransferPlan,
    ValueTransferPlanKind, WritablePathDeclaration, WritablePathSegment, BYTECODE_ISA_VERSION,
    BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use crate::bytecode::opcodes::opcode_table_fingerprint;
use crate::types::TypeRefIr;

pub(crate) fn plan(kind: ValueTransferPlanKind) -> ValueTransferPlan {
    match kind {
        ValueTransferPlanKind::SnapshotShare => ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::Trivial,
        },
        ValueTransferPlanKind::MoveOnly => ValueTransferPlan::MoveOnly {
            drop: ValueDropPlan::Trivial,
        },
        ValueTransferPlanKind::AffineResource => ValueTransferPlan::AffineResource {
            drop: ResourceDropPlan::ResourceTableRelease,
        },
        ValueTransferPlanKind::ExplicitCloneLease => ValueTransferPlan::ExplicitCloneLease {
            clone_adapter: crate::bytecode::dto::NativeValueAdapterRef {
                binding_key: "lifecycle.clone.fixture".to_string(),
            },
            drop: ResourceDropPlan::NativeAdapter {
                adapter: crate::bytecode::dto::NativeValueAdapterRef {
                    binding_key: "lifecycle.drop.fixture".to_string(),
                },
            },
        },
    }
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

pub(crate) fn executable_coordinate(executable_index: u32) -> crate::PackageExecutableCoordinate {
    crate::PackageExecutableCoordinate {
        file_ir_identity: "file-ir:module".to_string(),
        module_path: "module".to_string(),
        executable_index,
    }
}

/// Hand-written wordcode for `module::main` (not encoder-produced). Layout:
///
/// ```text
/// pc   instruction
/// 0    const pool[0]
/// 2    store_slot slot0
/// 4    jump_if_true -> 6
/// 6    call_local reloc[0], argCount 0, resultCount 0
/// 10   budget_checkpoint
/// 11   switch_tag table[0]
/// 13   jump -> 15
/// 15   enter_region activeRegion[0]
/// 17   budget_checkpoint
/// 18   leave_region activeRegion[0]
/// 20   call_service reloc[2], argCount 0, resultCount 1, resume pool[0]
/// 25   jump_if_false -> 6
/// 27   return
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
        0,
        0x14,
        0x13,
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
        1,
        0,
        0x11,
        0xFFFF_FFEB,
        0x25,
    ]
}

pub(crate) fn main_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: "module::main".to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: executable_coordinate(0),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: main_function_words(),
        relocations: vec![
            BytecodeRelocation::LocalExecutableRef {
                function_key: "module::helper".to_string(),
                specialization: BytecodeSpecialization {
                    type_arguments: vec![string_type()],
                    concrete_receiver: None,
                },
            },
            BytecodeRelocation::InterfaceRequirementRef {
                interface: crate::InterfaceInstantiationRef {
                    interface_abi_id: "interface:reader".to_string(),
                    canonical_type_args: Vec::new(),
                },
            },
            BytecodeRelocation::ServiceOperationRef {
                service_call: crate::ServiceCallRef {
                    service_requirement_slot: 0,
                    contract_operation_id: crate::ContractOperationId::new("operation:svc:call"),
                    expected_protocol_identity: crate::ServiceProtocolIdentity::new(
                        "protocol:svc:v1",
                    ),
                },
            },
            BytecodeRelocation::SyntheticCallbackRef {
                function_key: "module::main$callback0".to_string(),
            },
        ],
        call_loan_layouts: vec![CallLoanLayout {
            loans: vec![CallLoanBinding {
                parameter_ordinal: 0,
                root_slot: 1,
                writable_path_ref: 0,
            }],
        }],
        frame_layout: FrameLayout {
            slot_count: 4,
            slot_type_refs: vec![0, 0, 1, 1],
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                mode: crate::ParamModeIr::Value,
                plan: snapshot_share(),
            }],
            writable_local_slots: vec![1],
            result_count: 1,
            result_type_refs: vec![1],
            result_plans: vec![snapshot_share()],
            slot_plans: vec![
                snapshot_share(),
                snapshot_share(),
                plan(ValueTransferPlanKind::MoveOnly),
                plan(ValueTransferPlanKind::AffineResource),
            ],
        },
        max_operand_depth: 8,
        effect_summary_ref: crate::PackageCallableId::new("operation:module:main"),
        exception_regions: vec![ExceptionRegion {
            start_pc: 15,
            end_pc: 20,
            handler_pc: 27,
            handler_stack_height: 0,
            catch_matchers: vec![CatchMatcher::TypeRef { type_ref: 0 }],
            catch_slot: 1,
            catch_slot_type_ref: 0,
            cleanup_depth: 0,
        }],
        active_regions: vec![ActiveRegion {
            start_pc: 15,
            end_pc: 20,
            kind: ActiveRegionKind::Timeout {
                duration_ms: 1_000,
                site: crate::InstructionSourceSite::Synthetic {
                    reason: crate::SyntheticInstructionSiteReason::RuntimeControlFlow,
                },
            },
        }],
        switch_tables: vec![SwitchTable {
            cases: vec![
                SwitchCase {
                    tag_type_ref: 0,
                    target_pc: 4,
                },
                SwitchCase {
                    tag_type_ref: 1,
                    target_pc: 13,
                },
            ],
            default_pc: 13,
        }],
        statement_entries: vec![
            StatementEntry {
                pc: 0,
                statement_id: "s:main:0".to_string(),
                charge_kind: StatementChargeKind::FunctionEntry,
            },
            StatementEntry {
                pc: 10,
                statement_id: "s:main:1".to_string(),
                charge_kind: StatementChargeKind::Statement,
            },
            StatementEntry {
                pc: 25,
                statement_id: "s:main:2".to_string(),
                charge_kind: StatementChargeKind::LoopCheck,
            },
        ],
        source_map: vec![
            SourceMapEntry {
                start_pc: 0,
                end_pc: 10,
                site: crate::InstructionSourceSite::Source {
                    span: crate::SourceSpanRef {
                        source_id: 0,
                        start: crate::SourcePosition::new(1, 1),
                        end: crate::SourcePosition::new(3, 1),
                    },
                },
            },
            SourceMapEntry {
                start_pc: 10,
                end_pc: 28,
                site: crate::InstructionSourceSite::Source {
                    span: crate::SourceSpanRef {
                        source_id: 0,
                        start: crate::SourcePosition::new(3, 1),
                        end: crate::SourcePosition::new(9, 1),
                    },
                },
            },
        ],
    }
}

pub(crate) fn helper_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: "module::helper".to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: executable_coordinate(1),
        },
        type_parameters: vec!["T".to_string()],
        self_type_ref: None,
        words: vec![0x14, 0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 2,
            slot_type_refs: vec![0, 1],
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: vec![snapshot_share(), plan(ValueTransferPlanKind::MoveOnly)],
        },
        max_operand_depth: 2,
        effect_summary_ref: crate::PackageCallableId::new("operation:module:helper"),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: vec![StatementEntry {
            pc: 0,
            statement_id: "s:helper:entry".to_string(),
            charge_kind: StatementChargeKind::FunctionEntry,
        }],
        source_map: Vec::new(),
    }
}

pub(crate) fn callback_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: "module::main$callback0".to_string(),
        origin: BytecodeFunctionOrigin::SyntheticCallback {
            owner: executable_coordinate(0),
            site_ordinal: 0,
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x14, 0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 2,
            slot_type_refs: vec![0, 1],
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: vec![snapshot_share(), plan(ValueTransferPlanKind::MoveOnly)],
        },
        max_operand_depth: 2,
        effect_summary_ref: crate::PackageCallableId::new("operation:module:main$callback0"),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: vec![StatementEntry {
            pc: 0,
            statement_id: "s:callback:entry".to_string(),
            charge_kind: StatementChargeKind::FunctionEntry,
        }],
        source_map: Vec::new(),
    }
}

pub(crate) fn canonical_pools() -> BytecodePools {
    BytecodePools {
        constants: vec![
            BytecodePoolEntry::ConstantRef {
                reference: BytecodeConstantRef::LocalNode { node_index: 1 },
                type_ref: 0,
                plan: snapshot_share(),
            },
            BytecodePoolEntry::ConstantRef {
                reference: BytecodeConstantRef::LocalNode { node_index: 3 },
                type_ref: 0,
                plan: snapshot_share(),
            },
            BytecodePoolEntry::ConstantRef {
                reference: BytecodeConstantRef::LocalNode { node_index: 4 },
                type_ref: 0,
                plan: snapshot_share(),
            },
        ],
        types: vec![
            BytecodePoolEntry::TypeRef { ty: string_type() },
            BytecodePoolEntry::TypeRef { ty: number_type() },
        ],
        shapes: vec![BytecodePoolEntry::ShapeRef {
            shape: ShapeDeclaration {
                type_ref: 0,
                fields: vec![ShapeFieldDeclaration {
                    name: "value".to_string(),
                    type_ref: 0,
                    plan: snapshot_share(),
                }],
            },
        }],
        effects: vec![BytecodePoolEntry::HostEffectRef(HostEffectReference {
            target: crate::NativeTarget {
                namespace: "fixture".to_string(),
                symbol: "effect".to_string(),
                binding_key: Some("fixture.effect".to_string()),
                metadata: BTreeMap::new(),
            },
            signature: HostEffectSignature {
                parameter_types: Vec::new(),
                parameter_modes: Vec::new(),
                parameter_plans: Vec::new(),
                result_types: vec![number_type()],
                result_plans: vec![snapshot_share()],
                effects: crate::CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: true,
                    pending_effect_categories: vec![crate::PendingEffectCategory::NativeCall],
                    inout_path_effects: Vec::new(),
                },
            },
        })],
        resume: vec![BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            function_key: "module::main".to_string(),
            site_pc: 20,
            resume_pc: 25,
            expected_stack_height_before_result: 2,
            result_type_refs: vec![1],
            result_plans: vec![snapshot_share()],
            error_mode: ResumeErrorMode::RaiseAtSite,
        })],
        callback_capture: vec![BytecodePoolEntry::CallbackCaptureLayout(
            CallbackCaptureLayout {
                function_key: "module::main$callback0".to_string(),
                captures: vec![
                    CallbackCaptureDecl {
                        target_slot: 0,
                        type_ref: 0,
                        plan: snapshot_share(),
                    },
                    CallbackCaptureDecl {
                        target_slot: 1,
                        type_ref: 1,
                        plan: plan(ValueTransferPlanKind::MoveOnly),
                    },
                ],
            },
        )],
        writable_paths: vec![BytecodePoolEntry::WritablePath(WritablePathDeclaration {
            root_type_ref: 0,
            leaf_type_ref: 0,
            segments: vec![WritablePathSegment::DenseField {
                shape_ref: 0,
                field_ordinal: 0,
            }],
        })],
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
            FrozenConstantNode::Representation {
                type_ref: 0,
                value: 2,
            },
            FrozenConstantNode::Implementation {
                record: 2,
                behaviors: vec![FrozenBehaviorBinding {
                    function_key: "module::helper".to_string(),
                }],
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
    functions.insert("module::main$callback0".to_string(), callback_function());
    BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry: crate::native_value_lifecycle_registry_identity().clone(),
        bytecode_identity: "skiff-bytecode-image-v1:sha256:fixture".to_string(),
        image: BytecodeImage {
            functions,
            pools: canonical_pools(),
            constant_roots: BTreeMap::from([
                ("module.array".to_string(), 0),
                ("module.implementation".to_string(), 2),
                ("module.representation".to_string(), 1),
            ]),
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
