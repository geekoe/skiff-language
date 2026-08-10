use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeFunctionOrigin,
    BytecodeImage, BytecodeIntrinsicRef, BytecodePoolEntry, BytecodePools, BytecodeRelocation,
    BytecodeSpecialization, FrameLayout, FrozenConstantGraph, HostEffectReference,
    HostEffectSignature, InstructionSourceSite, IntrinsicReference, NativeTarget,
    PackageCallableId, PackageExecutableCoordinate, ParameterSlotDecl, RelocatableBytecodeFunction,
    ResumeDescriptor, ResumeErrorMode, SourceMapEntry, SourcePosition, SourceSpanRef,
    StatementAttributionId, StatementEntry, SyntheticInstructionSiteReason, TypeRefIr,
    ValueDropPlan, ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION,
};

use super::{
    no_effects, synthetic_callback_callable, RootProgram, CALLBACK_FUNCTION, HELPER_CALLABLE,
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

pub(super) fn bytecode_artifact(program: RootProgram) -> BytecodeArtifact {
    let mut functions = BTreeMap::new();
    functions.insert(ROOT_FUNCTION.to_string(), root_function(program));
    functions.insert(HELPER_FUNCTION.to_string(), helper_function());
    if program == RootProgram::SyntheticTarget {
        functions.insert(CALLBACK_FUNCTION.to_string(), callback_function());
    }
    BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions,
            pools: pools(program),
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph { nodes: Vec::new() },
            debug_table: None,
        },
    }
}

fn root_function(program: RootProgram) -> RelocatableBytecodeFunction {
    let (words, relocations, _resume, source_map) = root_body(program);
    let has_parameter = program.root_has_parameter();
    let has_local = matches!(program, RootProgram::FromType);
    let slot_count = u32::from(has_parameter || has_local);
    let slot_type_refs = (slot_count == 1).then_some(0).into_iter().collect();
    let slot_plan = if has_local {
        ValueTransferPlan::FromType {
            ty: TypeRefIr::builtin("string"),
        }
    } else {
        snapshot_plan()
    };
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
                    plan: snapshot_plan(),
                })
                .into_iter()
                .collect(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: (slot_count == 1).then_some(slot_plan).into_iter().collect(),
        },
        max_operand_depth: u32::from(matches!(program, RootProgram::Interface)),
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
            vec![0x06, 0, 0x24, 0, 0, 0, 0, 0, 0x25],
            vec![BytecodeRelocation::InterfaceRequirementRef {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: interface_identity(),
                    canonical_type_args: Vec::new(),
                },
            }],
            Some(resume_descriptor(2, 8)),
            vec![source_map(2, 8)],
        ),
        RootProgram::Host => (
            vec![0x80, 0, 0, 0, 0, 0x25],
            vec![BytecodeRelocation::HostEffectRef(HostEffectReference {
                target: NativeTarget {
                    namespace: "fixture".to_string(),
                    symbol: "host".to_string(),
                    binding_key: Some("fixture.host".to_string()),
                    metadata: BTreeMap::new(),
                },
                signature: HostEffectSignature {
                    parameter_types: Vec::new(),
                    parameter_modes: Vec::new(),
                    parameter_plans: Vec::new(),
                    result_types: Vec::new(),
                    result_plans: Vec::new(),
                    effects: no_effects(),
                },
            })],
            Some(resume_descriptor(0, 5)),
            vec![source_map(0, 5)],
        ),
        RootProgram::Intrinsic => (
            vec![0x81, 0, 0, 0, 0x25],
            vec![BytecodeRelocation::IntrinsicRef {
                intrinsic: IntrinsicReference {
                    target: BytecodeIntrinsicRef::Static {
                        canonical_key: "core.fixture.unavailable".to_string(),
                        signature_version: 1,
                    },
                    signature: HostEffectSignature {
                        parameter_types: Vec::new(),
                        parameter_modes: Vec::new(),
                        parameter_plans: Vec::new(),
                        result_types: Vec::new(),
                        result_plans: Vec::new(),
                        effects: no_effects(),
                    },
                },
            }],
            None,
            vec![source_map(0, 4)],
        ),
        RootProgram::FromType => (vec![0x25], Vec::new(), None, Vec::new()),
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
        types: matches!(program, RootProgram::Interface | RootProgram::FromType)
            .then(|| BytecodePoolEntry::TypeRef {
                ty: TypeRefIr::builtin("string"),
            })
            .into_iter()
            .collect(),
        shapes: Vec::new(),
        effects: Vec::new(),
        resume: match root_body(program).2 {
            Some(descriptor) => vec![BytecodePoolEntry::ResumeDescriptor(descriptor)],
            None => Vec::new(),
        },
        callback_capture: Vec::new(),
        writable_paths: Vec::new(),
    }
}

fn resume_descriptor(site_pc: u32, resume_pc: u32) -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc,
        resume_pc,
        expected_stack_height_before_result: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        error_mode: ResumeErrorMode::RaiseAtSite,
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
        slot_plans: Vec::new(),
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
