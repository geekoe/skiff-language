mod effects;
mod statements;
mod tail_matrix;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeFunctionOrigin,
    BytecodeImage, BytecodePoolEntry, BytecodePools, BytecodeRelocation, BytecodeSpecialization,
    CallableEffectSummary, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentRevision, FileIrRef, FrameLayout, FrozenConstantGraph, OperationCallableKind,
    OperationTargetRef, PackageArtifact, PackageCallableId, PackageCallableLinkFact,
    PackageExecutableCoordinate, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexRef,
    RelocatableBytecodeFunction, ServiceDeployment, TypeRefIr, BYTECODE_ISA_VERSION,
    BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, FunctionIndex, InstructionIndex, LinkedBytecodeCandidate,
    LinkedCallableEffectDeclaration, LinkedExactLocalTarget, LinkedFrameLayout, LinkedFunction,
    LinkedFunctionTables, LinkedInstruction, LinkedInstructionTarget, LinkedProgramPointState,
    LinkedResolvedOperand, LinkedSourceMapEntry, LinkedStackMapCandidate, LinkedStatementEntry,
    SpecializationKey,
};
use skiff_runtime_loader::{DeploymentBytecodeLoader, HydratedDeploymentBytecode};

use super::{bytecode_statement_manifest_identity, candidate_parts, contract, ExactResolver};

pub(crate) use tail_matrix::{loader_backed_tail_case, TailMatrixCase, TailMatrixFixture};

const CALLER_FUNCTION: &str = "fixture::caller";
const TARGET_FUNCTION: &str = "fixture::target";
const CALLER_CALLABLE: &str = "pkg-callable:example.local-authority:top-level:fixture.caller";
const TARGET_CALLABLE: &str = "pkg-callable:example.local-authority:top-level:fixture.target";

pub(in crate::tests) const TARGET_FUNCTION_INDEX: FunctionIndex = FunctionIndex::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalCallCandidateCorruption {
    None,
    TargetDeclarativeSummary,
    TargetEffectOwner,
    TargetCanonicalFunction,
    TargetAnalyzedNoPending,
    TargetAnalyzedMayPendingMismatch,
    TargetAnalyzedDuplicateCategory,
    TargetAnalyzedAbiMaySuspendMismatch,
    TargetAbiAliasMaySuspendDrift,
    TargetAliasSemanticSummaryDrift,
    StatementInstruction,
    StatementSequence,
    StatementAttributionId,
    StatementSite,
}

pub(crate) fn loader_backed_local_call(
    corruption: LocalCallCandidateCorruption,
) -> (HydratedDeploymentBytecode, LinkedBytecodeCandidate) {
    let bytecode = admitted_bytecode();
    let package = package(bytecode.as_ref(), corruption);
    let contract = contract();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(&contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:local-authority-test"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(&package).unwrap(),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "local authority test".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let package_reference = skiff_artifact_identity::package_artifact_ref(&package).unwrap();
    let resolver = ExactResolver {
        deployment: Arc::new(deployment),
        contract: Arc::new(contract),
        packages: BTreeMap::from([(package_reference.clone(), Arc::new(package))]),
        bytecodes: BTreeMap::from([(package_reference, bytecode)]),
    };
    let hydrated = DeploymentBytecodeLoader::new(&resolver)
        .load(&reference)
        .unwrap();
    let candidate = candidate(&hydrated, corruption);
    (hydrated, candidate)
}

fn admitted_bytecode() -> Arc<ValidatedBytecodeArtifact> {
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
            functions: BTreeMap::from([
                (CALLER_FUNCTION.to_string(), caller_artifact_function()),
                (TARGET_FUNCTION.to_string(), target_artifact_function()),
            ]),
            pools: BytecodePools {
                types: vec![
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("string"),
                    },
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("string"),
                    },
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("bytes"),
                    },
                ],
                ..BytecodePools::default()
            },
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn caller_artifact_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: CALLER_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(0),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x20, 0, 0, 0, 0x14, 0x25],
        relocations: vec![BytecodeRelocation::LocalExecutableRef {
            function_key: TARGET_FUNCTION.to_string(),
            specialization: BytecodeSpecialization {
                type_arguments: Vec::new(),
                concrete_receiver: None,
            },
        }],
        call_loan_layouts: Vec::new(),
        frame_layout: empty_artifact_frame(),
        max_operand_depth: 0,
        effect_summary_ref: callable(CALLER_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: statements::artifact_entries(),
        source_map: statements::artifact_source_map(),
    }
}

fn target_artifact_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: TARGET_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(1),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x25],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: empty_artifact_frame(),
        max_operand_depth: 0,
        effect_summary_ref: callable(TARGET_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn package(
    bytecode: &ValidatedBytecodeArtifact,
    corruption: LocalCallCandidateCorruption,
) -> PackageArtifact {
    package_with_caller_summary(bytecode, corruption, effects::canonical_summary())
}

fn package_with_caller_summary(
    bytecode: &ValidatedBytecodeArtifact,
    corruption: LocalCallCandidateCorruption,
    caller_summary: CallableEffectSummary,
) -> PackageArtifact {
    let package_id = "example.local-authority";
    let caller = callable(CALLER_CALLABLE);
    let target = callable(TARGET_CALLABLE);
    let target_summary = effects::target_summary(corruption);
    let file = FileIrRef::new("file-ir:local-authority", "fixture");
    let effects::PublicCallableAuthority {
        callable_id: target_alias,
        symbol: target_alias_symbol,
        callable_link: target_alias_link,
        implementation_export: target_implementation_export,
        semantic_facts: target_alias_facts,
        boundary_projection: target_alias_projection,
    } = effects::target_alias_authority(corruption, &target_summary, &file, 1);
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: skiff_artifact_model::PackageBuildId::new("unassigned"),
        files: vec![file.clone()],
        static_resources: Vec::new(),
        bytecode: Some(bytecode.reference().clone()),
        bytecode_statement_manifest_identity: bytecode_statement_manifest_identity(
            package_id,
            Some(bytecode),
        ),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([("fixture.target".to_string(), target_alias_symbol)]),
            implementation_symbols: BTreeMap::from([
                (
                    "fixture.caller".to_string(),
                    effects::callable_symbol(caller.clone()),
                ),
                (
                    "fixture.target".to_string(),
                    effects::callable_symbol(target.clone()),
                ),
            ]),
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
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::new(),
            constants: BTreeMap::new(),
            functions: BTreeMap::from([(
                "fixture.target".to_string(),
                target_implementation_export,
            )]),
            impl_methods: BTreeMap::new(),
            operation_targets: BTreeMap::new(),
        },
        callable_links: BTreeMap::from([
            (
                caller.clone(),
                callable_link(caller.clone(), file.clone(), 0),
            ),
            (target.clone(), callable_link(target.clone(), file, 1)),
            (target_alias.clone(), target_alias_link),
        ]),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::from([
            (caller, effects::semantic_facts(caller_summary)),
            (target, effects::semantic_facts(target_summary.clone())),
            (target_alias.clone(), target_alias_facts),
        ]),
        boundary_projections: BTreeMap::from([(target_alias, target_alias_projection)]),
        service_call_refs: Vec::new(),
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn candidate(
    hydrated: &HydratedDeploymentBytecode,
    corruption: LocalCallCandidateCorruption,
) -> LinkedBytecodeCandidate {
    let package = hydrated.packages().values().next().unwrap();
    let build = package.reference().package_build_id.clone();
    let caller_key = specialization(&build, CALLER_FUNCTION, CALLER_CALLABLE);
    let target_template = if corruption == LocalCallCandidateCorruption::TargetCanonicalFunction {
        CALLER_CALLABLE
    } else {
        TARGET_CALLABLE
    };
    let target_key = specialization(&build, TARGET_FUNCTION, target_template);
    let target_effect_owner = if corruption == LocalCallCandidateCorruption::TargetEffectOwner {
        callable(CALLER_CALLABLE)
    } else {
        callable(TARGET_CALLABLE)
    };
    let target_summary = if corruption == LocalCallCandidateCorruption::TargetDeclarativeSummary {
        effects::analyzed_no_effects()
    } else {
        package
            .artifact()
            .callable_semantic_facts
            .get(&callable(TARGET_CALLABLE))
            .unwrap()
            .effects
            .clone()
    };
    let functions = vec![
        linked_function(
            FunctionIndex::new(0),
            caller_key.clone(),
            vec![linked_call(), linked_budget(), linked_return(5)],
            callable(CALLER_CALLABLE),
            effects::canonical_summary(),
            statements::linked_entries(corruption),
            statements::linked_source_map(),
        ),
        linked_function(
            TARGET_FUNCTION_INDEX,
            target_key.clone(),
            vec![linked_return(0)],
            target_effect_owner,
            target_summary,
            Box::new([]),
            Vec::new(),
        ),
    ];
    let mut parts = candidate_parts(hydrated, None, None);
    parts.functions = functions;
    parts.exact_local_targets = vec![
        LinkedExactLocalTarget::new(caller_key, FunctionIndex::new(0)),
        LinkedExactLocalTarget::new(target_key, TARGET_FUNCTION_INDEX),
    ];
    LinkedBytecodeCandidate::try_from_parts(parts).unwrap()
}

fn linked_function(
    index: FunctionIndex,
    key: SpecializationKey,
    instructions: Vec<LinkedInstruction>,
    effect_owner: PackageCallableId,
    effect_summary: CallableEffectSummary,
    statement_entries: Box<[LinkedStatementEntry]>,
    source_map: Vec<LinkedSourceMapEntry>,
) -> LinkedFunction {
    let states = (0..instructions.len())
        .map(|instruction| {
            LinkedProgramPointState::new(
                InstructionIndex::new(u32::try_from(instruction).unwrap()),
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
            )
        })
        .collect::<Vec<_>>();
    let stack_map =
        LinkedStackMapCandidate::try_new(states.into_boxed_slice(), instructions.len(), 0, 0)
            .unwrap();
    LinkedFunction::new(
        index,
        key,
        instructions.into_boxed_slice(),
        empty_linked_frame(),
        0,
        LinkedCallableEffectDeclaration::new(effect_owner, effect_summary),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            statement_entries,
            source_map.into_boxed_slice(),
        ),
        stack_map,
    )
}

fn linked_call() -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::CallLocal,
        Box::new([0, 0, 0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(TARGET_FUNCTION_INDEX),
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

fn specialization(
    build: &skiff_artifact_model::PackageBuildId,
    function: &str,
    callable_id: &str,
) -> SpecializationKey {
    SpecializationKey::new(
        build.clone(),
        ArtifactFunctionKey::parse(function).unwrap(),
        callable(callable_id),
        Box::new([]),
        None,
    )
}

fn callable_link(
    callable_id: PackageCallableId,
    file_ref: FileIrRef,
    executable_index: u32,
) -> PackageCallableLinkFact {
    PackageCallableLinkFact {
        callable_id: callable_id.clone(),
        target: OperationTargetRef {
            file_ref,
            executable_index,
            callable_abi_id: callable_id.as_str().to_string(),
            callable_kind: OperationCallableKind::InternalFunction,
        },
    }
}

fn empty_artifact_frame() -> FrameLayout {
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

fn empty_linked_frame() -> LinkedFrameLayout {
    LinkedFrameLayout::new(
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
    )
    .unwrap()
}

fn coordinate(executable_index: u32) -> PackageExecutableCoordinate {
    PackageExecutableCoordinate {
        file_ir_identity: "file-ir:local-authority".to_string(),
        module_path: "fixture".to_string(),
        executable_index,
    }
}

fn callable(value: &str) -> PackageCallableId {
    PackageCallableId::new(value)
}
