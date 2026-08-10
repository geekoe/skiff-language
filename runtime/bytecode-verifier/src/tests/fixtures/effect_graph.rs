use std::{collections::BTreeMap, sync::Arc};

mod identities;
mod inout;
mod resume;
mod statements;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeFunctionOrigin,
    BytecodeImage, BytecodeRelocation, BytecodeSpecialization, CallableEffectSummary,
    CallableMayEffects, CallableProvenanceSummary, CallableProvenanceUnknownReason,
    CallableSemanticFacts, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentRevision, FileIrRef, FrozenConstantGraph, OperationCallableKind, OperationTargetRef,
    PackageArtifact, PackageBuildId, PackageCallableLinkFact, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRuntimeRequirements,
    PackageSchemaIndexRef, PackageTypeRef, RelocatableBytecodeFunction, ServiceDeployment,
    TypeRefIr, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedCallableEffectDeclaration,
    LinkedExactLocalTarget, LinkedFunction, LinkedFunctionTables, LinkedInstruction,
    LinkedInstructionTarget, LinkedProgramPointState, LinkedResolvedOperand, LinkedSourceMapEntry,
    LinkedStackMapCandidate, LinkedStatementEntry, SpecializationKey,
};
use skiff_runtime_loader::{DeploymentBytecodeLoader, HydratedDeploymentBytecode};

use self::identities::{callable, coordinate, function_key, specialization, symbol};
use super::{bytecode_statement_manifest_identity, candidate_parts, contract, ExactResolver};

#[derive(Clone)]
pub(crate) struct EffectGraphFunction {
    pub(crate) summary: CallableEffectSummary,
    pub(crate) may_suspend: bool,
    pub(crate) target: Option<u32>,
    pub(crate) call_kind: EffectGraphCallKind,
    pub(crate) trailing_return: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum EffectGraphCallKind {
    Ordinary,
    Tail,
    Resume,
    InOut,
}

pub(crate) fn analyzed(effects: CallableMayEffects) -> CallableEffectSummary {
    CallableEffectSummary::Analyzed { effects }
}

pub(crate) fn bottom() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

pub(crate) fn loader_backed_effect_graph(
    functions: Vec<EffectGraphFunction>,
) -> (HydratedDeploymentBytecode, LinkedBytecodeCandidate) {
    let bytecode = admitted_bytecode(&functions);
    let package = package(bytecode.as_ref(), &functions);
    let service_contract = contract();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(&service_contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:effect-graph-test"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(&package).unwrap(),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "effect graph test".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let package_reference = skiff_artifact_identity::package_artifact_ref(&package).unwrap();
    let resolver = ExactResolver {
        deployment: Arc::new(deployment),
        contract: Arc::new(service_contract),
        packages: BTreeMap::from([(package_reference.clone(), Arc::new(package))]),
        bytecodes: BTreeMap::from([(package_reference, bytecode)]),
    };
    let hydrated = DeploymentBytecodeLoader::new(&resolver)
        .load(&reference)
        .unwrap();
    let candidate = linked_candidate(&hydrated, &functions);
    (hydrated, candidate)
}

fn admitted_bytecode(functions: &[EffectGraphFunction]) -> Arc<ValidatedBytecodeArtifact> {
    let image_functions = functions
        .iter()
        .enumerate()
        .map(|(ordinal, function)| (function_key(ordinal), artifact_function(ordinal, function)))
        .collect();
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
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions: image_functions,
            pools: inout::artifact_pools(resume::artifact_pools(functions), functions),
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn artifact_function(
    ordinal: usize,
    function: &EffectGraphFunction,
) -> RelocatableBytecodeFunction {
    let (mut words, relocations) = match (function.call_kind, function.target) {
        (EffectGraphCallKind::Ordinary, Some(target)) => (
            vec![0x20, 0, 0, 0, 0x14, 0x25],
            vec![BytecodeRelocation::LocalExecutableRef {
                function_key: function_key(target as usize),
                specialization: BytecodeSpecialization {
                    type_arguments: Vec::new(),
                    concrete_receiver: None,
                },
            }],
        ),
        (EffectGraphCallKind::Tail, Some(target)) => (
            vec![0x21, 0, 0],
            vec![BytecodeRelocation::LocalExecutableRef {
                function_key: function_key(target as usize),
                specialization: BytecodeSpecialization {
                    type_arguments: Vec::new(),
                    concrete_receiver: None,
                },
            }],
        ),
        (EffectGraphCallKind::Resume, None) => (vec![0x61, 0, 0x25], Vec::new()),
        (EffectGraphCallKind::InOut, Some(target)) => (
            vec![0x26, 0, 0, 0, 0, 0x25],
            vec![BytecodeRelocation::LocalExecutableRef {
                function_key: function_key(target as usize),
                specialization: BytecodeSpecialization {
                    type_arguments: Vec::new(),
                    concrete_receiver: None,
                },
            }],
        ),
        (EffectGraphCallKind::Ordinary, None)
        | (EffectGraphCallKind::Tail, None)
        | (EffectGraphCallKind::InOut, None) => (vec![0x25], Vec::new()),
        (EffectGraphCallKind::Resume, Some(_)) => panic!("resume fixture cannot have a target"),
    };
    if function.trailing_return && matches!(function.call_kind, EffectGraphCallKind::Tail) {
        words.push(0x25);
    }
    RelocatableBytecodeFunction {
        function_key: function_key(ordinal),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(ordinal),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words,
        relocations,
        call_loan_layouts: inout::artifact_loan_layouts(function.call_kind),
        frame_layout: inout::artifact_frame(function.call_kind),
        max_operand_depth: resume::max_operand_depth(function.call_kind),
        effect_summary_ref: callable(ordinal),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: function
            .target
            .map_or_else(Vec::new, |_| match function.call_kind {
                EffectGraphCallKind::Ordinary => statements::artifact_entries(),
                EffectGraphCallKind::Tail => statements::artifact_tail_entries(),
                EffectGraphCallKind::Resume => Vec::new(),
                EffectGraphCallKind::InOut => statements::artifact_call_only_entries(),
            }),
        source_map: match function.call_kind {
            EffectGraphCallKind::Resume => statements::artifact_resume_source_map(),
            EffectGraphCallKind::Ordinary => function
                .target
                .map_or_else(Vec::new, |_| statements::artifact_source_map()),
            EffectGraphCallKind::Tail => function
                .target
                .map_or_else(Vec::new, |_| statements::artifact_tail_source_map()),
            EffectGraphCallKind::InOut => function
                .target
                .map_or_else(Vec::new, |_| statements::artifact_inout_source_map()),
        },
    }
}

fn package(
    bytecode: &ValidatedBytecodeArtifact,
    functions: &[EffectGraphFunction],
) -> PackageArtifact {
    let package_id = "example.effect-graph";
    let file = FileIrRef::new("file-ir:effect-graph", "fixture");
    let public_symbols = BTreeMap::new();
    let implementation_symbols = functions
        .iter()
        .enumerate()
        .map(|(ordinal, function)| {
            (
                symbol(ordinal),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable(ordinal),
                    signature: skiff_artifact_model::PackageCallableSignature {
                        type_params: Vec::new(),
                        parameters: Vec::new(),
                        return_type: PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("void"),
                        },
                        may_suspend: function.may_suspend,
                    },
                },
            )
        })
        .collect();
    let callable_links = functions
        .iter()
        .enumerate()
        .map(|(ordinal, _)| {
            let callable_id = callable(ordinal);
            (
                callable_id.clone(),
                PackageCallableLinkFact {
                    callable_id: callable_id.clone(),
                    target: OperationTargetRef {
                        file_ref: file.clone(),
                        executable_index: ordinal as u32,
                        callable_abi_id: callable_id.as_str().to_string(),
                        callable_kind: OperationCallableKind::InternalFunction,
                    },
                },
            )
        })
        .collect();
    let semantic_facts = functions
        .iter()
        .enumerate()
        .map(|(ordinal, function)| {
            (
                callable(ordinal),
                CallableSemanticFacts {
                    effects: function.summary.clone(),
                    provenance: CallableProvenanceSummary::Unknown {
                        reason: CallableProvenanceUnknownReason::AnalysisPending,
                    },
                    resolved_call_targets: BTreeMap::new(),
                },
            )
        })
        .collect();
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file],
        static_resources: Vec::new(),
        bytecode: Some(bytecode.reference().clone()),
        bytecode_statement_manifest_identity: bytecode_statement_manifest_identity(
            package_id,
            Some(bytecode),
        ),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols,
            implementation_symbols,
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .unwrap(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links,
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: semantic_facts,
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn linked_candidate(
    hydrated: &HydratedDeploymentBytecode,
    functions: &[EffectGraphFunction],
) -> LinkedBytecodeCandidate {
    let package = hydrated.packages().values().next().unwrap();
    let build = package.reference().package_build_id.clone();
    let keys = (0..functions.len())
        .map(|ordinal| specialization(&build, ordinal))
        .collect::<Vec<_>>();
    let linked_functions = functions
        .iter()
        .enumerate()
        .map(|(ordinal, function)| linked_function(ordinal, &keys, function))
        .collect();
    let mut parts = candidate_parts(hydrated, None, None);
    parts.functions = linked_functions;
    parts.exact_local_targets = keys
        .into_iter()
        .enumerate()
        .map(|(ordinal, key)| LinkedExactLocalTarget::new(key, FunctionIndex::new(ordinal as u32)))
        .collect();
    parts.resume_sites = resume::linked_sites(functions);
    inout::extend_linked_parts(&mut parts, &build, functions);
    LinkedBytecodeCandidate::try_from_parts(parts).unwrap()
}

fn linked_function(
    ordinal: usize,
    keys: &[SpecializationKey],
    function: &EffectGraphFunction,
) -> LinkedFunction {
    let mut instructions = match (function.call_kind, function.target) {
        (EffectGraphCallKind::Ordinary, Some(target)) => {
            vec![linked_call(target), linked_budget(), linked_return(5)]
        }
        (EffectGraphCallKind::Tail, Some(target)) => vec![linked_tail_call(target)],
        (EffectGraphCallKind::Resume, None) => vec![resume::linked_emit_stream(), linked_return(2)],
        (EffectGraphCallKind::InOut, Some(target)) => {
            vec![inout::linked_call(target), linked_return(5)]
        }
        (EffectGraphCallKind::Ordinary, None)
        | (EffectGraphCallKind::Tail, None)
        | (EffectGraphCallKind::InOut, None) => {
            vec![linked_return(0)]
        }
        (EffectGraphCallKind::Resume, Some(_)) => panic!("resume fixture cannot have a target"),
    };
    if function.trailing_return && matches!(function.call_kind, EffectGraphCallKind::Tail) {
        instructions.push(linked_return(3));
    }
    let states = (0..instructions.len())
        .map(|instruction| {
            LinkedProgramPointState::new(
                InstructionIndex::new(instruction as u32),
                Box::new([]),
                inout::linked_slot_states(function.call_kind),
                Box::new([]),
                Box::new([]),
            )
        })
        .collect::<Vec<_>>();
    let max_operand_depth = resume::max_operand_depth(function.call_kind);
    let stack_map = LinkedStackMapCandidate::try_new(
        states.into_boxed_slice(),
        instructions.len(),
        inout::slot_count(function.call_kind),
        max_operand_depth,
    )
    .unwrap();
    let statement_entries: Box<[LinkedStatementEntry]> = match (function.call_kind, function.target)
    {
        (EffectGraphCallKind::Ordinary, Some(_)) => statements::linked_entries(),
        (EffectGraphCallKind::Tail, Some(_)) => statements::linked_tail_entries(),
        (EffectGraphCallKind::InOut, Some(_)) => statements::linked_call_only_entries(),
        (EffectGraphCallKind::Resume, None)
        | (EffectGraphCallKind::Ordinary, None)
        | (EffectGraphCallKind::Tail, None)
        | (EffectGraphCallKind::InOut, None) => Vec::new().into_boxed_slice(),
        (EffectGraphCallKind::Resume, Some(_)) => panic!("resume fixture cannot have a target"),
    };
    let source_map: Box<[LinkedSourceMapEntry]> = match (function.call_kind, function.target) {
        (EffectGraphCallKind::Resume, None) => statements::linked_resume_source_map(),
        (EffectGraphCallKind::Ordinary, Some(_)) => statements::linked_source_map(),
        (EffectGraphCallKind::Tail, Some(_)) => statements::linked_tail_source_map(),
        (EffectGraphCallKind::InOut, Some(_)) => statements::linked_inout_source_map(),
        (EffectGraphCallKind::Ordinary, None)
        | (EffectGraphCallKind::Tail, None)
        | (EffectGraphCallKind::InOut, None) => Vec::new().into_boxed_slice(),
        (EffectGraphCallKind::Resume, Some(_)) => panic!("resume fixture cannot have a target"),
    };
    LinkedFunction::new(
        FunctionIndex::new(ordinal as u32),
        keys[ordinal].clone(),
        instructions.into_boxed_slice(),
        inout::linked_frame(function.call_kind),
        max_operand_depth,
        LinkedCallableEffectDeclaration::new(callable(ordinal), function.summary.clone()),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            inout::linked_loan_layouts(function.call_kind),
            statement_entries,
            source_map,
        ),
        stack_map,
    )
}

fn linked_call(target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::CallLocal,
        Box::new([0, 0, 0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(target)),
        )]),
        0,
    )
    .unwrap()
}

fn linked_tail_call(target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::TailCallLocal,
        Box::new([0, 0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(target)),
        )]),
        0,
    )
    .unwrap()
}

fn linked_budget() -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::BudgetCheckpoint,
        Box::new([]),
        Box::new([]),
        4,
    )
    .unwrap()
}

fn linked_return(pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::Return,
        Box::new([]),
        Box::new([]),
        pc,
    )
    .unwrap()
}
