use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use skiff_artifact_identity::{
    contract_operation_id, package_artifact_ref, service_contract_ref, service_deployment_ref,
    ValidatedBytecodeArtifact,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, GatewayEntryIdentity, PackageArtifact,
    ServiceContract, ServiceDeployment, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_package, CompilerPlatformSources, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageSourceInput, PublicationManifest, PublicationSourceGraph,
    SourceTree, SourceTreeFile,
};
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_input::PublicationApiSpec;
use skiff_compiler_source::source_graph::CompilerSourceFile;
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver;
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{decode_response_end_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    runtime_assembly_request::{
        RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
        RuntimeAssemblyRequestIngressFrameHeader, RuntimeAssemblyRequestIngressProtocol,
        RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
        RuntimeAssemblyRequestStartFrameWireHeader, RuntimeAssemblyRequestTraceFrameHeader,
    },
};
use tokio::{sync::mpsc, time::timeout};

use crate::host::{router_session::ConnectionBootstrap, RuntimeConfig, RuntimeHost};

const PACKAGE_ID: &str = "example.com/host-bytecode-scalar";
const VERSION: &str = "1.0.0";
const OPERATION: &str = "run";

struct CompiledFixture {
    artifact_root: PathBuf,
    deployment: skiff_artifact_model::ServiceDeploymentRef,
}

static FIXTURE: OnceLock<CompiledFixture> = OnceLock::new();

fn fixture() -> &'static CompiledFixture {
    FIXTURE.get_or_init(|| {
        std::thread::Builder::new()
            .name("host-bytecode-http-fixture".to_string())
            .stack_size(32 * 1024 * 1024)
            .spawn(compile_fixture)
            .expect("bytecode fixture compiler thread")
            .join()
            .expect("bytecode fixture compiler thread should not panic")
    })
}

fn compile_fixture() -> CompiledFixture {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime/host must live below the repository root")
        .to_path_buf();
    let platform_sources =
        CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let package_id = PublicationId::parse(PACKAGE_ID).expect("test package id");
    let temp = std::env::temp_dir().join(format!(
        "skiff-host-bytecode-src-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).expect("bytecode source dir");
    let source_path = temp.join("main.skiff");
    let text = "function run() -> number { return 42 }\n";
    std::fs::write(&source_path, text).expect("bytecode source");
    let source_tree = SourceTree {
        root: temp.clone(),
        sources: vec![SourceTreeFile {
            module_path: "main".to_string(),
            file_path: PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text.to_string(),
        source_path.display().to_string(),
    )
    .expect("parse bytecode source");
    let package = PackageSourceInput::new(
        PublicationManifest::new(
            package_id,
            VERSION.to_string(),
            PublicationApiSpec::empty(),
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
    let input = PackageCompileInput::new(&platform_sources, &package, &aliases, PACKAGE_ID, true);
    let compiled = compile_package(input).expect("compile bytecode package");
    let handoff = compiled.bytecode_handoff().expect("bytecode handoff");
    let package = Arc::new(compiled.package().artifact.clone());
    let bytecode = Arc::new(
        ValidatedBytecodeArtifact::admit(handoff.artifact().clone()).expect("admit bytecode"),
    );

    let operation_id = contract_operation_id(PACKAGE_ID, VERSION, OPERATION).expect("operation id");
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: PACKAGE_ID.to_string(),
        contract_version: VERSION.to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: OPERATION.to_string(),
                contract: BoundaryOperationContract {
                    parameters: Vec::new(),
                    return_value: BoundaryReturn {
                        ty: ContractTypeRef::builtin("number"),
                        value_plan: BoundaryValuePlan::Linkable {
                            carrier: BoundaryValueCarrier::DetachedValueGraph,
                            encoding: BoundaryValueEncoding::CanonicalValue,
                            owner: BoundaryValueOwner::Provider,
                            lifetime: BoundaryValueLifetime::Call,
                        },
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
            service: PACKAGE_ID.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract)
        .expect("contract identities");

    let package_ref = package_artifact_ref(&package).expect("package ref");
    let contract_ref = service_contract_ref(&contract).expect("contract ref");
    let callable_id = package
        .callable_links
        .keys()
        .next()
        .expect("compiled scalar package callable")
        .clone();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref,
        deployment_revision: DeploymentRevision::new("revision-host-bytecode-http"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref.clone(),
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation_id,
            package_callable_id: callable_id,
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "host bytecode HTTP".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
        .expect("deployment identity");
    let deployment_ref = service_deployment_ref(&deployment);

    let artifact_root = std::env::temp_dir().join(format!(
        "skiff-host-bytecode-artifacts-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    let store = CanonicalArtifactStore::create(&artifact_root).expect("artifact store");
    store
        .write_package_bytecode(&package_ref, bytecode.artifact())
        .expect("write bytecode");
    store
        .write_package_artifact(&package)
        .expect("write package");
    store
        .write_service_contract(&contract)
        .expect("write contract");
    store
        .write_service_deployment(&deployment)
        .expect("write deployment");

    std::fs::remove_dir_all(temp).expect("clean source dir");

    CompiledFixture {
        artifact_root,
        deployment: deployment_ref,
    }
}

fn canonical_header(
    fixture: &CompiledFixture,
    request_id: &str,
) -> RuntimeAssemblyRequestStartFrameHeader {
    RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: fixture.deployment.clone(),
            build_id: None,
            gateway_entry_identity: GatewayEntryIdentity::parse(format!(
                "skiff-gateway-entry-v2:sha256:{}",
                "a".repeat(64)
            ))
            .expect("gateway identity"),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                method: "POST".to_string(),
                path: "/run".to_string(),
            },
        },
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-bytecode-http".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: RuntimeAssemblyHttpRequestFrameHeader {
            method: "POST".to_string(),
            url: "http://api.example.test/run".to_string(),
            path: "/run".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: false,
        test_case_capability: None,
        test_case_parent_request_id: None,
    }
}

fn connection_bootstrap(fixture: &CompiledFixture) -> ConnectionBootstrap {
    ConnectionBootstrap {
        resolver: FilesystemRuntimeAssemblyContentResolver::open(&fixture.artifact_root)
            .expect("bytecode filesystem resolver"),
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        activation: serde_json::from_value(serde_json::json!({ "profile": "test" }))
            .expect("test bootstrap activation"),
        max_response_bytes: 1024,
    }
}

#[derive(Clone, Default)]
struct TestDbCapabilityFactory;

impl skiff_runtime_capability_context::DbCapabilityFactory for TestDbCapabilityFactory {
    fn context_for_request(
        &self,
        _owner: String,
        _request_id: String,
    ) -> skiff_runtime_capability_context::DbCapabilityContext {
        skiff_runtime_capability_context::DbCapabilityContext::unavailable()
    }
}

#[derive(Clone, Default)]
struct TestDbProviderFactory;

impl skiff_runtime_capability_context::DbProviderFactory for TestDbProviderFactory {
    fn build(
        &self,
        _input: skiff_runtime_capability_context::DbProviderBuildInput,
    ) -> skiff_runtime_capability_context::DbCapabilityResult<
        skiff_runtime_capability_context::DbCapabilitySource,
    > {
        Ok(skiff_runtime_capability_context::DbCapabilitySource::new(
            Some(TestDbCapabilityFactory),
        ))
    }
}

fn test_host() -> RuntimeHost {
    RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::new(TestDbProviderFactory),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-bytecode-http".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-bytecode-http-home"),
        profile: "test".to_string(),
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("bytecode HTTP runtime host")
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_http_bytecode_request_executes_through_scalar_vm() {
    let fixture = fixture();
    let host = test_host();
    let route = host
        .bytecode_deployments
        .route(&fixture.deployment, &fixture.artifact_root)
        .await
        .expect("bytecode route should load");
    assert!(route.is_some(), "fixture must carry a bytecode deployment");

    let bootstrap = connection_bootstrap(fixture);
    let header = canonical_header(fixture, "bytecode-http-42");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_runtime_assembly_request(
        "bytecode-http-session",
        RuntimeAssemblyRequestStartFrameWireHeader::Http(header),
        Vec::new(),
        &bootstrap,
        sender,
    )
    .await;

    let message = timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("bytecode response timeout")
        .expect("bytecode response channel closed");
    let RouterWriterMessage::Binary(frame) = message else {
        panic!("bytecode response must be a binary transport frame")
    };
    let (_header, payload) = decode_response_end_frame(&frame).expect("bytecode response.end");
    assert_eq!(payload, b"42.0");
}
