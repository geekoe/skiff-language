use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractOperationId, ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, PackageArtifact, ServiceContract,
    ServiceDeployment, SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_package, CompilerPlatformSources, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageSourceInput, PublicationManifest, PublicationSourceGraph,
    SourceTree, SourceTreeFile,
};
use skiff_runtime_bytecode_verifier::{verify, VerificationLimits, VerifiedLinkedBytecodeImage};
use skiff_runtime_deployment_image::{DeploymentImage, PinnedDeploymentEntry};
use skiff_runtime_linked_bytecode::{ActiveRegionIndex, InstructionIndex};
use skiff_runtime_linker::{link_deployment, LinkLimits};
use skiff_runtime_loader::{DeploymentBytecodeContentResolver, DeploymentBytecodeLoader};
use skiff_runtime_model::vm_value::ValueSlot;

use super::{Vm, VmFiber, VmFiberState};
use crate::{VmError, VmLimits, VmProjectionHandoff};

#[test]
fn mint_seam_accepts_no_caller_supplied_authority_facts() {
    let mint: fn(&mut VmFiber) -> Result<VmProjectionHandoff, VmError> =
        VmFiber::mint_projection_handoff;

    let _ = mint;
}

#[test]
fn handoff_derives_exact_active_fiber_facts_and_monotonic_sequence() {
    let image = test_image();
    let mut fiber = image.start();
    let frame = fiber.frames.last().unwrap().clone();
    let frame_depth = fiber.frames.len();
    let active_region_depth = fiber.active_regions.len();

    let first = fiber.mint_projection_handoff().unwrap();
    let second = fiber.mint_projection_handoff().unwrap();

    assert!(Arc::ptr_eq(first.image(), &image.verified));
    assert!(Arc::ptr_eq(second.image(), &image.verified));
    assert_eq!(first.function(), frame.function());
    assert_eq!(first.instruction(), frame.instruction());
    assert_eq!(first.frame_depth(), frame_depth);
    assert_eq!(first.operand_height(), frame.operand_height());
    assert_eq!(first.active_region_depth(), active_region_depth);
    assert_eq!(first.projection_sequence(), 0);
    assert_eq!(second.projection_sequence(), 1);
    assert_eq!(fiber.state(), VmFiberState::Runnable);
}

#[test]
fn pc_shape_region_and_sequence_failures_return_no_handoff_and_terminalize() {
    let image = test_image();

    let mut invalid_pc = image.start();
    let function = invalid_pc.frames.last().unwrap().function();
    invalid_pc
        .frames
        .last_mut()
        .unwrap()
        .jump_to(InstructionIndex::new(u32::MAX));
    assert_eq!(
        mint_error(&mut invalid_pc),
        VmError::InstructionPointerOutOfBounds {
            function,
            instruction: InstructionIndex::new(u32::MAX),
        }
    );
    assert_eq!(invalid_pc.state(), VmFiberState::Terminal);

    let mut invalid_stack = image.start();
    let expected = invalid_stack.frames.last().unwrap().operand_height();
    invalid_stack
        .frames
        .last_mut()
        .unwrap()
        .set_operand_height(expected + 1);
    assert_eq!(
        mint_error(&mut invalid_stack),
        VmError::OperandStackShapeMismatch {
            function,
            expected,
            actual: expected + 1,
        }
    );
    assert_eq!(invalid_stack.projection_sequence, 0);
    assert_eq!(invalid_stack.state(), VmFiberState::Terminal);

    let mut invalid_region = image.start();
    invalid_region
        .active_regions
        .push(ActiveRegionIndex::new(0));
    assert!(matches!(
        mint_error(&mut invalid_region),
        VmError::RegionLeaveMismatch { .. }
    ));
    assert_eq!(invalid_region.projection_sequence, 0);
    assert_eq!(invalid_region.state(), VmFiberState::Terminal);

    let mut overflow = image.start();
    overflow.projection_sequence = u64::MAX;
    assert_eq!(mint_error(&mut overflow), VmError::ResumeTokenMismatch);
    assert_eq!(overflow.projection_sequence, u64::MAX);
    assert_eq!(overflow.state(), VmFiberState::Terminal);
}

#[test]
fn handoff_is_absent_from_production_control_and_request_consumers() {
    let control = include_str!("../control.rs");
    assert!(!control.contains("VmProjectionHandoff"));

    let fiber = include_str!("../fiber.rs");
    let dispatch = fiber
        .split_once("enum DispatchOutcome")
        .and_then(|(_, suffix)| suffix.split_once("\n}\n"))
        .map(|(body, _)| body)
        .expect("DispatchOutcome source block");
    assert!(!dispatch.contains("VmProjectionHandoff"));
    assert_eq!(fiber.matches("mint_projection_handoff(").count(), 1);

    let request_src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("request/src");
    assert_no_handoff_reference(&request_src);
}

fn mint_error(fiber: &mut VmFiber) -> VmError {
    match fiber.mint_projection_handoff() {
        Ok(_) => panic!("invalid VM state minted projection authority"),
        Err(error) => error,
    }
}

fn assert_no_handoff_reference(path: &std::path::Path) {
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            assert_no_handoff_reference(&path);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            assert!(
                !source.contains("VmProjectionHandoff"),
                "request production source unexpectedly consumes handoff: {}",
                path.display()
            );
        }
    }
}

struct ProjectionTestImage {
    verified: Arc<VerifiedLinkedBytecodeImage>,
    image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
    operation: ContractOperationId,
}

impl ProjectionTestImage {
    fn build() -> Self {
        const PACKAGE_ID: &str = "example.com/vm-projection-authority";
        let (package, bytecode) = compile_test_package(PACKAGE_ID);
        let operation =
            skiff_artifact_identity::contract_operation_id(PACKAGE_ID, "1.0.0", "run").unwrap();
        let contract = service_contract(PACKAGE_ID, operation.clone());
        let (deployment, deployment_ref) =
            service_deployment(&package, &contract, operation.clone());
        let resolver = TestResolver {
            deployment,
            contract,
            package,
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_ref)
            .unwrap();
        let candidate = link_deployment(&hydrated, &link_limits()).unwrap();
        let verified = Arc::new(verify(hydrated, candidate, &verification_limits()).unwrap());
        let image = Arc::new(DeploymentImage::try_new(Arc::clone(&verified)).unwrap());
        Self {
            verified,
            image,
            operation,
        }
    }

    fn start(&self) -> VmFiber {
        let entry = self.verified.operation_entry(&self.operation).unwrap();
        let pinned = PinnedDeploymentEntry::try_new(Arc::clone(&self.image), entry).unwrap();
        Vm::start(pinned, Box::<[ValueSlot]>::default(), vm_limits()).unwrap()
    }
}

fn test_image() -> &'static ProjectionTestImage {
    static IMAGE: OnceLock<ProjectionTestImage> = OnceLock::new();
    IMAGE.get_or_init(ProjectionTestImage::build)
}

fn compile_test_package(
    package_id: &str,
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .unwrap()
        .to_path_buf();
    let platform_sources = CompilerPlatformSources::new(&repository_root).unwrap();
    let publication_id = skiff_compiler_core::id::PublicationId::parse(package_id).unwrap();
    let temp = std::env::temp_dir().join(format!(
        "skiff-vm-projection-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let source = "function run() -> number { return 1 }\n";
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, source).unwrap();
    let source_tree = SourceTree {
        root: temp.clone(),
        sources: vec![SourceTreeFile {
            module_path: "main".to_string(),
            file_path: PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: source.len() as u64,
        }],
    };
    let parsed = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        source.to_string(),
        source_path.display().to_string(),
    )
    .unwrap();
    let package = PackageSourceInput::new(
        PublicationManifest::new(
            publication_id.clone(),
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
            ManifestProvenance {
                owner: ManifestOwner::UserOrBuiltinPackage,
                path: PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        PublicationSourceGraph::from_compiler_sources(vec![parsed]),
        Vec::new(),
    );
    let compiled = compile_package(PackageCompileInput::new(
        &platform_sources,
        &package,
        &BTreeMap::new(),
        publication_id.as_str(),
        true,
    ))
    .unwrap();
    let bytecode = Arc::new(
        ValidatedBytecodeArtifact::admit(compiled.bytecode_handoff().unwrap().artifact().clone())
            .unwrap(),
    );
    let package = Arc::new(compiled.package().artifact.clone());
    std::fs::remove_dir_all(temp).unwrap();
    (package, bytecode)
}

fn service_contract(package_id: &str, operation: ContractOperationId) -> Arc<ServiceContract> {
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: package_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation,
                stable_key: "run".to_string(),
                contract: BoundaryOperationContract {
                    parameters: Vec::new(),
                    return_value: BoundaryReturn {
                        ty: ContractTypeRef::builtin("number"),
                        value_plan: detached_plan(),
                    },
                    stream: BoundaryStreamContract::Unary,
                    callbacks: BoundaryCallbackContract::None,
                    effect_guarantee: BoundaryEffectGuarantee {
                        detached_parameters: true,
                        detached_return: true,
                        detached_error: true,
                        no_caller_reachable_mutation: true,
                        no_caller_value_escape: true,
                        no_same_heap_identity: true,
                    },
                },
            },
        )]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: skiff_artifact_model::ContractDiagnosticText {
            service: package_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    Arc::new(contract)
}

fn detached_plan() -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn service_deployment(
    package: &PackageArtifact,
    contract: &ServiceContract,
    operation: ContractOperationId,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let callable = package.callable_links.keys().next().unwrap().clone();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:vm-projection-authority"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(package).unwrap(),
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation,
            package_callable_id: callable,
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "VM projection authority test".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference)
}

struct TestResolver {
    deployment: Arc<ServiceDeployment>,
    contract: Arc<ServiceContract>,
    package: Arc<PackageArtifact>,
    bytecode: Arc<ValidatedBytecodeArtifact>,
}

impl DeploymentBytecodeContentResolver for TestResolver {
    fn resolve_deployment(
        &self,
        reference: &skiff_artifact_model::ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        anyhow::ensure!(
            &skiff_artifact_identity::service_deployment_ref(&self.deployment) == reference,
            "deployment reference mismatch"
        );
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &skiff_artifact_model::ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        anyhow::ensure!(
            &skiff_artifact_identity::service_contract_ref(&self.contract).unwrap() == reference,
            "contract reference mismatch"
        );
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        reference: &skiff_artifact_model::PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        anyhow::ensure!(
            &skiff_artifact_identity::package_artifact_ref(&self.package).unwrap() == reference,
            "package reference mismatch"
        );
        Ok(Arc::clone(&self.package))
    }

    fn resolve_package_bytecode(
        &self,
        package: &skiff_artifact_model::PackageArtifactRef,
        reference: &skiff_artifact_model::BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        anyhow::ensure!(
            &skiff_artifact_identity::package_artifact_ref(&self.package).unwrap() == package,
            "bytecode package mismatch"
        );
        anyhow::ensure!(
            self.bytecode.reference() == reference,
            "bytecode reference mismatch"
        );
        Ok(Arc::clone(&self.bytecode))
    }
}

fn vm_limits() -> VmLimits {
    VmLimits::new(
        NonZeroUsize::new(128).unwrap(),
        NonZeroUsize::new(4096).unwrap(),
        NonZeroU32::new(1024).unwrap(),
        NonZeroU32::new(1024).unwrap(),
    )
}

fn link_limits() -> LinkLimits {
    LinkLimits {
        max_packages: u64::MAX,
        max_root_specializations: u64::MAX,
        max_specializations: u64::MAX,
        max_code_words_per_function: u64::MAX,
        max_total_code_words: u64::MAX,
        max_relocations_per_function: u64::MAX,
        max_total_relocations: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_total_image_table_entries: u64::MAX,
        max_total_function_table_entries: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_expanded_type_nodes: u64::MAX,
        max_expanded_type_bytes: u64::MAX,
        max_constant_graph_nodes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}

fn verification_limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: u64::MAX,
        max_total_instructions: u64::MAX,
        max_instructions_per_function: u64::MAX,
        max_frame_slots_per_function: u64::MAX,
        max_operand_depth: u64::MAX,
        max_control_flow_edges_per_function: u64::MAX,
        max_exception_regions_per_function: u64::MAX,
        max_switch_targets_per_function: u64::MAX,
        max_statement_events_per_pc: u64::MAX,
        max_statement_events_per_function: u64::MAX,
        max_total_statement_events: u64::MAX,
        max_source_map_entries_per_function: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_arity: u64::MAX,
        max_callback_captures_per_callback: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_value_lifecycle_nodes: u64::MAX,
        max_value_lifecycle_canonical_bytes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}
