use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallIr, CallTargetIr, CallableEffectSummary, CallableMayEffects, ContractOperationId, ExprIr,
    ExprRefIr, FileIrUnit, InstructionSourceSite, LiteralIr, PackageCallableId, ServiceCallRef,
    ServiceProtocolIdentity, SyntheticInstructionSiteReason, TypeRefIr,
};
use skiff_compiler_lowering::mir::{
    MirBlock, MirCallArgument, MirDirectCallFacts, MirExecutableKind, MirExpression, MirFunction,
    MirLiveness, MirParam, MirParamMode, MirSlot, MirSlotKind, MirSourceEventPlan,
    MirSourceEventUnavailableReason, MirStatementEntry, MirStmt, MirStmtKind, MirUnit,
};

use crate::{admit_phase_1_bytecode_mir, BytecodeEmissionError, Phase1MirFactMismatch};

#[test]
fn phase_1_bytecode_admission_rejects_unavailable_source_events_before_token_mint() {
    let unit = unit(vec![function("run", 0)]);

    let error = admit_phase_1_bytecode_mir(&[unit]).unwrap_err();

    assert!(matches!(
        error,
        BytecodeEmissionError::Phase1SourceEventsUnavailable {
            ref module_path,
            ref function_key,
            reason: MirSourceEventUnavailableReason::SourceOwnerNotProvided,
        } if module_path == "main" && function_key == "main::run"
    ));
}

#[test]
fn phase_1_bytecode_admission_requires_service_boundary_plans() {
    let mut mir = unit(vec![function("run", 0)]);
    mir.external_refs.service_call_refs = vec![ServiceCallRef {
        service_requirement_slot: 0,
        contract_operation_id: ContractOperationId::new("operation:echo"),
        expected_protocol_identity: ServiceProtocolIdentity::new("protocol:echo"),
    }];

    let error = admit_phase_1_bytecode_mir(&[mir]).unwrap_err();

    assert!(matches!(
        error,
        BytecodeEmissionError::MissingServiceBoundaryPlan { .. }
    ));
}

#[test]
fn phase_1_bytecode_admission_rejects_parameter_slot_and_load_type_drift() {
    let mut parameter_slot = function("run", 0);
    parameter_slot.params.push(MirParam {
        name: "value".to_string(),
        slot: 0,
        ty: TypeRefIr::builtin("number"),
        mode: MirParamMode::Value,
    });
    parameter_slot.slots.push(MirSlot {
        slot: 0,
        name: "value".to_string(),
        kind: MirSlotKind::Local,
        writable_local: false,
        ty: Some(TypeRefIr::builtin("number")),
    });
    let error = admit_phase_1_bytecode_mir(&[unit(vec![parameter_slot])]).unwrap_err();
    assert!(matches!(
        error,
        BytecodeEmissionError::Phase1MirFactMismatch {
            mismatch: Phase1MirFactMismatch::ParameterSlotKind,
            ..
        }
    ));

    let mut load = function("run", 0);
    load.slots.push(MirSlot {
        slot: 0,
        name: "value".to_string(),
        kind: MirSlotKind::Local,
        writable_local: false,
        ty: Some(TypeRefIr::builtin("number")),
    });
    load.expressions.push(MirExpression {
        index: 0,
        expression: ExprIr::LoadSlot { slot: 0 },
        ty: TypeRefIr::builtin("bool"),
        writable: None,
        direct_call: None,
        stream_result: None,
        remote_interface: None,
    });
    let error = admit_phase_1_bytecode_mir(&[unit(vec![load])]).unwrap_err();
    assert!(matches!(
        error,
        BytecodeEmissionError::Phase1MirFactMismatch {
            mismatch: Phase1MirFactMismatch::LoadSlotType,
            ..
        }
    ));
}

#[test]
fn phase_1_bytecode_admission_rejects_init_slot_type_drift() {
    let mut run = function("run", 0);
    run.slots.push(MirSlot {
        slot: 0,
        name: "value".to_string(),
        kind: MirSlotKind::Local,
        writable_local: false,
        ty: Some(TypeRefIr::builtin("number")),
    });
    run.expressions.push(MirExpression {
        index: 0,
        expression: ExprIr::Literal {
            value: LiteralIr::Bool { value: true },
        },
        ty: TypeRefIr::builtin("bool"),
        writable: None,
        direct_call: None,
        stream_result: None,
        remote_interface: None,
    });
    run.blocks.push(MirBlock {
        id: 0,
        label: "entry".to_string(),
        statements: vec![MirStmt {
            statement_index: 0,
            span: None,
            kind: MirStmtKind::InitSlot {
                slot: 0,
                value: expression(0),
            },
        }],
        successors: Vec::new(),
    });
    run.statements.push(MirStatementEntry {
        statement_index: 0,
        span: None,
    });

    let error = admit_phase_1_bytecode_mir(&[unit(vec![run])]).unwrap_err();
    assert!(matches!(
        error,
        BytecodeEmissionError::Phase1MirFactMismatch {
            mismatch: Phase1MirFactMismatch::InitSlotType,
            ..
        }
    ));
}

#[test]
fn phase_1_bytecode_admission_exact_joins_local_callee_argument_types() {
    let mut caller = function("caller", 1);
    caller.return_type = TypeRefIr::builtin("number");
    caller.expressions = vec![
        MirExpression {
            index: 0,
            expression: ExprIr::Literal {
                value: LiteralIr::Bool { value: true },
            },
            ty: TypeRefIr::builtin("bool"),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        },
        MirExpression {
            index: 1,
            expression: ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::LocalExecutable {
                        executable_index: 0,
                    },
                    concrete_receiver: None,
                    site: synthetic_site(),
                    args: vec![expression(0)],
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            },
            ty: TypeRefIr::builtin("number"),
            writable: None,
            direct_call: Some(MirDirectCallFacts {
                concrete_receiver: None,
                receiver_call_abi: None,
                parameter_modes: vec![MirParamMode::Value],
                arguments: vec![MirCallArgument::Value {
                    value: expression(0),
                }],
            }),
            stream_result: None,
            remote_interface: None,
        },
    ];
    let mut callee = function("callee", 0);
    callee.return_type = TypeRefIr::builtin("number");
    callee.params.push(MirParam {
        name: "value".to_string(),
        slot: 0,
        ty: TypeRefIr::builtin("number"),
        mode: MirParamMode::Value,
    });
    callee.slots.push(MirSlot {
        slot: 0,
        name: "value".to_string(),
        kind: MirSlotKind::Param,
        writable_local: false,
        ty: Some(TypeRefIr::builtin("number")),
    });

    let error = admit_phase_1_bytecode_mir(&[unit(vec![caller, callee])]).unwrap_err();
    assert!(matches!(
        error,
        BytecodeEmissionError::Phase1MirFactMismatch {
            mismatch: Phase1MirFactMismatch::LocalCallArgumentType,
            ..
        }
    ));
}

fn unit(functions: Vec<MirFunction>) -> MirUnit {
    let mut file_ir = FileIrUnit::empty("main", "source-hash");
    file_ir.file_ir_identity = "file:main".to_string();
    MirUnit {
        file_ir_identity: file_ir.file_ir_identity,
        module_path: file_ir.module_path,
        actor_declarations: file_ir.actor_declarations,
        external_refs: file_ir.external_refs,
        source_map: file_ir.source_map,
        type_table: file_ir.type_table,
        package_type_records: BTreeMap::new(),
        link_targets: file_ir.link_targets,
        constants: Vec::new(),
        functions,
    }
}

fn function(name: &str, executable_index: u32) -> MirFunction {
    MirFunction {
        executable_index,
        origin: skiff_artifact_model::PackageExecutableCoordinate {
            file_ir_identity: "file:main".to_string(),
            module_path: "main".to_string(),
            executable_index,
        },
        symbol: format!("main.{name}"),
        kind: MirExecutableKind::Function,
        native: false,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        receiver: None,
        slots: Vec::new(),
        index_accesses: BTreeMap::new(),
        expression_blocks: BTreeMap::new(),
        expressions: Vec::new(),
        blocks: Vec::new(),
        regions: Vec::new(),
        statements: Vec::new(),
        source_event_plan: MirSourceEventPlan::unavailable(
            MirSourceEventUnavailableReason::SourceOwnerNotProvided,
        ),
        stream_result: None,
        liveness: MirLiveness::default(),
        effect_summary_ref: PackageCallableId::new(format!("callable:main:{name}")),
        effect_summary: CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: false,
                pending_effect_categories: Vec::new(),
                inout_path_effects: Vec::new(),
            },
        },
        source_span: None,
    }
}

fn expression(expression: u32) -> ExprRefIr {
    ExprRefIr { expression }
}

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
    }
}
