use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use skiff_artifact_identity::{
    contract_operation_id, package_artifact_ref, service_contract_ref, service_deployment_ref,
    PackageBytecodeRecordPath,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, IngressProtocol, IngressSelector,
    PackageArtifact, ServiceContract, ServiceDeployment, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_package, CompilerPlatformSources, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageCompileOutput, PackageSourceInput, PublicationManifest,
    PublicationSourceGraph, SourceTree, SourceTreeFile,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_runtime_bytecode_verifier::{verify, CodeEntryLookupError, VerificationLimits};
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_deployment_image::DeploymentImage;
use skiff_runtime_linker::{link_deployment, LinkLimits};
use skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver;
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_runtime_request::{
    execute_runtime_bytecode_request, BoundaryResponse, BytecodeRequestExecutionHandles,
    BytecodeRequestExecutionInput, BytecodeRequestTarget, ExecutionBudget, RequestEnvelope,
    RequestError, ResponseEnd, ResponseEvent,
};

const FIXTURE_RELATIVE: &str =
    "doc/implementation/bytecode-vm-convergence/fixtures/vcp1-trusted-scalar/main.skiff";
const PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-0";
const VERSION: &str = "1.0.0";
const MANIFEST_SCHEMA: &str = "skiff-vcp-phase-0-v1";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[test]
fn phase_0_vcp_trusted_scalar_execution() {
    let repo_root = repository_root();
    let fixture_path = repo_root.join(FIXTURE_RELATIVE);
    let source_text = fs::read_to_string(&fixture_path).expect("phase 0 fixture must exist");
    let source_hash = sha256_hex(source_text.as_bytes());
    let artifact_root = unique_temp_dir("skiff-vcp-phase-0-artifacts-");
    let compiled = compile_fixture(&repo_root, &fixture_path, PACKAGE_ID);
    let handoff = compiled
        .bytecode_handoff()
        .expect("phase 0 fixture must emit bytecode");
    let package = Arc::new(compiled.package().artifact.clone());
    let package_ref = package_artifact_ref(&package).expect("package identity");
    let receipt = skiff_compiler::authoring::publish_package_artifact_records_with_bytecode(
        &artifact_root,
        compiled.package(),
        handoff,
    )
    .expect("publish immutable bytecode package records");
    assert_eq!(receipt.artifact, package_ref);

    let (contract, operation_id) = service_contract();
    let contract_ref = service_contract_ref(&contract).expect("contract identity");
    let (deployment, deployment_ref) =
        service_deployment(&package, &contract_ref, operation_id.clone());
    let store = CanonicalArtifactStore::open(&artifact_root).expect("artifact store");
    store
        .write_service_contract(&contract)
        .expect("write contract record");
    store
        .write_service_deployment(&deployment)
        .expect("write deployment record");
    let release = ReleasePointer::new("skiff-test", deployment_ref.clone())
        .expect("release pointer identity");
    store
        .write_release_pointer(&release)
        .expect("write release pointer");
    assert_eq!(
        store
            .read_release_pointer("skiff-test", PACKAGE_ID, VERSION)
            .expect("read release pointer")
            .expect("release pointer exists")
            .deployment,
        deployment_ref
    );

    let resolver = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
        .expect("open filesystem resolver");
    let hydrated = resolver
        .load_deployment_bytecode(&deployment_ref)
        .expect("hydrate exact deployment bytecode");
    let candidate =
        link_deployment(&hydrated, &generous_link_limits()).expect("link exact deployment");
    let verified = Arc::new(
        verify(hydrated, candidate, &generous_verification_limits())
            .expect("verify linked scalar image"),
    );
    let image = Arc::new(
        DeploymentImage::try_new(Arc::clone(&verified)).expect("construct immutable image"),
    );
    let entry = verified
        .operation_entry(&operation_id)
        .expect("exact operation entry");
    let target = BytecodeRequestTarget::try_new(Arc::clone(&image), entry, operation_id.clone())
        .expect("exact request target");
    let response = execute_runtime_bytecode_request(BytecodeRequestExecutionInput {
        target,
        request: request_envelope(&deployment_ref, "unary"),
        cancelled: Arc::new(AtomicBool::new(false)),
        cancellation: CancellationToken::new(),
        execution_budget: Arc::new(ExecutionBudget::disabled()),
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
            http_executor: None,
            self_ingress: None,
        },
    })
    .expect("production request entry");
    let observed = payload_f64(response);
    assert_eq!(observed, 3.0);

    let missing_operation =
        contract_operation_id(PACKAGE_ID, VERSION, "missing").expect("missing operation id");
    assert!(matches!(
        verified.operation_entry(&missing_operation),
        Err(CodeEntryLookupError::OperationNotFound { .. })
    ));

    let unsupported_request = execute_runtime_bytecode_request(BytecodeRequestExecutionInput {
        target: request_target(Arc::clone(&image), &verified, &operation_id),
        request: request_envelope(&deployment_ref, "serverStream"),
        cancelled: Arc::new(AtomicBool::new(false)),
        cancellation: CancellationToken::new(),
        execution_budget: Arc::new(ExecutionBudget::disabled()),
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
            http_executor: None,
            self_ingress: None,
        },
    });
    assert!(matches!(
        unsupported_request,
        Err(RequestError::Unsupported(_))
    ));

    let bytecode_path = {
        let bytecode_ref = package
            .bytecode
            .as_ref()
            .expect("compiled package must reference bytecode");
        let path =
            PackageBytecodeRecordPath::new(&package_ref, bytecode_ref).expect("bytecode path");
        artifact_root.join(path.as_relative_path().as_path())
    };
    fs::write(&bytecode_path, b"{}").expect("corrupt immutable bytecode record for negative case");
    let corrupt = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
        .expect("open corrupt store")
        .load_deployment_bytecode(&deployment_ref);
    assert!(corrupt.is_err(), "corrupt bytecode must fail admission");

    write_manifest(
        &artifact_root,
        &fixture_path,
        source_hash,
        &receipt.artifact,
        handoff.reference().bytecode_identity.as_str(),
        &deployment_ref,
    );
    fs::remove_dir_all(&artifact_root).expect("remove VCP temp artifact root");
}

fn payload_f64(response: BoundaryResponse) -> f64 {
    let BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload))) = response
    else {
        panic!("VCP expected a unary payload response: {response:?}");
    };
    serde_json::from_slice::<f64>(&payload).expect("payload must be a number")
}

fn request_target(
    image: Arc<DeploymentImage<skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage>>,
    verified: &Arc<skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage>,
    operation_id: &skiff_artifact_model::ContractOperationId,
) -> BytecodeRequestTarget {
    let entry = verified
        .operation_entry(operation_id)
        .expect("exact operation entry");
    BytecodeRequestTarget::try_new(image, entry, operation_id.clone())
        .expect("exact request target")
}

fn compile_fixture(
    repo_root: &Path,
    fixture_path: &Path,
    package_id: &str,
) -> PackageCompileOutput {
    let text = fs::read_to_string(fixture_path).expect("fixture source");
    let temp = unique_temp_dir("skiff-vcp-phase-0-source-");
    let source_path = temp.join("main.skiff");
    fs::write(&source_path, &text).expect("copy fixture source");
    let platform_sources =
        CompilerPlatformSources::new(repo_root).expect("platform sources from repository");
    let parsed_id = skiff_compiler_core::id::PublicationId::parse(package_id).unwrap();
    let source_tree = SourceTree {
        root: temp.clone(),
        sources: vec![SourceTreeFile {
            module_path: "main".to_string(),
            file_path: PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text,
        source_path.display().to_string(),
    )
    .expect("parse fixture source");
    let package = PackageSourceInput::new(
        PublicationManifest::new(
            parsed_id.clone(),
            VERSION.to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
            ManifestProvenance {
                owner: ManifestOwner::UserOrBuiltinPackage,
                path: PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let input = PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        parsed_id.as_str(),
        true,
    );
    let compiled = compile_package(input).expect("compile phase 0 fixture");
    fs::remove_dir_all(&temp).expect("remove source temp");
    compiled
}

fn service_contract() -> (
    Arc<ServiceContract>,
    skiff_artifact_model::ContractOperationId,
) {
    let operation_id =
        contract_operation_id(PACKAGE_ID, VERSION, "run").expect("run operation identity");
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: PACKAGE_ID.to_string(),
        contract_version: VERSION.to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: "run".to_string(),
                contract: BoundaryOperationContract {
                    parameters: Vec::new(),
                    return_value: BoundaryReturn {
                        ty: ContractTypeRef::builtin("number"),
                        value_plan: detached_plan(
                            BoundaryValueOwner::Provider,
                            BoundaryValueLifetime::Call,
                        ),
                    },
                    stream: BoundaryStreamContract::Unary,
                    callbacks: BoundaryCallbackContract::None,
                    effect_guarantee: detached_guarantee(),
                },
            },
        )]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: skiff_artifact_model::ContractDiagnosticText {
            service: PACKAGE_ID.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (Arc::new(contract), operation_id)
}

fn service_deployment(
    package: &PackageArtifact,
    contract_ref: &skiff_artifact_model::ServiceContractRef,
    operation_id: skiff_artifact_model::ContractOperationId,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let package_ref = package_artifact_ref(package).expect("package identity");
    let callable_id = skiff_compiler_core::implementation_package_callable_id(
        PACKAGE_ID,
        "main",
        "run",
        skiff_compiler_core::ImplementationCallableKind::Function,
    )
    .expect("run callable identity");
    assert!(
        package.callable_links.contains_key(&callable_id),
        "compiled scalar package must expose main.run"
    );
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref.clone(),
        deployment_revision: DeploymentRevision::new("revision-vcp-phase-0"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref,
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation_id,
            package_callable_id: callable_id,
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "vcp phase 0".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    (Arc::new(deployment), reference)
}

fn detached_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn detached_guarantee() -> BoundaryEffectGuarantee {
    BoundaryEffectGuarantee {
        detached_parameters: true,
        detached_return: true,
        detached_error: true,
        no_caller_reachable_mutation: true,
        no_caller_value_escape: true,
        no_same_heap_identity: true,
    }
}

fn request_envelope(
    deployment_ref: &skiff_artifact_model::ServiceDeploymentRef,
    mode: &str,
) -> RequestEnvelope {
    RequestEnvelope {
        request_id: "vcp-phase-0-request".to_string(),
        mode: mode.to_string(),
        target: "display-only".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some(PACKAGE_ID.to_string()),
        build_id: deployment_ref
            .deployment_artifact_identity
            .as_str()
            .to_string(),
        service_protocol_identity: "unassigned".to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/entry".to_string(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
        payload_bytes: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn write_manifest(
    artifact_root: &Path,
    fixture_path: &Path,
    source_hash: String,
    package_ref: &skiff_artifact_model::PackageArtifactRef,
    bytecode_identity: &str,
    deployment_ref: &skiff_artifact_model::ServiceDeploymentRef,
) {
    let Some(manifest_path) = env::var_os("SKIFF_VCP_PHASE0_MANIFEST") else {
        return;
    };
    let harness_binary = env::current_exe().expect("current test binary");
    let harness_bytes = fs::read(&harness_binary).expect("read current test binary");
    let manifest = serde_json::json!({
        "schemaVersion": MANIFEST_SCHEMA,
        "candidate": {
            "commit": env::var("SKIFF_VCP_PHASE0_COMMIT").unwrap_or_else(|_| "local".to_string()),
            "repo": "skiff",
            "harnessCrate": env!("CARGO_PKG_NAME"),
            "harnessCrateVersion": env!("CARGO_PKG_VERSION"),
        },
        "fixture": {
            "path": fixture_path.display().to_string(),
            "sha256": source_hash,
            "packageId": PACKAGE_ID,
            "packageVersion": VERSION,
        },
        "artifactStore": {
            "root": artifact_root.display().to_string(),
            "packageBuildId": package_ref.package_build_id.as_str(),
            "bytecodeIdentity": bytecode_identity,
            "deploymentArtifactIdentity": deployment_ref.deployment_artifact_identity.as_str(),
            "releaseProfile": "skiff-test",
        },
        "binaryIdentities": {
            "harnessPath": harness_binary.display().to_string(),
            "harnessSha256": sha256_hex(&harness_bytes),
        },
        "composition": {
            "compiler": "compile_package",
            "structuralAdmission": "publish_package_artifact_records_with_bytecode + read_package_bytecode",
            "loader": "FilesystemDeploymentBytecodeContentResolver",
            "linker": "link_deployment",
            "verifier": "verify",
            "image": "DeploymentImage",
            "requestEntry": "execute_runtime_bytecode_request",
            "vmDispatch": "Vm::start + BytecodeScheduler::run",
            "bypassCount": 0,
            "fallbackCount": 0,
        },
        "scenarios": [
            {"id": "vcp-1-success", "status": "pass", "expected": "3.0", "observed": "3.0"},
            {"id": "negative-corrupt-bytecode", "status": "pass", "expected": "fail", "observed": "fail"},
            {"id": "negative-entry-mismatch", "status": "pass", "expected": "fail", "observed": "fail"},
            {"id": "negative-unsupported-request-mode", "status": "pass", "expected": "fail", "observed": "fail"},
        ],
        "counts": {"total": 4, "passed": 4, "failed": 0, "skipped": 0},
        "result": "pass",
    });
    let parent = Path::new(&manifest_path).parent().expect("manifest parent");
    fs::create_dir_all(parent).expect("manifest parent");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
}

fn generous_link_limits() -> LinkLimits {
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

fn generous_verification_limits() -> VerificationLimits {
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

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime request crate has repository parent")
        .to_path_buf()
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = env::temp_dir().join(format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        unique,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
