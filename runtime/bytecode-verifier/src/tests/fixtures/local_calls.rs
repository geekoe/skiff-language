use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeFunctionOrigin,
    BytecodeImage, BytecodePools, BytecodeRelocation, BytecodeSpecialization,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
    CallableProvenanceUnknownReason, CallableSemanticFacts, DeploymentArtifactIdentity,
    DeploymentDiagnosticText, DeploymentRevision, FileIrRef, FrameLayout, FrozenConstantGraph,
    InstructionSourceSite, OperationCallableKind, OperationTargetRef, PackageArtifact,
    PackageCallableId, PackageCallableLinkFact, PackageCallableSignature,
    PackageExecutableCoordinate, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRuntimeRequirements,
    PackageSchemaIndexRef, PackageTypeRef, RelocatableBytecodeFunction, ServiceDeployment,
    SourceMapEntry, StatementChargeKind, StatementEntry, SyntheticInstructionSiteReason, TypeRefIr,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, FunctionIndex, InstructionBoundaryIndex, InstructionIndex,
    LinkedBytecodeCandidate, LinkedCallableEffectDeclaration, LinkedExactLocalTarget,
    LinkedFrameLayout, LinkedFunction, LinkedFunctionTables, LinkedInstruction,
    LinkedInstructionTarget, LinkedProgramPointState, LinkedResolvedOperand, LinkedSourceMapEntry,
    LinkedStackMapCandidate, LinkedStatementEntry, SpecializationKey,
};
use skiff_runtime_loader::{DeploymentBytecodeLoader, HydratedDeploymentBytecode};

use super::{candidate_parts, contract, ExactResolver};

const CALLER_FUNCTION: &str = "fixture::caller";
const TARGET_FUNCTION: &str = "fixture::target";
const CALLER_CALLABLE: &str = "pkg-callable:example.local-authority:top-level:fixture.caller";
const TARGET_CALLABLE: &str = "pkg-callable:example.local-authority:top-level:fixture.target";

pub(in crate::tests) const TARGET_FUNCTION_INDEX: FunctionIndex = FunctionIndex::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::tests) enum LocalCallCandidateCorruption {
    None,
    TargetDeclarativeSummary,
    TargetEffectOwner,
    TargetCanonicalFunction,
}

pub(in crate::tests) fn loader_backed_local_call(
    corruption: LocalCallCandidateCorruption,
) -> (HydratedDeploymentBytecode, LinkedBytecodeCandidate) {
    let bytecode = admitted_bytecode();
    let package = package(bytecode.reference().clone());
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
    let resolver = ExactResolver {
        deployment: Arc::new(deployment),
        contract: Arc::new(contract),
        package: Arc::new(package),
        bytecode,
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
            pools: BytecodePools::default(),
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
        words: vec![0x20, 0, 0, 0, 0x25],
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
        statement_entries: vec![StatementEntry {
            pc: 0,
            statement_id: "fixture:caller:entry".to_string(),
            charge_kind: StatementChargeKind::FunctionEntry,
        }],
        source_map: vec![SourceMapEntry {
            start_pc: 0,
            end_pc: 4,
            site: call_site(),
        }],
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
        statement_entries: vec![StatementEntry {
            pc: 0,
            statement_id: "fixture:target:entry".to_string(),
            charge_kind: StatementChargeKind::FunctionEntry,
        }],
        source_map: Vec::new(),
    }
}

fn package(bytecode: skiff_artifact_model::BytecodeArtifactRef) -> PackageArtifact {
    let package_id = "example.local-authority";
    let caller = callable(CALLER_CALLABLE);
    let target = callable(TARGET_CALLABLE);
    let file = FileIrRef::new("file-ir:local-authority", "fixture");
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: skiff_artifact_model::PackageBuildId::new("unassigned"),
        files: vec![file.clone()],
        static_resources: Vec::new(),
        bytecode: Some(bytecode),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::from([
                (
                    "fixture.caller".to_string(),
                    callable_symbol(caller.clone()),
                ),
                (
                    "fixture.target".to_string(),
                    callable_symbol(target.clone()),
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
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::from([
            (
                caller.clone(),
                callable_link(caller.clone(), file.clone(), 0),
            ),
            (target.clone(), callable_link(target.clone(), file, 1)),
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
            (caller, semantic_facts()),
            (target, semantic_facts()),
        ]),
        boundary_projections: BTreeMap::new(),
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
        analyzed_no_effects()
    } else {
        canonical_summary()
    };
    let functions = vec![
        linked_function(
            FunctionIndex::new(0),
            caller_key.clone(),
            vec![linked_call(), linked_return(4)],
            callable(CALLER_CALLABLE),
            canonical_summary(),
            vec![LinkedSourceMapEntry::new(
                InstructionIndex::new(0),
                InstructionBoundaryIndex::new(1),
                call_site(),
            )],
        ),
        linked_function(
            TARGET_FUNCTION_INDEX,
            target_key.clone(),
            vec![linked_return(0)],
            target_effect_owner,
            target_summary,
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
    source_map: Vec<LinkedSourceMapEntry>,
) -> LinkedFunction {
    let statement_id = if index == TARGET_FUNCTION_INDEX {
        "fixture:target:entry"
    } else {
        "fixture:caller:entry"
    };
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
            Box::new([LinkedStatementEntry::new(
                InstructionIndex::new(0),
                statement_id,
                StatementChargeKind::FunctionEntry,
            )
            .unwrap()]),
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

fn callable_symbol(callable_id: PackageCallableId) -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::Callable {
        callable_id,
        signature: PackageCallableSignature {
            type_params: Vec::new(),
            parameters: Vec::new(),
            return_type: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("void"),
            },
            may_suspend: false,
        },
    }
}

fn semantic_facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: canonical_summary(),
        provenance: CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::AnalysisPending,
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn canonical_summary() -> CallableEffectSummary {
    CallableEffectSummary::analysis_pending()
}

fn analyzed_no_effects() -> CallableEffectSummary {
    CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
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

fn call_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    }
}

fn callable(value: &str) -> PackageCallableId {
    PackageCallableId::new(value)
}
