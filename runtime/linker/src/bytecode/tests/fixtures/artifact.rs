use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeFunctionOrigin,
    BytecodeImage, BytecodeIntrinsicRef, BytecodePoolEntry, BytecodePools, BytecodeRelocation,
    BytecodeSpecialization, FrameLayout, FrozenConstantGraph, HostEffectReference,
    HostEffectSignature, InstructionSourceSite, IntrinsicReference, NativeTarget,
    PackageCallableId, PackageExecutableCoordinate, ParameterSlotDecl, RelocatableBytecodeFunction,
    ResumeDescriptor, ResumeErrorMode, SourceMapEntry, SourcePosition, SourceSpanRef,
    ResourceDropPlan, StatementAttributionId, StatementEntry,
    SyntheticInstructionSiteReason, TypeRefIr, ValueDropPlan, ValueTransferPlan,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};

use super::{
    constants, synthetic_callback_callable, RootProgram, CALLBACK_FUNCTION, HELPER_CALLABLE,
    HELPER_FUNCTION, ROOT_CALLABLE, ROOT_FUNCTION,
};

const MODULE: &str = "fixture";

pub(super) fn admitted_bytecode(program: RootProgram) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode_artifact(program);
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

pub(super) fn empty_admitted_bytecode() -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode_artifact(RootProgram::LocalCall);
    artifact.image.functions.clear();
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

pub(super) fn constant_only_admitted_bytecode(
    program: constants::ConstantProgram,
) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode_artifact(RootProgram::Constant(program));
    artifact.image.functions.clear();
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

pub(super) fn bytecode_artifact(program: RootProgram) -> BytecodeArtifact {
    let mut functions = BTreeMap::new();
    functions.insert(ROOT_FUNCTION.to_string(), root_function(program));
    functions.insert(HELPER_FUNCTION.to_string(), helper_function());
    if program == RootProgram::SyntheticTarget {
        functions.insert(CALLBACK_FUNCTION.to_string(), callback_function());
    }
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions,
            pools: pools(program),
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph { nodes: Vec::new() },
            debug_table: None,
        },
    };
    if let RootProgram::Constant(constant) = program {
        constants::populate_bytecode(&mut artifact, constant);
    }
    artifact
}

fn root_function(program: RootProgram) -> RelocatableBytecodeFunction {
    let (words, relocations, _resume, source_map) = root_body(program);
    let has_parameter = program.root_has_parameter();
    let has_local = matches!(program, RootProgram::FromType);
    let slot_count = if program == RootProgram::StreamNextLoop {
        3
    } else {
        u32::from(has_parameter || has_local)
    };
    let slot_type_refs = match program {
        RootProgram::StreamNextLoop => vec![0, 1, 0],
        RootProgram::Interface | RootProgram::StreamProducer => vec![1],
        _ if slot_count == 1 => vec![0],
        _ => Vec::new(),
    };
    let slot_plan = if has_local {
        ValueTransferPlan::FromType {
            ty: TypeRefIr::builtin("string"),
        }
    } else if program == RootProgram::StreamNext || program == RootProgram::StreamNextLoop {
        stream_plan()
    } else if program == RootProgram::StreamProducer {
        stream_item_plan()
    } else {
        snapshot_plan()
    };
    let slot_plans = match program {
        RootProgram::StreamNextLoop => vec![stream_plan(), snapshot_plan(), stream_plan()],
        _ => (slot_count == 1).then_some(slot_plan).into_iter().collect(),
    };
    let has_result = matches!(
        program,
        RootProgram::Host
            | RootProgram::Intrinsic
            | RootProgram::RecordShape
            | RootProgram::ArraysMaps
    );
    let is_stream_producer = matches!(program, RootProgram::StreamProducer);
    let result_type_refs = has_result.then_some(0).into_iter().collect::<Vec<_>>();
    let result_plans = has_result
        .then(|| result_plan(program))
        .into_iter()
        .collect::<Vec<_>>();
    RelocatableBytecodeFunction {
        function_key: ROOT_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(0),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words,
        relocations,
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count,
            slot_type_refs,
            parameter_slots: has_parameter
                .then(|| ParameterSlotDecl {
                    slot: 0,
                    mode: skiff_artifact_model::ParamModeIr::Value,
                    plan: match program {
                        RootProgram::StreamNext | RootProgram::StreamNextLoop => stream_plan(),
                        RootProgram::StreamProducer => stream_item_plan(),
                        _ => snapshot_plan(),
                    },
                })
                .into_iter()
                .collect(),
            writable_local_slots: Vec::new(),
            result_count: u32::from(has_result),
            result_type_refs,
            result_plans,
            stream_result_type_ref: is_stream_producer.then_some(0),
            slot_plans,
        },
        max_operand_depth: match program {
            RootProgram::RecordShape => 2,
            RootProgram::Interface
            | RootProgram::Host
            | RootProgram::Intrinsic
            | RootProgram::ArraysMaps
            | RootProgram::StreamNext
            | RootProgram::StreamNextLoop
            | RootProgram::StreamProducer
            | RootProgram::Constant(_) => 1,
            _ => 0,
        },
        effect_summary_ref: PackageCallableId::new(ROOT_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: root_statement_entries(program),
        source_map,
    }
}

fn helper_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: HELPER_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(1),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: empty_frame(),
        max_operand_depth: 0,
        effect_summary_ref: PackageCallableId::new(HELPER_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn callback_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: CALLBACK_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::SyntheticCallback {
            owner: coordinate(0),
            site_ordinal: 0,
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: empty_frame(),
        max_operand_depth: 0,
        effect_summary_ref: synthetic_callback_callable(),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn root_statement_entries(program: RootProgram) -> Vec<StatementEntry> {
    if matches!(program, RootProgram::StreamNext) {
        return vec![StatementEntry {
            pc: 0,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Generated { ordinal: 0 },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
            },
        }];
    }
    if matches!(program, RootProgram::StreamNextLoop) {
        return vec![StatementEntry {
            pc: 3,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Generated { ordinal: 0 },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
            },
        }];
    }
    if matches!(program, RootProgram::StreamProducer) {
        return vec![StatementEntry {
            pc: 2,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Generated { ordinal: 0 },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
            },
        }];
    }
    if !matches!(
        program,
        RootProgram::LocalCall | RootProgram::SyntheticTarget | RootProgram::ServiceDependency
    ) {
        return Vec::new();
    }
    vec![
        StatementEntry {
            pc: 0,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Statement {
                statement_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site(1),
        },
        StatementEntry {
            pc: 0,
            sequence_ordinal: 1,
            attribution_id: StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site(2),
        },
        StatementEntry {
            pc: 4,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Generated { ordinal: 0 },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
            },
        },
    ]
}

fn source_site(source_id: u64) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    }
}

fn root_body(
    program: RootProgram,
) -> (
    Vec<u32>,
    Vec<BytecodeRelocation>,
    Option<ResumeDescriptor>,
    Vec<SourceMapEntry>,
) {
    match program {
        RootProgram::LocalCall | RootProgram::ServiceDependency => (
            vec![0x20, 0, 0, 0, 0x25],
            vec![BytecodeRelocation::LocalExecutableRef {
                function_key: HELPER_FUNCTION.to_string(),
                specialization: BytecodeSpecialization {
                    type_arguments: Vec::new(),
                    concrete_receiver: None,
                },
            }],
            None,
            vec![source_map(0, 4)],
        ),
        RootProgram::SyntheticTarget => (
            vec![0x20, 0, 0, 0, 0x25],
            vec![BytecodeRelocation::LocalExecutableRef {
                function_key: CALLBACK_FUNCTION.to_string(),
                specialization: BytecodeSpecialization {
                    type_arguments: Vec::new(),
                    concrete_receiver: None,
                },
            }],
            None,
            vec![source_map(0, 4)],
        ),
        RootProgram::Interface => (
            vec![0x06, 0, 0x24, 0, 0, 0, 1, 0, 0x08, 0x25],
            vec![BytecodeRelocation::InterfaceRequirementRef {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: interface_identity(),
                    canonical_type_args: Vec::new(),
                },
            }],
            Some(interface_resume_descriptor()),
            vec![source_map(2, 8)],
        ),
        RootProgram::Host => (
            vec![0x80, 0, 0, 1, 0, 0x25],
            vec![BytecodeRelocation::HostEffectRef(valid_host_effect())],
            Some(host_resume_descriptor()),
            vec![source_map(0, 5)],
        ),
        RootProgram::Intrinsic => (
            vec![0x81, 0, 0, 1, 0x25],
            vec![BytecodeRelocation::IntrinsicRef {
                intrinsic: valid_intrinsic(),
            }],
            None,
            vec![source_map(0, 4)],
        ),
        RootProgram::RecordShape => (
            vec![0x06, 0, 0x40, 0, 1, 0x41, 0, 0, 0x25],
            Vec::new(),
            None,
            vec![source_map(2, 5)],
        ),
        RootProgram::ArraysMaps => (
            vec![0x50, 1, 0x25],
            Vec::new(),
            None,
            vec![source_map(0, 2)],
        ),
        RootProgram::FromType => (vec![0x25], Vec::new(), None, Vec::new()),
        RootProgram::StreamNext => (
            vec![0x60, 0, 0, 0x08, 0x25, 0x25],
            Vec::new(),
            Some(stream_next_resume_descriptor()),
            vec![source_map(0, 3)],
        ),
        RootProgram::StreamNextLoop => (
            vec![0x02, 0, 2, 0x60, 2, 0, 0x03, 1, 0x10, 4_294_967_289, 0x25],
            Vec::new(),
            Some(stream_next_loop_resume_descriptor()),
            vec![source_map(3, 6)],
        ),
        RootProgram::StreamProducer => (
            vec![0x06, 0, 0x61, 0, 0x25],
            Vec::new(),
            Some(stream_producer_resume_descriptor()),
            vec![source_map(2, 4)],
        ),
        RootProgram::Constant(_) => (vec![0x00, 0, 0x08, 0x25], Vec::new(), None, Vec::new()),
    }
}

fn interface_identity() -> String {
    let identity = TypeRefIr::ServiceSymbol {
        symbol: skiff_artifact_model::ServiceSymbolRef {
            module_path: MODULE.to_string(),
            symbol: "Reader".to_string(),
        },
    };
    String::from_utf8(skiff_canonical_json::canonical_json_bytes(&identity).unwrap()).unwrap()
}

fn pools(program: RootProgram) -> BytecodePools {
    BytecodePools {
        constants: Vec::new(),
        types: match program {
            RootProgram::Host => vec![BytecodePoolEntry::TypeRef {
                ty: TypeRefIr::builtin("Date"),
            }],
            RootProgram::Intrinsic => vec![
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    },
                },
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("string"),
                },
            ],
            RootProgram::Interface => vec![
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("string"),
                },
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::AnyInterface {
                        interface: skiff_artifact_model::InterfaceInstantiationRef {
                            interface_abi_id: interface_identity(),
                            canonical_type_args: Vec::new(),
                        },
                    },
                },
            ],
            RootProgram::FromType => vec![BytecodePoolEntry::TypeRef {
                ty: TypeRefIr::builtin("string"),
            }],
            RootProgram::RecordShape => vec![
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("string"),
                },
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::Record {
                        fields: BTreeMap::from([(
                            "name".to_string(),
                            TypeRefIr::builtin("string"),
                        )]),
                    },
                },
            ],
            RootProgram::ArraysMaps => vec![
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    },
                },
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("string"),
                },
            ],
            RootProgram::StreamNext | RootProgram::StreamProducer => vec![
                BytecodePoolEntry::TypeRef { ty: stream_type() },
                BytecodePoolEntry::TypeRef { ty: item_type() },
            ],
            RootProgram::StreamNextLoop => vec![
                BytecodePoolEntry::TypeRef { ty: number_stream_type() },
                BytecodePoolEntry::TypeRef { ty: TypeRefIr::builtin("number") },
            ],
            _ => Vec::new(),
        },
        shapes: match program {
            RootProgram::RecordShape => vec![BytecodePoolEntry::ShapeRef {
                shape: skiff_artifact_model::ShapeDeclaration {
                    type_ref: 1,
                    fields: vec![skiff_artifact_model::ShapeFieldDeclaration {
                        name: "name".to_string(),
                        type_ref: 0,
                        plan: snapshot_plan(),
                    }],
                },
            }],
            _ => Vec::new(),
        },
        effects: Vec::new(),
        resume: match root_body(program).2 {
            Some(descriptor) => vec![BytecodePoolEntry::ResumeDescriptor(descriptor)],
            None => Vec::new(),
        },
        callback_capture: Vec::new(),
        writable_paths: Vec::new(),
    }
}

fn valid_host_effect() -> HostEffectReference {
    let entry = skiff_artifact_model::host_effect_registry()
        .entries()
        .iter()
        .find(|entry| entry.binding_key == "core.date.now")
        .expect("built-in host registry has core.date.now");
    HostEffectReference {
        target: NativeTarget {
            namespace: "Date".to_string(),
            symbol: "now".to_string(),
            binding_key: Some(entry.binding_key.clone()),
            metadata: BTreeMap::new(),
        },
        signature: HostEffectSignature {
            parameter_types: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_plans: Vec::new(),
            result_types: vec![TypeRefIr::builtin("Date")],
            result_plans: vec![snapshot_plan()],
            effects: entry.signature.effects.clone(),
        },
        db_operation: None,
    }
}

fn valid_intrinsic() -> IntrinsicReference {
    let entry = skiff_artifact_model::intrinsic_registry()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                &entry.target,
                BytecodeIntrinsicRef::Static { canonical_key, .. }
                    if canonical_key == "core.array.empty"
            )
        })
        .expect("built-in intrinsic registry has core.array.empty");
    IntrinsicReference {
        target: entry.target.clone(),
        signature: HostEffectSignature {
            parameter_types: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_plans: Vec::new(),
            result_types: vec![TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            }],
            result_plans: vec![ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }],
            effects: entry.signature.effects.clone(),
        },
    }
}

fn interface_resume_descriptor() -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc: 2,
        resume_pc: 8,
        end_resume_pc: None,
        expected_stack_height_before_result: 0,
        result_type_refs: vec![0],
        result_plans: vec![snapshot_plan()],
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn host_resume_descriptor() -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc: 0,
        resume_pc: 5,
        end_resume_pc: None,
        expected_stack_height_before_result: 0,
        result_type_refs: vec![0],
        result_plans: vec![snapshot_plan()],
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn stream_next_resume_descriptor() -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc: 0,
        resume_pc: 3,
        end_resume_pc: Some(5),
        expected_stack_height_before_result: 0,
        result_type_refs: vec![1],
        result_plans: vec![stream_item_plan()],
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn stream_next_loop_resume_descriptor() -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc: 3,
        resume_pc: 6,
        end_resume_pc: Some(10),
        expected_stack_height_before_result: 0,
        result_type_refs: vec![1],
        result_plans: vec![snapshot_plan()],
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn stream_producer_resume_descriptor() -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc: 2,
        resume_pc: 4,
        end_resume_pc: None,
        expected_stack_height_before_result: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn stream_type() -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::builtin("string")],
    }
}

fn item_type() -> TypeRefIr {
    TypeRefIr::builtin("string")
}

fn number_stream_type() -> TypeRefIr {
    TypeRefIr::Builtin {
        name: "Stream".to_string(),
        args: vec![TypeRefIr::builtin("number")],
    }
}

fn stream_plan() -> ValueTransferPlan {
    ValueTransferPlan::AffineResource {
        drop: ResourceDropPlan::ResourceTableRelease,
    }
}

fn stream_item_plan() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    }
}

fn source_map(start_pc: u32, end_pc: u32) -> SourceMapEntry {
    SourceMapEntry {
        start_pc,
        end_pc,
        site: InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
        },
    }
}

fn empty_frame() -> FrameLayout {
    FrameLayout {
        slot_count: 0,
        slot_type_refs: Vec::new(),
        parameter_slots: Vec::new(),
        writable_local_slots: Vec::new(),
        result_count: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        stream_result_type_ref: None,
        slot_plans: Vec::new(),
    }
}

fn result_plan(program: RootProgram) -> ValueTransferPlan {
    match program {
        RootProgram::ArraysMaps | RootProgram::Intrinsic => ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::SnapshotRelease,
        },
        _ => snapshot_plan(),
    }
}

fn snapshot_plan() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    }
}

fn coordinate(executable_index: u32) -> PackageExecutableCoordinate {
    PackageExecutableCoordinate {
        file_ir_identity: "file-ir:fixture".to_string(),
        module_path: MODULE.to_string(),
        executable_index,
    }
}
