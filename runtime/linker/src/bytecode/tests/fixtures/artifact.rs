use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BoundaryDropPlan, BoundaryTransfer,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueFact, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, BytecodeArtifact, BytecodeFunctionOrigin, BytecodeImage,
    BytecodeIntrinsicRef, BytecodePoolEntry, BytecodePools, BytecodeRelocation,
    BytecodeSpecialization, CallableEffectSummary, CallableMayEffects, CallbackCaptureLayout,
    ContractTypeRef, FrameLayout, FrozenConstantGraph, FunctionTypeParamIr, HostEffectReference,
    HostEffectSignature, InstructionSourceSite, InterfaceMethodSlotSignatureIr,
    InterfaceRequirementMethod, IntrinsicReference, LocalInterfaceMethod, LocalInterfaceRef,
    NativeTarget, PackageCallableId, PackageExecutableCoordinate, PackageRefIr, PackageSymbolRef,
    ParameterSlotDecl, ReceiverCallAbi, RelocatableBytecodeFunction, RemoteInterfaceMethod,
    RemoteInterfaceRef, ResourceDropPlan, ResumeDescriptor, ResumeErrorMode,
    ServiceCallBoundaryFacts, ServiceCallRef, SourceMapEntry, SourcePosition, SourceSpanRef,
    StatementAttributionId, StatementEntry, SyntheticInstructionSiteReason, TypeRefIr,
    ValueDropPlan, ValueProvenance, ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION,
};

use super::{
    constants, synthetic_callback_callable_for, RootProgram, CALLBACK_FUNCTION, HELPER_CALLABLE,
    HELPER_FUNCTION, ROOT_CALLABLE, ROOT_FUNCTION,
};

const MODULE: &str = "fixture";

pub(super) fn admitted_bytecode(program: RootProgram) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode_artifact(program);
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

pub(super) fn service_operation_artifact(drifted: bool) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode_artifact(RootProgram::ServiceOperation);
    if drifted {
        let relocation = artifact
            .image
            .functions
            .get_mut(ROOT_FUNCTION)
            .unwrap()
            .relocations
            .first_mut()
            .unwrap();
        let BytecodeRelocation::ServiceOperationRef { service_call } = relocation else {
            panic!("service operation fixture has a service relocation")
        };
        service_call
            .boundary_plan_mut()
            .arguments
            .push(BoundaryValueFact {
                contract_type: ContractTypeRef::builtin("string"),
                value_plan: BoundaryValuePlan::Linkable {
                    carrier: BoundaryValueCarrier::DetachedValueGraph,
                    encoding: BoundaryValueEncoding::CanonicalValue,
                    owner: BoundaryValueOwner::Caller,
                    lifetime: BoundaryValueLifetime::Call,
                },
                transfer: BoundaryTransfer::Copy,
                drop: BoundaryDropPlan::SnapshotRelease,
                source: ValueProvenance::CallerParameter { index: 0 },
            });
    }
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

pub(super) fn remote_interface_artifact(drifted: bool) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode_artifact(RootProgram::RemoteInterface);
    if drifted {
        let relocation = artifact
            .image
            .functions
            .get_mut(ROOT_FUNCTION)
            .unwrap()
            .relocations
            .first_mut()
            .unwrap();
        let BytecodeRelocation::RemoteInterfaceRef { interface } = relocation else {
            panic!("remote interface fixture has a remote relocation")
        };
        let method = &mut interface.methods[0];
        let argument = method.boundary_plan.arguments[0].clone();
        method.boundary_plan.arguments.push(argument);
    }
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
    functions.insert(HELPER_FUNCTION.to_string(), helper_function(program));
    if matches!(
        program,
        RootProgram::SyntheticTarget | RootProgram::UnreachableCallback
    ) {
        let owner = u32::from(program == RootProgram::UnreachableCallback);
        functions.insert(CALLBACK_FUNCTION.to_string(), callback_function(owner));
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
        RootProgram::Interface
        | RootProgram::StreamProducer
        | RootProgram::ReorderedStreamResumePool => vec![1],
        RootProgram::ReorderedStreamNextResumePool => vec![0],
        _ if slot_count == 1 => vec![0],
        _ => Vec::new(),
    };
    let slot_plan = if has_local {
        ValueTransferPlan::FromType {
            ty: TypeRefIr::builtin("string"),
        }
    } else if matches!(
        program,
        RootProgram::StreamNext
            | RootProgram::StreamNextLoop
            | RootProgram::ReorderedStreamNextResumePool
    ) {
        stream_plan()
    } else if matches!(
        program,
        RootProgram::StreamProducer | RootProgram::ReorderedStreamResumePool
    ) {
        stream_item_plan()
    } else if matches!(
        program,
        RootProgram::Interface | RootProgram::LocalInterface | RootProgram::RecordShape
    ) {
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
    let is_stream_producer = matches!(
        program,
        RootProgram::StreamProducer | RootProgram::ReorderedStreamResumePool
    );
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
                        RootProgram::StreamNext
                        | RootProgram::StreamNextLoop
                        | RootProgram::ReorderedStreamNextResumePool => stream_plan(),
                        RootProgram::StreamProducer | RootProgram::ReorderedStreamResumePool => {
                            stream_item_plan()
                        }
                        RootProgram::Interface
                        | RootProgram::LocalInterface
                        | RootProgram::RecordShape => stream_item_plan(),
                        _ => snapshot_plan(),
                    },
                    dense_record_shape_ref: None,
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
            | RootProgram::RemoteInterface
            | RootProgram::LocalInterface
            | RootProgram::ReorderedStreamNextResumePool
            | RootProgram::Host
            | RootProgram::Intrinsic
            | RootProgram::ArraysMaps
            | RootProgram::StreamNext
            | RootProgram::StreamNextLoop
            | RootProgram::StreamProducer
            | RootProgram::ReorderedStreamResumePool
            | RootProgram::SyntheticTarget
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

fn helper_function(program: RootProgram) -> RelocatableBytecodeFunction {
    if matches!(
        program,
        RootProgram::Constant(constants::ConstantProgram::Implementation)
    ) {
        return implementation_helper_function();
    }
    if program == RootProgram::UnreachableInterface {
        return private_interface_function();
    }
    if program == RootProgram::LocalInterface {
        return local_interface_helper_function();
    }
    if program == RootProgram::UnreachableCallback {
        return callback_maker_function(HELPER_FUNCTION, HELPER_CALLABLE, coordinate(1));
    }
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

fn implementation_helper_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: HELPER_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(1),
        },
        type_parameters: Vec::new(),
        self_type_ref: Some(0),
        words: vec![0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![0],
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                mode: skiff_artifact_model::ParamModeIr::Value,
                plan: ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::SnapshotRelease,
                },
                dense_record_shape_ref: None,
            }],
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
            slot_plans: vec![ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }],
        },
        max_operand_depth: 0,
        effect_summary_ref: PackageCallableId::new(HELPER_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn callback_function(owner_index: u32) -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: CALLBACK_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::SyntheticCallback {
            owner: coordinate(owner_index),
            site_ordinal: 0,
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![0],
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                mode: skiff_artifact_model::ParamModeIr::Value,
                plan: snapshot_plan(),
                dense_record_shape_ref: None,
            }],
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
            slot_plans: vec![snapshot_plan()],
        },
        max_operand_depth: 0,
        effect_summary_ref: synthetic_callback_callable_for(if owner_index == 0 {
            ROOT_CALLABLE
        } else {
            HELPER_CALLABLE
        }),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn private_interface_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: HELPER_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(1),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x06, 0, 0x24, 0, 0, 0, 1, 0, 0x08, 0x25],
        relocations: vec![BytecodeRelocation::InterfaceRequirementRef {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface_identity(),
                canonical_type_args: Vec::new(),
            },
            methods: Vec::new(),
        }],
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![1],
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                mode: skiff_artifact_model::ParamModeIr::Value,
                plan: stream_item_plan(),
                dense_record_shape_ref: None,
            }],
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
            slot_plans: vec![stream_item_plan()],
        },
        max_operand_depth: 1,
        effect_summary_ref: PackageCallableId::new(HELPER_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: vec![source_map(2, 8)],
    }
}

fn local_interface_helper_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: HELPER_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(1),
        },
        type_parameters: Vec::new(),
        self_type_ref: Some(0),
        words: vec![0x06, 0, 0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 1,
            slot_type_refs: vec![0],
            parameter_slots: vec![ParameterSlotDecl {
                slot: 0,
                mode: skiff_artifact_model::ParamModeIr::Value,
                plan: stream_item_plan(),
                dense_record_shape_ref: None,
            }],
            writable_local_slots: Vec::new(),
            result_count: 1,
            result_type_refs: vec![0],
            result_plans: vec![stream_item_plan()],
            stream_result_type_ref: None,
            slot_plans: vec![stream_item_plan()],
        },
        max_operand_depth: 1,
        effect_summary_ref: PackageCallableId::new(HELPER_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn callback_maker_function(
    function_key: &str,
    effect_summary: &str,
    executable: PackageExecutableCoordinate,
) -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: function_key.to_string(),
        origin: BytecodeFunctionOrigin::Executable { executable },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x32, 0, 0, 0, 0x08, 0x25],
        relocations: vec![BytecodeRelocation::SyntheticCallbackRef {
            function_key: CALLBACK_FUNCTION.to_string(),
        }],
        call_loan_layouts: Vec::new(),
        frame_layout: empty_frame(),
        max_operand_depth: 1,
        effect_summary_ref: PackageCallableId::new(effect_summary),
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
    if matches!(program, RootProgram::ReorderedStreamResumePool) {
        return [2, 6]
            .into_iter()
            .enumerate()
            .map(|(ordinal, pc)| StatementEntry {
                pc,
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Generated {
                    ordinal: u32::try_from(ordinal).unwrap(),
                },
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
                },
            })
            .collect();
    }
    if !matches!(
        program,
        RootProgram::LocalCall
            | RootProgram::SyntheticTarget
            | RootProgram::ServiceDependency
            | RootProgram::ServiceOperation
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
            pc: if program == RootProgram::ServiceOperation {
                5
            } else {
                4
            },
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
            vec![0x32, 0, 0, 0, 0x08, 0x25],
            vec![BytecodeRelocation::SyntheticCallbackRef {
                function_key: CALLBACK_FUNCTION.to_string(),
            }],
            None,
            vec![source_map(0, 4)],
        ),
        RootProgram::UnreachableCallback | RootProgram::UnreachableInterface => {
            (vec![0x25], Vec::new(), None, Vec::new())
        }
        RootProgram::Interface => (
            vec![0x06, 0, 0x24, 0, 0, 0, 1, 0, 0x08, 0x25],
            vec![BytecodeRelocation::InterfaceRequirementRef {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: interface_identity(),
                    canonical_type_args: Vec::new(),
                },
                methods: Vec::new(),
            }],
            Some(interface_resume_descriptor(ROOT_FUNCTION)),
            vec![source_map(2, 8)],
        ),
        RootProgram::LocalInterface => {
            let interface = skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface_identity(),
                canonical_type_args: Vec::new(),
            };
            let signature = InterfaceMethodSlotSignatureIr {
                params: vec![FunctionTypeParamIr {
                    name: "self".to_string(),
                    ty: TypeRefIr::builtin("string"),
                }],
                return_type: TypeRefIr::builtin("string"),
            };
            let effects = CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: false,
                    pending_effect_categories: Vec::new(),
                    inout_path_effects: Vec::new(),
                },
            };
            let local_methods = vec![LocalInterfaceMethod {
                slot: 0,
                method_name: "label".to_string(),
                method_abi_id: skiff_artifact_identity::canonical_interface_method_abi_id(
                    &interface, "label",
                ),
                signature: signature.clone(),
                effects: effects.clone(),
                function_key: HELPER_FUNCTION.to_string(),
                receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
            }];
            let requirement_methods = vec![InterfaceRequirementMethod {
                slot: 0,
                method_abi_id: skiff_artifact_identity::canonical_interface_method_abi_id(
                    &interface, "label",
                ),
                signature,
                effects,
            }];
            (
                vec![0x06, 0, 0x30, 0, 0x24, 1, 0, 0, 1, 0, 0x08, 0x25],
                vec![
                    BytecodeRelocation::LocalInterfaceRef {
                        interface: LocalInterfaceRef {
                            interface: interface.clone(),
                            concrete_type: TypeRefIr::builtin("string"),
                            methods: local_methods,
                        },
                    },
                    BytecodeRelocation::InterfaceRequirementRef {
                        interface,
                        methods: requirement_methods,
                    },
                ],
                Some(interface_resume_descriptor_at(ROOT_FUNCTION, 4, 10)),
                vec![source_map(4, 10)],
            )
        }
        RootProgram::RemoteInterface => {
            let interface = skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface_identity(),
                canonical_type_args: Vec::new(),
            };
            let method_abi =
                skiff_artifact_identity::canonical_interface_method_abi_id(&interface, "read");
            let signature = InterfaceMethodSlotSignatureIr {
                params: vec![
                    FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: TypeRefIr::builtin("Self"),
                    },
                    FunctionTypeParamIr {
                        name: "seed".to_string(),
                        ty: TypeRefIr::builtin("string"),
                    },
                ],
                return_type: TypeRefIr::builtin("string"),
            };
            let (provider_contract, _, provider_operation) =
                super::records::remote_contract("example.bytecode-link-provider", "call");
            (
                vec![0x31, 0, 0x08, 0x25],
                vec![BytecodeRelocation::RemoteInterfaceRef {
                    interface: RemoteInterfaceRef {
                        service_requirement_slot: 7,
                        public_instance_key: "reader".to_string(),
                        interface,
                        methods: vec![RemoteInterfaceMethod {
                            slot: 0,
                            method_abi_id: method_abi,
                            signature,
                            contract_operation_id: provider_operation,
                            boundary_plan: super::records::remote_service_boundary_plan(),
                        }],
                        callee_protocol_identity: provider_contract
                            .service_protocol_identity
                            .clone(),
                    },
                }],
                None,
                Vec::new(),
            )
        }
        RootProgram::Host => (
            vec![0x80, 0, 0, 1, 0, 0x25],
            vec![BytecodeRelocation::HostEffectRef(valid_host_effect())],
            Some(host_resume_descriptor()),
            vec![source_map(0, 5)],
        ),
        RootProgram::ReorderedStreamNextResumePool => (
            vec![0x60, 0, 1, 0x08, 0x60, 0, 0, 0x08, 0x25, 0x25],
            Vec::new(),
            None,
            vec![source_map(0, 3), source_map(4, 7)],
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
        RootProgram::ReorderedStreamResumePool => (
            vec![0x06, 0, 0x61, 1, 0x06, 0, 0x61, 0, 0x25],
            Vec::new(),
            None,
            vec![source_map(2, 4), source_map(6, 8)],
        ),
        RootProgram::ServiceOperation => {
            let (provider_contract, _, provider_operation) =
                super::records::contract("example.bytecode-link-provider", "call", false);
            let service_call = ServiceCallRef {
                service_requirement_slot: 7,
                contract_operation_id: provider_operation,
                expected_protocol_identity: provider_contract.service_protocol_identity.clone(),
            };
            (
                vec![0x22, 0, 0, 0, 0, 0x25],
                vec![BytecodeRelocation::ServiceOperationRef {
                    service_call: ServiceCallBoundaryFacts::new(
                        service_call,
                        super::records::service_boundary_plan(),
                    ),
                }],
                Some(service_resume_descriptor()),
                vec![source_map(0, 5)],
            )
        }
        RootProgram::Constant(_) => (vec![0x00, 0, 0x08, 0x25], Vec::new(), None, Vec::new()),
    }
}

pub(super) fn interface_identity() -> String {
    let identity = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: "example.bytecode-link".to_string(),
            },
            symbol_path: format!("{MODULE}.Reader"),
            abi_expectation: None,
        },
    };
    String::from_utf8(skiff_canonical_json::canonical_json_bytes(&identity).unwrap()).unwrap()
}

fn pools(program: RootProgram) -> BytecodePools {
    BytecodePools {
        constants: Vec::new(),
        types: match program {
            RootProgram::Host => vec![type_entry(TypeRefIr::builtin("Date"), snapshot_plan())],
            RootProgram::Intrinsic => vec![
                type_entry(
                    TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    },
                    ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::SnapshotRelease,
                    },
                ),
                type_entry(TypeRefIr::builtin("string"), stream_item_plan()),
            ],
            RootProgram::Interface
            | RootProgram::LocalInterface
            | RootProgram::UnreachableInterface => vec![
                type_entry(TypeRefIr::builtin("string"), stream_item_plan()),
                type_entry(
                    TypeRefIr::AnyInterface {
                        interface: skiff_artifact_model::InterfaceInstantiationRef {
                            interface_abi_id: interface_identity(),
                            canonical_type_args: Vec::new(),
                        },
                    },
                    stream_item_plan(),
                ),
            ],
            RootProgram::ReorderedStreamNextResumePool => vec![
                type_entry(stream_type(), stream_plan()),
                type_entry(item_type(), stream_item_plan()),
            ],
            RootProgram::FromType => {
                vec![type_entry(TypeRefIr::builtin("string"), stream_item_plan())]
            }
            RootProgram::RecordShape => vec![
                type_entry(TypeRefIr::builtin("string"), stream_item_plan()),
                type_entry(
                    TypeRefIr::Record {
                        fields: BTreeMap::from([(
                            "name".to_string(),
                            TypeRefIr::builtin("string"),
                        )]),
                    },
                    ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::SnapshotRelease,
                    },
                ),
            ],
            RootProgram::ArraysMaps => vec![
                type_entry(
                    TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    },
                    ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::SnapshotRelease,
                    },
                ),
                type_entry(TypeRefIr::builtin("string"), stream_item_plan()),
            ],
            RootProgram::StreamNext
            | RootProgram::StreamProducer
            | RootProgram::ReorderedStreamResumePool => vec![
                type_entry(stream_type(), stream_plan()),
                type_entry(item_type(), stream_item_plan()),
            ],
            RootProgram::StreamNextLoop => vec![
                type_entry(number_stream_type(), stream_plan()),
                type_entry(TypeRefIr::builtin("number"), snapshot_plan()),
            ],
            RootProgram::ServiceOperation => {
                let record = super::records::std_service_internal_error_record();
                vec![type_entry(
                    TypeRefIr::PackageSchema {
                        package_id: record.package_id,
                        stable_schema_key: record.stable_schema_key,
                        package_schema_type_id: record.package_schema_type_id,
                    },
                    ValueTransferPlan::SnapshotShare {
                        drop: ValueDropPlan::SnapshotRelease,
                    },
                )]
            }
            RootProgram::SyntheticTarget => vec![
                type_entry(TypeRefIr::builtin("string"), snapshot_plan()),
                type_entry(
                    TypeRefIr::AnyInterface {
                        interface: skiff_artifact_model::InterfaceInstantiationRef {
                            interface_abi_id: interface_identity(),
                            canonical_type_args: Vec::new(),
                        },
                    },
                    snapshot_plan(),
                ),
            ],
            RootProgram::UnreachableCallback => {
                vec![type_entry(TypeRefIr::builtin("string"), snapshot_plan())]
            }
            RootProgram::RemoteInterface => {
                let record = super::records::std_service_internal_error_record();
                vec![
                    type_entry(TypeRefIr::builtin("string"), stream_item_plan()),
                    type_entry(
                        TypeRefIr::AnyInterface {
                            interface: skiff_artifact_model::InterfaceInstantiationRef {
                                interface_abi_id: interface_identity(),
                                canonical_type_args: Vec::new(),
                            },
                        },
                        stream_item_plan(),
                    ),
                    type_entry(
                        TypeRefIr::PackageSchema {
                            package_id: record.package_id,
                            stable_schema_key: record.stable_schema_key,
                            package_schema_type_id: record.package_schema_type_id,
                        },
                        ValueTransferPlan::SnapshotShare {
                            drop: ValueDropPlan::SnapshotRelease,
                        },
                    ),
                ]
            }
            _ => Vec::new(),
        },
        shapes: match program {
            RootProgram::RecordShape => vec![BytecodePoolEntry::ShapeRef {
                shape: skiff_artifact_model::ShapeDeclaration {
                    type_ref: 1,
                    plan: skiff_artifact_model::ValueTransferPlan::SnapshotShare {
                        drop: skiff_artifact_model::ValueDropPlan::SnapshotRelease,
                    },
                    privileged_affine_composite: None,
                    fields: vec![skiff_artifact_model::ShapeFieldDeclaration {
                        name: "name".to_string(),
                        type_ref: 0,
                        plan: stream_item_plan(),
                    }],
                },
            }],
            RootProgram::StreamProducer => vec![BytecodePoolEntry::ShapeRef {
                shape: skiff_artifact_model::ShapeDeclaration {
                    type_ref: 1,
                    plan: stream_item_plan(),
                    privileged_affine_composite: None,
                    fields: Vec::new(),
                },
            }],
            RootProgram::ReorderedStreamResumePool => reordered_stream_shapes(),
            _ => Vec::new(),
        },
        effects: Vec::new(),
        resume: match program {
            RootProgram::UnreachableInterface => vec![BytecodePoolEntry::ResumeDescriptor(
                interface_resume_descriptor(HELPER_FUNCTION),
            )],
            RootProgram::ReorderedStreamNextResumePool => {
                reordered_stream_next_resume_descriptors()
            }
            RootProgram::ReorderedStreamResumePool => reordered_stream_resume_descriptors(),
            _ => root_body(program)
                .2
                .map(BytecodePoolEntry::ResumeDescriptor)
                .into_iter()
                .collect(),
        },
        callback_capture: matches!(
            program,
            RootProgram::SyntheticTarget | RootProgram::UnreachableCallback
        )
        .then(|| {
            BytecodePoolEntry::CallbackCaptureLayout(CallbackCaptureLayout {
                function_key: CALLBACK_FUNCTION.to_string(),
                captures: Vec::new(),
            })
        })
        .into_iter()
        .collect(),
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
        db_operation: None,
    }
}

fn interface_resume_descriptor(function_key: &str) -> ResumeDescriptor {
    interface_resume_descriptor_at(function_key, 2, 8)
}

fn interface_resume_descriptor_at(
    function_key: &str,
    site_pc: u32,
    resume_pc: u32,
) -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: function_key.to_string(),
        site_pc,
        resume_pc,
        end_resume_pc: None,
        expected_stack_height_before_result: 0,
        result_type_refs: vec![0],
        result_plans: vec![stream_item_plan()],
        result_materializations: vec![None],
        emit_stream_item_shape_ref: None,
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn reordered_stream_next_resume_descriptors() -> Vec<BytecodePoolEntry> {
    [
        stream_next_resume_descriptor_at(4, 7, 9),
        stream_next_resume_descriptor_at(0, 3, 9),
    ]
    .into_iter()
    .map(BytecodePoolEntry::ResumeDescriptor)
    .collect()
}

fn host_resume_descriptor() -> ResumeDescriptor {
    host_resume_descriptor_at(0, 5)
}

fn service_resume_descriptor() -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc: 0,
        resume_pc: 5,
        end_resume_pc: None,
        expected_stack_height_before_result: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        result_materializations: Vec::new(),
        emit_stream_item_shape_ref: None,
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn host_resume_descriptor_at(site_pc: u32, resume_pc: u32) -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc,
        resume_pc,
        end_resume_pc: None,
        expected_stack_height_before_result: 0,
        result_type_refs: vec![0],
        result_plans: vec![snapshot_plan()],
        result_materializations: vec![None],
        emit_stream_item_shape_ref: None,
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn stream_next_resume_descriptor() -> ResumeDescriptor {
    stream_next_resume_descriptor_at(0, 3, 5)
}

fn stream_next_resume_descriptor_at(
    site_pc: u32,
    resume_pc: u32,
    end_resume_pc: u32,
) -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc,
        resume_pc,
        end_resume_pc: Some(end_resume_pc),
        expected_stack_height_before_result: 0,
        result_type_refs: vec![1],
        result_plans: vec![stream_item_plan()],
        result_materializations: vec![None],
        emit_stream_item_shape_ref: None,
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
        result_materializations: vec![None],
        emit_stream_item_shape_ref: None,
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn stream_producer_resume_descriptor() -> ResumeDescriptor {
    stream_producer_resume_descriptor_at(2, 4, 0)
}

fn stream_producer_resume_descriptor_at(
    site_pc: u32,
    resume_pc: u32,
    shape_ref: u32,
) -> ResumeDescriptor {
    ResumeDescriptor {
        function_key: ROOT_FUNCTION.to_string(),
        site_pc,
        resume_pc,
        end_resume_pc: None,
        expected_stack_height_before_result: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        result_materializations: Vec::new(),
        emit_stream_item_shape_ref: Some(shape_ref),
        error_mode: ResumeErrorMode::RaiseAtSite,
    }
}

fn reordered_stream_resume_descriptors() -> Vec<BytecodePoolEntry> {
    [
        stream_producer_resume_descriptor_at(6, 8, 1),
        stream_producer_resume_descriptor_at(2, 4, 0),
    ]
    .into_iter()
    .map(BytecodePoolEntry::ResumeDescriptor)
    .collect()
}

fn reordered_stream_shapes() -> Vec<BytecodePoolEntry> {
    [vec!["alpha"], vec!["beta", "gamma"]]
        .into_iter()
        .map(|fields| BytecodePoolEntry::ShapeRef {
            shape: skiff_artifact_model::ShapeDeclaration {
                type_ref: 1,
                plan: stream_item_plan(),
                privileged_affine_composite: None,
                fields: fields
                    .into_iter()
                    .map(|name| skiff_artifact_model::ShapeFieldDeclaration {
                        name: name.to_string(),
                        type_ref: 1,
                        plan: stream_item_plan(),
                    })
                    .collect(),
            },
        })
        .collect()
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
        RootProgram::ArraysMaps | RootProgram::Intrinsic | RootProgram::RecordShape => {
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }
        }
        _ => snapshot_plan(),
    }
}

fn snapshot_plan() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    }
}

fn type_entry(ty: TypeRefIr, plan: ValueTransferPlan) -> BytecodePoolEntry {
    BytecodePoolEntry::TypeRef {
        ty,
        representation_carrier: None,
        plan,
    }
}

fn coordinate(executable_index: u32) -> PackageExecutableCoordinate {
    PackageExecutableCoordinate {
        file_ir_identity: "file-ir:fixture".to_string(),
        module_path: MODULE.to_string(),
        executable_index,
    }
}
