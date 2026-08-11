mod linked;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifact, BytecodeFunctionOrigin,
    BytecodeImage, BytecodePoolEntry, BytecodePools, BytecodeRelocation, BytecodeSpecialization,
    DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentRevision, FrameLayout,
    FrozenConstantGraph, InstructionSourceSite, PackageArtifact, PackageCallableParameter,
    PackageCallableSignature, PackageLocalAbiSymbol, PackageTypeRef, ParamIr, ParamModeIr,
    ParameterSlotDecl, RelocatableBytecodeFunction, ServiceDeployment, SourceMapEntry,
    StatementAttributionId, StatementEntry, SyntheticInstructionSiteReason, TypeRefIr,
    ValueDropPlan, ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{InstructionIndex, LinkedBytecodeCandidate};
use skiff_runtime_loader::{DeploymentBytecodeLoader, HydratedDeploymentBytecode};

use super::{
    callable, coordinate, package_with_caller_summary, LocalCallCandidateCorruption,
    CALLER_CALLABLE, CALLER_FUNCTION, TARGET_CALLABLE, TARGET_FUNCTION,
};
use crate::tests::fixtures::{contract, ExactResolver};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TailMatrixCase {
    LiveCleanup,
    MovedAndUninitialized,
    StackResidue,
    ArgumentClassCorruption,
    ArgumentPlanCorruption,
    ResultPlanCorruption,
}

pub(crate) struct TailMatrixFixture {
    pub(crate) hydrated: HydratedDeploymentBytecode,
    pub(crate) candidate: LinkedBytecodeCandidate,
    pub(crate) tail_instruction: InstructionIndex,
}

pub(crate) fn loader_backed_tail_case(case: TailMatrixCase) -> TailMatrixFixture {
    let bytecode = admitted_bytecode(case);
    let package = tail_package(bytecode.as_ref(), case);
    let contract = contract();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(&contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:tail-matrix-test"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(&package).unwrap(),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "tail matrix test".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
    let package_ref = skiff_artifact_identity::package_artifact_ref(&package).unwrap();
    let resolver = ExactResolver {
        deployment: Arc::new(deployment),
        contract: Arc::new(contract),
        packages: BTreeMap::from([(package_ref.clone(), Arc::new(package))]),
        bytecodes: BTreeMap::from([(package_ref, bytecode)]),
    };
    let hydrated = DeploymentBytecodeLoader::new(&resolver)
        .load(&deployment_ref)
        .unwrap();
    let candidate = linked::candidate(&hydrated, case);
    TailMatrixFixture {
        hydrated,
        candidate,
        tail_instruction: InstructionIndex::new(case.tail_instruction()),
    }
}

fn tail_package(bytecode: &ValidatedBytecodeArtifact, case: TailMatrixCase) -> PackageArtifact {
    let mut package = package_with_caller_summary(
        bytecode,
        LocalCallCandidateCorruption::TargetAnalyzedNoPending,
        super::effects::analyzed_no_effects(),
    );
    let caller_signature = package_signature(case.caller_parameter_types(), case.caller_results());
    let target_signature = package_signature(case.target_slots(), case.target_results());
    replace_signature(
        package
            .package_local_abi
            .implementation_symbols
            .get_mut("fixture.caller")
            .unwrap(),
        caller_signature,
    );
    replace_signature(
        package
            .package_local_abi
            .implementation_symbols
            .get_mut("fixture.target")
            .unwrap(),
        target_signature.clone(),
    );
    replace_signature(
        package
            .package_local_abi
            .public_symbols
            .get_mut("fixture.target")
            .unwrap(),
        target_signature,
    );
    package
        .implementation_links
        .functions
        .get_mut("fixture.target")
        .unwrap()
        .signature = skiff_artifact_model::ExecutableSignatureIr {
        params: case
            .target_slots()
            .into_iter()
            .enumerate()
            .map(|(ordinal, ty)| ParamIr {
                name: format!("arg{ordinal}"),
                slot: u32::try_from(ordinal).unwrap(),
                ty: type_ref(ty),
                mode: ParamModeIr::Value,
            })
            .collect(),
        return_type: return_type(&case.target_results()),
        self_type: None,
        may_suspend: false,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
    package
}

fn package_signature(parameter_types: Vec<u32>, results: Vec<u32>) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: parameter_types
            .into_iter()
            .enumerate()
            .map(|(ordinal, ty)| PackageCallableParameter {
                name: format!("arg{ordinal}"),
                ty: PackageTypeRef::Local {
                    local_type: type_ref(ty),
                },
                mode: ParamModeIr::Value,
            })
            .collect(),
        return_type: PackageTypeRef::Local {
            local_type: return_type(&results),
        },
        may_suspend: false,
    }
}

fn replace_signature(symbol: &mut PackageLocalAbiSymbol, replacement: PackageCallableSignature) {
    let PackageLocalAbiSymbol::Callable { signature, .. } = symbol else {
        panic!("tail fixture callable symbol changed kind")
    };
    *signature = replacement;
}

fn return_type(results: &[u32]) -> TypeRefIr {
    match results {
        [] => TypeRefIr::builtin("void"),
        [ty] => type_ref(*ty),
        _ => panic!("tail fixture supports at most one result"),
    }
}

fn type_ref(ty: u32) -> TypeRefIr {
    match ty {
        0 | 1 => TypeRefIr::builtin("string"),
        2 => TypeRefIr::builtin("bytes"),
        _ => panic!("tail fixture type index is out of bounds"),
    }
}

fn admitted_bytecode(case: TailMatrixCase) -> Arc<ValidatedBytecodeArtifact> {
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
            functions: BTreeMap::from([
                (CALLER_FUNCTION.to_string(), artifact_function(case, true)),
                (TARGET_FUNCTION.to_string(), artifact_function(case, false)),
            ]),
            pools: BytecodePools {
                constants: Vec::new(),
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
                shapes: Vec::new(),
                effects: Vec::new(),
                resume: Vec::new(),
                callback_capture: Vec::new(),
                writable_paths: Vec::new(),
            },
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph { nodes: Vec::new() },
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn artifact_function(case: TailMatrixCase, caller: bool) -> RelocatableBytecodeFunction {
    let (function_key, executable, words, relocations, frame, max_depth, effect_owner) = if caller {
        (
            CALLER_FUNCTION,
            0,
            case.caller_words(),
            vec![BytecodeRelocation::LocalExecutableRef {
                function_key: TARGET_FUNCTION.to_string(),
                specialization: BytecodeSpecialization {
                    type_arguments: Vec::new(),
                    concrete_receiver: None,
                },
            }],
            artifact_frame(
                &case.caller_slots(),
                &case.caller_parameters(),
                &case.caller_writable(),
                &case.caller_results(),
            ),
            case.caller_max_depth(),
            CALLER_CALLABLE,
        )
    } else {
        (
            TARGET_FUNCTION,
            1,
            case.target_words(),
            Vec::new(),
            artifact_frame(&case.target_slots(), &[0], &[], &case.target_results()),
            case.target_max_depth(),
            TARGET_CALLABLE,
        )
    };
    RelocatableBytecodeFunction {
        function_key: function_key.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(executable),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words,
        relocations,
        call_loan_layouts: Vec::new(),
        frame_layout: frame,
        max_operand_depth: max_depth,
        effect_summary_ref: callable(effect_owner),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: caller
            .then(|| artifact_tail_statement(case))
            .into_iter()
            .collect(),
        source_map: caller
            .then(|| artifact_tail_source_map(case))
            .into_iter()
            .collect(),
    }
}

fn artifact_frame(
    slots: &[u32],
    parameters: &[u32],
    writable: &[u32],
    results: &[u32],
) -> FrameLayout {
    FrameLayout {
        slot_count: u32::try_from(slots.len()).unwrap(),
        slot_type_refs: slots.to_vec(),
        parameter_slots: parameters
            .iter()
            .copied()
            .map(|slot| ParameterSlotDecl {
                slot,
                mode: ParamModeIr::Value,
                plan: artifact_plan(slots[slot as usize]),
            })
            .collect(),
        writable_local_slots: writable.to_vec(),
        result_count: u32::try_from(results.len()).unwrap(),
        result_type_refs: results.to_vec(),
        result_plans: results.iter().copied().map(artifact_plan).collect(),
        stream_result_type_ref: None,
        slot_plans: slots.iter().copied().map(artifact_plan).collect(),
    }
}

fn artifact_plan(_ty: u32) -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    }
}

fn artifact_tail_statement(case: TailMatrixCase) -> StatementEntry {
    StatementEntry {
        pc: case.tail_pc(),
        sequence_ordinal: 0,
        attribution_id: StatementAttributionId::Expression {
            expression_index: 0,
            occurrence_ordinal: 0,
        },
        site: tail_site(),
    }
}

fn artifact_tail_source_map(case: TailMatrixCase) -> SourceMapEntry {
    SourceMapEntry {
        start_pc: case.tail_pc(),
        end_pc: case.tail_pc() + 3,
        site: tail_site(),
    }
}

pub(super) fn tail_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    }
}

impl TailMatrixCase {
    fn caller_parameter_types(self) -> Vec<u32> {
        let slots = self.caller_slots();
        self.caller_parameters()
            .into_iter()
            .map(|slot| slots[slot as usize])
            .collect()
    }

    fn caller_slots(self) -> Vec<u32> {
        match self {
            Self::LiveCleanup | Self::StackResidue => vec![0, 0],
            Self::MovedAndUninitialized => vec![0, 0],
            Self::ArgumentClassCorruption => vec![2],
            Self::ArgumentPlanCorruption | Self::ResultPlanCorruption => vec![0],
        }
    }

    fn caller_parameters(self) -> Vec<u32> {
        match self {
            Self::LiveCleanup | Self::StackResidue => vec![0, 1],
            Self::MovedAndUninitialized
            | Self::ArgumentClassCorruption
            | Self::ArgumentPlanCorruption
            | Self::ResultPlanCorruption => vec![0],
        }
    }

    fn caller_writable(self) -> Vec<u32> {
        match self {
            Self::MovedAndUninitialized => vec![1],
            _ => Vec::new(),
        }
    }

    fn caller_results(self) -> Vec<u32> {
        match self {
            Self::ResultPlanCorruption => vec![0],
            _ => Vec::new(),
        }
    }

    fn target_slots(self) -> Vec<u32> {
        vec![1]
    }

    fn target_results(self) -> Vec<u32> {
        match self {
            Self::ResultPlanCorruption => vec![1],
            _ => Vec::new(),
        }
    }

    fn caller_words(self) -> Vec<u32> {
        match self {
            Self::StackResidue => vec![0x07, 0, 0x07, 1, 0x21, 0, 1],
            _ => vec![0x07, 0, 0x21, 0, 1],
        }
    }

    fn target_words(self) -> Vec<u32> {
        match self {
            Self::ResultPlanCorruption => vec![0x07, 0, 0x25],
            _ => vec![0x25],
        }
    }

    const fn tail_pc(self) -> u32 {
        match self {
            Self::StackResidue => 4,
            _ => 2,
        }
    }

    const fn tail_instruction(self) -> u32 {
        match self {
            Self::StackResidue => 2,
            _ => 1,
        }
    }

    const fn caller_max_depth(self) -> u32 {
        match self {
            Self::StackResidue => 2,
            _ => 1,
        }
    }

    const fn target_max_depth(self) -> u32 {
        match self {
            Self::ResultPlanCorruption => 1,
            _ => 0,
        }
    }
}
