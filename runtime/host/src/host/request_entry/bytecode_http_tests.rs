use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::loader::bytecode_admission::BytecodeRouteSelector;
use skiff_artifact_identity::{
    contract_operation_id, gateway_entry_identity, package_artifact_ref, service_contract_ref,
    service_deployment_ref, ValidatedBytecodeArtifact,
};
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, BoundaryCallbackContract, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentGatewayEntry, DeploymentOperationBinding, DeploymentRevision, GatewayAdapterArg,
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayHttpProtocolSurface, GatewayProtocolSurface,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketRpcProfile, GatewayWebSocketShapeVersion, PackageBinding, PackageCallableId,
    PackageLocalAbiSymbol, PackageRequirementKey, ServiceContract, ServiceDeployment,
    WebSocketEntryId,
    SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
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
use skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver;
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{decode_binary_frame, decode_response_end_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    protocol::{
        BytecodeHttpRequestFrameHeader, BytecodeRequestCallerFrameHeader,
        BytecodeRequestIngressFrameHeader, BytecodeRequestIngressProtocol,
        BytecodeRequestRoutingFrameHeader, BytecodeRequestStartFrameHeader,
        BytecodeRequestStartFrameWireHeader, BytecodeRequestTraceFrameHeader,
        BytecodeTaskInvocationFrameHeader, BytecodeTaskRequestCallerFrameHeader,
        BytecodeTaskRequestRoutingFrameHeader, BytecodeTaskRequestStartFrameHeader,
        BytecodeWebSocketConnectIngressFrameHeader, BytecodeWebSocketConnectIngressProtocol,
        BytecodeWebSocketConnectRequestFrameHeader,
        BytecodeWebSocketConnectRequestStartFrameHeader,
        BytecodeWebSocketConnectRoutingFrameHeader,
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
    legacy_deployment: skiff_artifact_model::ServiceDeploymentRef,
    http_gateway_identity: GatewayEntryIdentity,
    websocket_gateway_identity: GatewayEntryIdentity,
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

fn implementation_callable_id(
    package: &skiff_artifact_model::PackageArtifact,
    selector: &str,
) -> PackageCallableId {
    let symbol = package
        .package_local_abi
        .implementation_symbols
        .get(selector)
        .unwrap_or_else(|| panic!("package fixture has no implementation callable {selector}"));
    match symbol {
        PackageLocalAbiSymbol::Callable { callable_id, .. } => callable_id.clone(),
        _ => panic!("package fixture callable {selector} is not a function"),
    }
}

fn http_gateway_entry(
    handler: PackageCallableId,
) -> (
    GatewayEntryKey,
    DeploymentGatewayEntry,
    GatewayEntryIdentity,
) {
    let key = GatewayEntryKey::parse("run").expect("HTTP gateway entry key");
    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::RawHttp,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpRequest],
            request_body_schema: None,
            response_schema: None,
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let identity = gateway_entry_identity(&protocol_surface).expect("HTTP gateway entry identity");
    let entry = DeploymentGatewayEntry {
        gateway_entry_identity: identity.clone(),
        protocol_surface,
        handler: Some(handler),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::RawHttp,
            args: vec![GatewayAdapterArg {
                param: "request".to_string(),
                source: GatewayAdapterSource::HttpRequest,
            }],
        },
        close_handler: None,
        close_adapter_plan: None,
    };
    (key, entry, identity)
}

fn websocket_gateway_entry(
    handler: PackageCallableId,
) -> (
    GatewayEntryKey,
    DeploymentGatewayEntry,
    GatewayEntryIdentity,
) {
    let key = GatewayEntryKey::parse("websocket").expect("WebSocket gateway entry key");
    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketConnect(
            GatewayWebSocketConnectProtocolSurface {
                connect_request_shape: GatewayWebSocketShapeVersion::V1,
                connect_result_shape: GatewayWebSocketShapeVersion::V1,
                connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                external_sources: vec![
                    GatewayAdapterSource::WebSocketConnectRequest,
                    GatewayAdapterSource::WebSocketConnectionId,
                ],
                downlink_frames: vec![
                    GatewayWebSocketDownlinkFrame::Binary,
                    GatewayWebSocketDownlinkFrame::Text,
                ],
                rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
                connection_close_shape: GatewayWebSocketShapeVersion::V1,
                close_external_sources: Vec::new(),
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let identity =
        gateway_entry_identity(&protocol_surface).expect("WebSocket gateway entry identity");
    let entry = DeploymentGatewayEntry {
        gateway_entry_identity: identity.clone(),
        protocol_surface,
        handler: Some(handler),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketConnect,
            args: Vec::new(),
        },
        close_handler: None,
        close_adapter_plan: None,
    };
    (key, entry, identity)
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
    let artifact_root = std::env::temp_dir().join(format!(
        "skiff-host-bytecode-artifacts-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    skiff_compiler::authoring::seed_official_std_package(&platform_sources, &artifact_root)
        .expect("seed compiler-owned std");
    let source_path = temp.join("main.skiff");
    let text = r#"import std

function run() -> number { return 42 }

function runHttp(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.HttpResponse {
    status: 200,
    headers: Array.empty<std.http.HttpHeader>(),
    body: bytes.fromUtf8("42.0"),
  }
}
"#;
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
    let (published_std, _) =
        skiff_compiler::authoring::author_official_std_package_with_bytecode(&platform_sources)
            .expect("author compiler-owned std with bytecode");
    let std_package = published_std.artifact;
    let std_package_ref = package_artifact_ref(&std_package).expect("compiler-owned std ref");
    let available_packages = [std_package];
    let input = PackageCompileInput::new(&platform_sources, &package, &aliases, PACKAGE_ID, true)
        .with_available_canonical_packages(&available_packages)
        .with_canonical_artifact_root(&artifact_root);
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
    let callable_id = implementation_callable_id(&package, "main.run");
    let http_callable_id = implementation_callable_id(&package, "main.runHttp");
    let legacy_callable_id = callable_id.clone();
    let legacy_operation_id = operation_id.clone();
    let (http_gateway_key, http_gateway_entry, http_gateway_identity) =
        http_gateway_entry(http_callable_id);
    let (websocket_gateway_key, websocket_gateway_entry, websocket_gateway_identity) =
        websocket_gateway_entry(callable_id.clone());
    let gateway_entries = BTreeMap::from([
        (http_gateway_key.clone(), http_gateway_entry),
        (websocket_gateway_key.clone(), websocket_gateway_entry),
    ]);
    let ingress = vec![
        skiff_artifact_model::DeploymentIngressBinding {
            selector: skiff_artifact_model::IngressSelector {
                protocol: skiff_artifact_model::IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: "/run".to_string(),
            },
            gateway_entry_key: http_gateway_key.clone(),
        },
        skiff_artifact_model::DeploymentIngressBinding {
            selector: skiff_artifact_model::IngressSelector {
                protocol: skiff_artifact_model::IngressProtocol::WebSocket,
                method: None,
                path: "/run".to_string(),
            },
            gateway_entry_key: websocket_gateway_key.clone(),
        },
    ];
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
        package_bindings: vec![PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: package.package_build_id.clone(),
                package_requirement_alias: "std".to_string(),
            },
            package: std_package_ref,
        }],
        service_selectors: Vec::new(),
        gateway_entries: gateway_entries.clone(),
        ingress: ingress.clone(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "host bytecode HTTP".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
        .expect("deployment identity");
    let deployment_ref = service_deployment_ref(&deployment);

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

    let mut legacy_package = package.as_ref().clone();
    legacy_package.bytecode = None;
    legacy_package.bytecode_statement_manifest_identity =
        derive_bytecode_statement_manifest_identity(&legacy_package.package_id, &[])
            .expect("empty bytecode statement manifest is canonical");
    legacy_package.synthetic_callback_owners.clear();
    legacy_package.bytecode_schema_records.clear();
    skiff_artifact_identity::assign_package_artifact_identities(&mut legacy_package)
        .expect("legacy package identities");
    let legacy_package_ref = package_artifact_ref(&legacy_package).expect("legacy package ref");
    store
        .write_package_artifact(&legacy_package)
        .expect("write legacy package");
    let mut legacy_deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: deployment.contract.clone(),
        deployment_revision: DeploymentRevision::new("revision-host-legacy-http"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: legacy_package_ref,
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: legacy_operation_id,
            package_callable_id: legacy_callable_id,
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "host legacy HTTP".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut legacy_deployment)
        .expect("legacy deployment identity");
    let legacy_deployment_ref = service_deployment_ref(&legacy_deployment);
    store
        .write_service_deployment(&legacy_deployment)
        .expect("write legacy deployment");

    std::fs::remove_dir_all(temp).expect("clean source dir");

    CompiledFixture {
        artifact_root,
        deployment: deployment_ref,
        legacy_deployment: legacy_deployment_ref,
        http_gateway_identity,
        websocket_gateway_identity,
    }
}

fn canonical_header(
    fixture: &CompiledFixture,
    request_id: &str,
) -> BytecodeRequestStartFrameHeader {
    canonical_header_for_deployment(
        &fixture.deployment,
        request_id,
        &fixture.http_gateway_identity,
    )
}

fn canonical_header_for_deployment(
    deployment: &skiff_artifact_model::ServiceDeploymentRef,
    request_id: &str,
    gateway_entry_identity: &GatewayEntryIdentity,
) -> BytecodeRequestStartFrameHeader {
    BytecodeRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: BytecodeRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: BytecodeRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: deployment.clone(),
            build_id: None,
            gateway_entry_identity: gateway_entry_identity.clone(),
            ingress: BytecodeRequestIngressFrameHeader {
                protocol: BytecodeRequestIngressProtocol::Http,
                method: "POST".to_string(),
                path: "/run".to_string(),
            },
        },
        client_session: None,
        deadline: None,
        trace: BytecodeRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-bytecode-http".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: BytecodeHttpRequestFrameHeader {
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

fn task_header(fixture: &CompiledFixture, request_id: &str) -> BytecodeTaskRequestStartFrameHeader {
    BytecodeTaskRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: BytecodeTaskRequestCallerFrameHeader {
            kind: "service".to_string(),
        },
        routing: BytecodeTaskRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: fixture.deployment.clone(),
            build_id: Some(
                fixture
                    .deployment
                    .deployment_artifact_identity
                    .as_str()
                    .to_string(),
            ),
        },
        invocation: BytecodeTaskInvocationFrameHeader {
            kind: "task".to_string(),
            target_kind: "function".to_string(),
            target: "function:run".to_string(),
        },
        deadline: None,
        trace: BytecodeRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-bytecode-task".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        test_effects_enabled: false,
        test_case_capability: None,
        task_attempt: None,
    }
}

fn websocket_connect_header(
    fixture: &CompiledFixture,
    request_id: &str,
) -> BytecodeWebSocketConnectRequestStartFrameHeader {
    let gateway_entry_identity = fixture.websocket_gateway_identity.clone();
    BytecodeWebSocketConnectRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: BytecodeRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: BytecodeWebSocketConnectRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: fixture.deployment.clone(),
            build_id: Some(
                fixture
                    .deployment
                    .deployment_artifact_identity
                    .as_str()
                    .to_string(),
            ),
            gateway_entry_identity: gateway_entry_identity.clone(),
            ingress: BytecodeWebSocketConnectIngressFrameHeader {
                protocol: BytecodeWebSocketConnectIngressProtocol::WebSocket,
                method: (),
                path: "/run".to_string(),
            },
        },
        client_session: None,
        deadline: None,
        trace: BytecodeRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-bytecode-websocket-connect".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        websocket_connect: BytecodeWebSocketConnectRequestFrameHeader {
            connection_id: "bytecode-websocket-connection".to_string(),
            url: "ws://api.example.test/run".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            version: None,
            websocket_entry_id: WebSocketEntryId::parse(format!(
                "skiff-websocket-entry-v1:sha256:{}",
                "0".repeat(64)
            ))
            .expect("websocket entry id"),
            gateway_entry_identity,
        },
        test_effects_enabled: false,
    }
}

async fn assert_bytecode_response_error(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    expected_request_id: &str,
    message_substring: &str,
) {
    loop {
        let message = timeout(Duration::from_secs(10), receiver.recv())
            .await
            .expect("bytecode failure response timeout")
            .expect("bytecode failure response channel closed");
        let RouterWriterMessage::Binary(frame) = message else {
            panic!("bytecode failure must be a binary transport frame")
        };
        let raw = decode_binary_frame(&frame).expect("bytecode response.error");
        let header = raw
            .header
            .as_object()
            .expect("bytecode response.error header must be an object");
        if header.get("type").and_then(serde_json::Value::as_str) == Some("runtime.capabilities") {
            continue;
        }
        assert_eq!(
            header.get("type").and_then(serde_json::Value::as_str),
            Some("response.error")
        );
        assert_eq!(
            header.get("requestId").and_then(serde_json::Value::as_str),
            Some(expected_request_id)
        );
        let serialized =
            serde_json::to_string(&raw.header).expect("bytecode error header serializes");
        assert!(
            serialized.contains(message_substring),
            "expected bytecode error containing {message_substring:?}, got {serialized}"
        );
        return;
    }
}

fn connection_bootstrap(fixture: &CompiledFixture) -> ConnectionBootstrap {
    ConnectionBootstrap {
        resolver: FilesystemDeploymentBytecodeContentResolver::open(&fixture.artifact_root)
            .expect("bytecode filesystem resolver"),
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
    test_host_with_bytecode_only(false)
}

fn test_host_with_bytecode_only(bytecode_only: bool) -> RuntimeHost {
    RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::new(TestDbProviderFactory),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-bytecode-http".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-bytecode-http-home"),
        profile: "test".to_string(),
        bytecode_only,
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("bytecode HTTP runtime host")
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_http_bytecode_request_executes_through_scalar_vm() {
    let fixture = fixture();
    let host = test_host_with_bytecode_only(true);
    let route = host
        .bytecode_deployments
        .route(
            &fixture.deployment,
            &fixture.artifact_root,
            BytecodeRouteSelector::Operation,
        )
        .await
        .expect("bytecode route should load");
    assert!(route.is_some(), "fixture must carry a bytecode deployment");

    let bootstrap = connection_bootstrap(fixture);
    let header = canonical_header(fixture, "bytecode-http-42");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_bytecode_request(
        "bytecode-http-session",
        BytecodeRequestStartFrameWireHeader::Http(header),
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

#[tokio::test(flavor = "current_thread")]
async fn canonical_http_bytecode_only_rejects_non_bytecode_deployment_before_legacy() {
    let fixture = fixture();
    let host = test_host_with_bytecode_only(true);
    let legacy_route = host
        .bytecode_deployments
        .route(
            &fixture.legacy_deployment,
            &fixture.artifact_root,
            BytecodeRouteSelector::Operation,
        )
        .await
        .expect("legacy deployment bytecode lookup should succeed");
    assert!(
        legacy_route.is_none(),
        "fixture must carry a deployment without a bytecode record"
    );
    let bootstrap = connection_bootstrap(fixture);
    let header = canonical_header_for_deployment(
        &fixture.legacy_deployment,
        "bytecode-only-legacy-http",
        &fixture.http_gateway_identity,
    );
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_bytecode_request(
        "bytecode-only-legacy-session",
        BytecodeRequestStartFrameWireHeader::Http(header.clone()),
        Vec::new(),
        &bootstrap,
        sender,
    )
    .await;
    assert_bytecode_response_error(
        &mut receiver,
        &header.request_id,
        "bytecode is required for this deployment",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_http_server_stream_with_scalar_operation_fails_closed() {
    let fixture = fixture();
    let host = test_host();
    let bootstrap = connection_bootstrap(fixture);
    let mut header = canonical_header(fixture, "bytecode-http-server-stream");
    header.mode = "serverStream".to_string();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_bytecode_request(
        "bytecode-http-server-stream-session",
        BytecodeRequestStartFrameWireHeader::Http(header.clone()),
        Vec::new(),
        &bootstrap,
        sender,
    )
    .await;
    assert_bytecode_response_error(
        &mut receiver,
        &header.request_id,
        "serverStream request completed without a response stream",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_task_bytecode_request_reaches_bytecode_admission_and_fails_closed_without_legacy(
) {
    let fixture = fixture();
    let host = test_host();
    let bootstrap = connection_bootstrap(fixture);
    let header = task_header(fixture, "bytecode-task-42");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_bytecode_request(
        "bytecode-task-session",
        BytecodeRequestStartFrameWireHeader::Task(header.clone()),
        b"{}".to_vec(),
        &bootstrap,
        sender,
    )
    .await;
    assert_bytecode_response_error(&mut receiver, &header.request_id, "ingress_selector").await;
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_websocket_connect_bytecode_request_executes_scalar_vm_then_fails_closed_without_legacy(
) {
    let fixture = fixture();
    let host = test_host();
    let bootstrap = connection_bootstrap(fixture);
    let header = websocket_connect_header(fixture, "bytecode-websocket-connect-42");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_bytecode_request(
        "bytecode-websocket-connect-session",
        BytecodeRequestStartFrameWireHeader::WebSocketConnect(header.clone()),
        Vec::new(),
        &bootstrap,
        sender,
    )
    .await;
    assert_bytecode_response_error(
        &mut receiver,
        &header.request_id,
        "bytecode WebSocket connect response mapping is not supported",
    )
    .await;
}
