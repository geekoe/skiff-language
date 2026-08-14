// These integration tests keep the request boundary on image-backed typed
// values and exercise both immediate completion and the production pending lane.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    num::NonZeroUsize,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use skiff_artifact_identity::{gateway_entry_identity, ValidatedBytecodeArtifact};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentOperationBinding, DeploymentRevision, GatewayAdapterArg,
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, IngressProtocol, IngressSelector, PackageArtifact, PackageCallableId,
    PackageLocalAbiSymbol, ServiceContract, ServiceDeployment, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
    compile_package, BytecodeEmissionError, CompilerPlatformSources, ManifestOwner,
    ManifestProvenance, PackageCompileError, PackageCompileInput, PackageCompileOutput,
    PackageSourceInput, Phase1UnsupportedCapability, PublicationManifest, PublicationSourceGraph,
    SourceTree, SourceTreeFile,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_capability_context::{CancellationToken, ExecutionBudgetReason};
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionEntry, DeploymentExecutionImage, LinkLimits,
};
use skiff_runtime_loader::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeLoader,
    FilesystemDeploymentBytecodeContentResolver, HydratedDeploymentBytecode,
};
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
    service_error::CatchIdentity,
    vm_heap::{
        VmContainerElements, VmHeap, VmHeapError, VmHeapOperation, VmHeapPathSegment, VmMapEntry,
        VmRecordField, WritablePathPreparation,
    },
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle},
};
use skiff_runtime_request::execution_budget::{
    AdmittedRequestDeadline, ExecutionBudgetPolicy, ExecutionWinner, TrustedMonotonicClock,
};
use skiff_runtime_request::{
    drive_runtime_bytecode_request, drive_runtime_bytecode_request_controlled, BinaryHttpRequest,
    BinaryHttpRequestMetadata, BoundaryResponse, BytecodeRequestExecutionHandles,
    BytecodeRequestExecutionInput, BytecodeServerStreamFrame, BytecodeServerStreamWriteFailure,
    BytecodeServerStreamWriteFuture, BytecodeServerStreamWriterPort, ControlledBytecodeDrive,
    DrivenBytecodeRequestOwnerInventory, ExecutionBudget,
    GatewayAdapterArg as RequestGatewayAdapterArg,
    GatewayAdapterSource as RequestGatewayAdapterSource, HttpAdapter, HttpAdapterCallable,
    HttpAdapterKind, RequestEnvelope, RequestError, RequestExecutionOwnerInventorySnapshot,
    RequestVmHeap, ResponseEnd, ResponseEvent,
};

fn compile_scalar_package() -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    compile_scalar_package_with_source(
        "function run() -> number {
  return 2.0
}
",
    )
}

fn compile_scalar_package_with_source(
    text: &str,
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    let compiled = compile_test_package_with_source(text).unwrap();
    let handoff = compiled.bytecode_handoff().unwrap();
    let package_artifact = Arc::new(compiled.package().artifact.clone());
    let bytecode = Arc::new(ValidatedBytecodeArtifact::admit(handoff.artifact().clone()).unwrap());
    (package_artifact, bytecode)
}

fn compile_test_package_with_source(
    text: &str,
) -> Result<PackageCompileOutput, PackageCompileError> {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("request manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let package_id =
        skiff_compiler_core::id::PublicationId::parse("example.com/vm-scalar").unwrap();
    static NEXT_TEST_TEMP: AtomicU64 = AtomicU64::new(0);
    let temp = std::env::temp_dir().join(format!(
        "skiff-request-vm-scalar-{}-{}-{}",
        std::process::id(),
        NEXT_TEST_TEMP.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).unwrap();
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
        text.to_string(),
        source_path.display().to_string(),
    )
    .unwrap();
    let package = PackageSourceInput::new(
        PublicationManifest::new(
            package_id,
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
        PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let input = PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        "example.com/vm-scalar",
        true,
    );
    let compiled = compile_package(input);
    std::fs::remove_dir_all(temp).unwrap();
    compiled
}

fn service_contract(
    package_id: &str,
) -> (
    Arc<ServiceContract>,
    skiff_artifact_model::ContractOperationId,
) {
    let operation_id =
        skiff_artifact_identity::contract_operation_id(package_id, "1.0.0", "run").unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: package_id.to_string(),
        contract_version: "1.0.0".to_string(),
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
            service: package_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (Arc::new(contract), operation_id)
}

fn service_deployment(
    package: &PackageArtifact,
    contract: &ServiceContract,
    operation_id: skiff_artifact_model::ContractOperationId,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let package_ref = skiff_artifact_identity::package_artifact_ref(package).unwrap();
    let contract_ref = skiff_artifact_identity::service_contract_ref(contract).unwrap();
    let callable_id = package
        .callable_links
        .keys()
        .next()
        .expect("compiled scalar package has a callable")
        .clone();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref,
        deployment_revision: DeploymentRevision::new("revision:request-vm-scalar"),
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
            display_name: "request vm scalar".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference)
}

fn implementation_callable_id(package: &PackageArtifact, selector: &str) -> PackageCallableId {
    let symbol = package
        .package_local_abi
        .implementation_symbols
        .get(selector)
        .unwrap_or_else(|| panic!("compiled scalar package has no callable {selector}"));
    match symbol {
        PackageLocalAbiSymbol::Callable { callable_id, .. } => callable_id.clone(),
        _ => panic!("compiled scalar symbol {selector} is not callable"),
    }
}

fn scalar_gateway_contract(package_id: &str) -> Arc<ServiceContract> {
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: package_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::new(),
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

fn scalar_gateway_entry(
    key: &str,
    handler: PackageCallableId,
    schema: GatewayExternalSchema,
) -> (GatewayEntryKey, DeploymentGatewayEntry) {
    let key = GatewayEntryKey::parse(key).unwrap();
    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpBody],
            request_body_schema: Some(schema.clone()),
            response_schema: Some(schema),
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let identity = gateway_entry_identity(&protocol_surface).unwrap();
    let entry = DeploymentGatewayEntry {
        gateway_entry_identity: identity,
        protocol_surface,
        handler: Some(handler),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::TypedJson,
            args: vec![GatewayAdapterArg {
                param: "value".to_string(),
                source: GatewayAdapterSource::HttpBody,
            }],
        },
        close_handler: None,
        close_adapter_plan: None,
    };
    (key, entry)
}

fn scalar_gateway_deployment(
    package: &PackageArtifact,
    contract: &ServiceContract,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
    BTreeMap<&'static str, (IngressSelector, GatewayEntryIdentity)>,
) {
    let definitions = [
        ("number", "main.numberBody", GatewayExternalSchema::Number),
        ("bool", "main.boolBody", GatewayExternalSchema::Boolean),
        ("null", "main.nullBody", GatewayExternalSchema::Null),
    ];
    let mut gateway_entries = BTreeMap::new();
    let mut ingress = Vec::new();
    let mut keys = BTreeMap::new();
    for (name, selector, schema) in definitions {
        let (key, entry) =
            scalar_gateway_entry(name, implementation_callable_id(package, selector), schema);
        let selector = IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: format!("/{name}"),
        };
        let identity = entry.gateway_entry_identity.clone();
        ingress.push(DeploymentIngressBinding {
            selector: selector.clone(),
            gateway_entry_key: key.clone(),
        });
        keys.insert(name, (selector, identity));
        gateway_entries.insert(key, entry);
    }

    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:request-vm-scalar-gateways"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(package).unwrap(),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries,
        ingress,
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "request vm scalar gateways".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference, keys)
}

struct ScalarGatewayFixture {
    image: Arc<DeploymentExecutionImage>,
    keys: BTreeMap<&'static str, (IngressSelector, GatewayEntryIdentity)>,
}

impl ScalarGatewayFixture {
    fn build() -> Self {
        let (package, bytecode) = compile_scalar_package_with_source(
            "function numberBody(value: number) -> number {
  return value + 1.0
}

function boolBody(value: bool) -> bool {
  return value
}

function nullBody(value: null) -> null {
  return value
}
",
        );
        let contract = scalar_gateway_contract(package.package_id.as_str());
        let (deployment, deployment_reference, keys) =
            scalar_gateway_deployment(&package, &contract);
        let resolver = TestResolver {
            deployment,
            contract,
            package,
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .unwrap();
        let image = execution_image(hydrated);
        Self { image, keys }
    }

    fn target(&self, name: &str) -> DeploymentExecutionEntry {
        let (selector, identity) = self
            .keys
            .get(name)
            .unwrap_or_else(|| panic!("unknown scalar gateway {name}"));
        self.image.http_gateway_entry(selector, identity).unwrap()
    }
}

fn scalar_gateway_fixture() -> &'static ScalarGatewayFixture {
    static FIXTURE: OnceLock<ScalarGatewayFixture> = OnceLock::new();
    FIXTURE.get_or_init(ScalarGatewayFixture::build)
}

struct PendingSleepFixture {
    image: Arc<DeploymentExecutionImage>,
    selector: IngressSelector,
    gateway_identity: GatewayEntryIdentity,
}

static NEXT_SLEEP_TEST_TEMP: AtomicU64 = AtomicU64::new(0);

impl PendingSleepFixture {
    fn build() -> Self {
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("request crate has a repository root")
            .to_path_buf();
        let fixture_root = repository_root
            .join("runtime/host/src/host/request_entry/phase_4_proof_support/fixtures/vcp4-sleep");
        let artifact_root = std::env::temp_dir().join(format!(
            "skiff-request-p5-sleep-{}-{}-{}",
            std::process::id(),
            NEXT_SLEEP_TEST_TEMP.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&artifact_root).unwrap();
        let platform_sources = CompilerPlatformSources::new(&repository_root)
            .expect("open repository platform sources");
        seed_official_std_package(&platform_sources, &artifact_root)
            .expect("seed canonical std into the same fixture store");
        let receipt = build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &fixture_root,
            &artifact_root,
            "skiff-test",
            true,
        )
        .unwrap_or_else(|error| panic!("production authoring accepts sleep fixture: {error}"));
        let deployment_reference =
            serde_json::from_value::<skiff_artifact_model::ServiceDeploymentRef>(
                receipt
                    .pointer("/serviceDeploymentReceipt/deployment")
                    .cloned()
                    .expect("authoring receipt carries deployment"),
            )
            .expect("authoring deployment receipt remains typed");
        let store = CanonicalArtifactStore::open(&artifact_root).unwrap();
        let deployment = store
            .read_service_deployment(&deployment_reference)
            .expect("read canonical sleep deployment");
        let ingress = deployment
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == "/phase-4/vcp"
            })
            .expect("sleep fixture publishes its exact HTTP ingress");
        let selector = ingress.selector.clone();
        let gateway_identity = deployment
            .gateway_entries
            .get(&ingress.gateway_entry_key)
            .expect("sleep ingress pins a gateway entry")
            .gateway_entry_identity
            .clone();
        let resolver = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
            .expect("open canonical sleep fixture resolver");
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .expect("load canonical sleep fixture closure");
        let image = execution_image(hydrated);
        std::fs::remove_dir_all(&artifact_root).unwrap();
        Self {
            image,
            selector,
            gateway_identity,
        }
    }

    fn target(&self) -> DeploymentExecutionEntry {
        self.image
            .http_gateway_entry(&self.selector, &self.gateway_identity)
            .unwrap()
    }
}

fn pending_sleep_fixture() -> &'static PendingSleepFixture {
    static FIXTURE: OnceLock<PendingSleepFixture> = OnceLock::new();
    FIXTURE.get_or_init(PendingSleepFixture::build)
}

struct ServerStreamFixture {
    image: Arc<DeploymentExecutionImage>,
    selector: IngressSelector,
    gateway_identity: GatewayEntryIdentity,
}

static NEXT_SERVER_STREAM_TEST_TEMP: AtomicU64 = AtomicU64::new(0);

impl ServerStreamFixture {
    fn build() -> Self {
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("request crate has a repository root")
            .to_path_buf();
        let temp = std::env::temp_dir().join(format!(
            "skiff-request-p5-server-stream-{}-{}-{}",
            std::process::id(),
            NEXT_SERVER_STREAM_TEST_TEMP.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let fixture_root = temp.join("source");
        let artifact_root = temp.join("artifacts");
        std::fs::create_dir_all(&fixture_root).unwrap();
        std::fs::create_dir_all(&artifact_root).unwrap();
        for (name, contents) in [
            (
                "package.yml",
                "id: test.skiff/bytecode-vm-phase-5-request\nversion: 1.0.0\n",
            ),
            (
                "service.yml",
                "id: test.skiff/bytecode-vm-phase-5-request\n",
            ),
            ("api.yml", "{}\n"),
            (
                "http.yml",
                "run:\n  method: POST\n  path: /phase-5/request-stream\n  kind: rawHttp\n  handler: main.run\n  adapterArgs:\n    - param: request\n      source: { kind: http.request }\n",
            ),
            (
                "main.skiff",
                "import std\n\nfunction run(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {\n  emit({ tag: \"start\", status: 207, headers: [] })\n  emit({ tag: \"chunk\", value: bytes.fromUtf8(request.body.toUtf8String()) })\n  emit({ tag: \"end\" })\n  return null\n}\n",
            ),
        ] {
            std::fs::write(fixture_root.join(name), contents).unwrap();
        }

        let platform_sources = CompilerPlatformSources::new(&repository_root)
            .expect("open repository platform sources");
        seed_official_std_package(&platform_sources, &artifact_root)
            .expect("seed canonical std into the server-stream fixture store");
        let receipt = build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &fixture_root,
            &artifact_root,
            "skiff-test",
            true,
        )
        .unwrap_or_else(|error| panic!("production authoring accepts stream fixture: {error}"));
        let deployment_reference =
            serde_json::from_value::<skiff_artifact_model::ServiceDeploymentRef>(
                receipt
                    .pointer("/serviceDeploymentReceipt/deployment")
                    .cloned()
                    .expect("authoring receipt carries deployment"),
            )
            .expect("authoring deployment receipt remains typed");
        let store = CanonicalArtifactStore::open(&artifact_root).unwrap();
        let deployment = store
            .read_service_deployment(&deployment_reference)
            .expect("read canonical server-stream deployment");
        let ingress = deployment
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == "/phase-5/request-stream"
            })
            .expect("server-stream fixture publishes its exact HTTP ingress");
        let selector = ingress.selector.clone();
        let gateway_identity = deployment
            .gateway_entries
            .get(&ingress.gateway_entry_key)
            .expect("server-stream ingress pins a gateway entry")
            .gateway_entry_identity
            .clone();
        let resolver = FilesystemDeploymentBytecodeContentResolver::open(&artifact_root)
            .expect("open canonical server-stream fixture resolver");
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .expect("load canonical server-stream fixture closure");
        let image = execution_image(hydrated);
        std::fs::remove_dir_all(temp).unwrap();
        Self {
            image,
            selector,
            gateway_identity,
        }
    }

    fn target(&self) -> DeploymentExecutionEntry {
        self.image
            .http_gateway_entry(&self.selector, &self.gateway_identity)
            .unwrap()
    }
}

fn server_stream_fixture() -> &'static ServerStreamFixture {
    static FIXTURE: OnceLock<ServerStreamFixture> = OnceLock::new();
    FIXTURE.get_or_init(ServerStreamFixture::build)
}

#[derive(Default)]
struct PendingWriterAckState {
    result: Mutex<Option<Result<(), BytecodeServerStreamWriteFailure>>>,
    waker: Mutex<Option<Waker>>,
    waiting: AtomicBool,
    completed: AtomicBool,
    dropped: AtomicUsize,
}

#[derive(Clone, Default)]
struct PendingWriterAck(Arc<PendingWriterAckState>);

impl PendingWriterAck {
    fn complete(&self, result: Result<(), BytecodeServerStreamWriteFailure>) -> bool {
        if !self.0.waiting.load(Ordering::Acquire)
            || self
                .0
                .completed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return false;
        }
        *self
            .0
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
        if let Some(waker) = self
            .0
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            waker.wake();
        }
        true
    }

    fn dropped_count(&self) -> usize {
        self.0.dropped.load(Ordering::Acquire)
    }

    fn has_live_waiter(&self) -> bool {
        self.0.waiting.load(Ordering::Acquire)
    }
}

struct PendingWriterAckFuture {
    ack: PendingWriterAck,
    waiting: bool,
}

impl Future for PendingWriterAckFuture {
    type Output = Result<(), BytecodeServerStreamWriteFailure>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self
            .ack
            .0
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(result) = result {
            self.ack.0.waiting.store(false, Ordering::Release);
            self.waiting = false;
            return Poll::Ready(result);
        }
        self.ack.0.waiting.store(true, Ordering::Release);
        *self
            .ack
            .0
            .waker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(context.waker().clone());
        self.waiting = true;
        Poll::Pending
    }
}

impl Drop for PendingWriterAckFuture {
    fn drop(&mut self) {
        if self.waiting {
            self.ack.0.waiting.store(false, Ordering::Release);
            self.ack
                .0
                .waker
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            self.ack.0.dropped.fetch_add(1, Ordering::AcqRel);
        }
    }
}

enum WriterPlan {
    Ready(Result<(), BytecodeServerStreamWriteFailure>),
    Pending(PendingWriterAck),
}

#[derive(Clone)]
struct ControlledServerStreamWriter {
    plans: Arc<Mutex<VecDeque<WriterPlan>>>,
    frames: Arc<Mutex<Vec<BytecodeServerStreamFrame>>>,
}

impl ControlledServerStreamWriter {
    fn new(plans: impl IntoIterator<Item = WriterPlan>) -> Self {
        Self {
            plans: Arc::new(Mutex::new(plans.into_iter().collect())),
            frames: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn frames(&self) -> Vec<BytecodeServerStreamFrame> {
        self.frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl BytecodeServerStreamWriterPort for ControlledServerStreamWriter {
    fn flush(
        &self,
        frame: BytecodeServerStreamFrame,
        _execution: skiff_runtime_request::OwnedExecutionControl,
    ) -> BytecodeServerStreamWriteFuture {
        let plans = Arc::clone(&self.plans);
        let frames = Arc::clone(&self.frames);
        Box::pin(async move {
            let plan = plans
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .expect("server-stream test writer has one plan per polled frame");
            frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(frame);
            match plan {
                WriterPlan::Ready(result) => result,
                WriterPlan::Pending(ack) => {
                    PendingWriterAckFuture {
                        ack,
                        waiting: false,
                    }
                    .await
                }
            }
        })
    }
}

fn server_stream_input(
    writer: Arc<dyn BytecodeServerStreamWriterPort>,
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    max_response_bytes: NonZeroUsize,
) -> BytecodeRequestExecutionInput {
    let fixture = server_stream_fixture();
    let mut request = request_envelope();
    request.request_id = "phase-5-request-server-stream".to_string();
    request.mode = "serverStream".to_string();
    request.service_id = Some("test.skiff/bytecode-vm-phase-5-request".to_string());
    request.ingress_selector = Some(fixture.selector.clone());
    request.binary_http = Some(BinaryHttpRequest {
        metadata: BinaryHttpRequestMetadata {
            method: "POST".to_string(),
            url: "https://example.test/phase-5/request-stream".to_string(),
            path: "/phase-5/request-stream".to_string(),
            query: vec![skiff_runtime_request::HttpNameValue {
                name: "q".to_string(),
                value: "typed".to_string(),
            }],
            headers: vec![skiff_runtime_request::HttpNameValue {
                name: "x-phase".to_string(),
                value: "5".to_string(),
            }],
        },
        body: b"chunk".to_vec(),
    });
    request.http_adapter = Some(HttpAdapter {
        kind: HttpAdapterKind::RawHttp,
        handler: HttpAdapterCallable::PackageFunction {
            package_id: "test.skiff/bytecode-vm-phase-5-request".to_string(),
            symbol_path: "main.run".to_string(),
        },
        guard: None,
        pre: None,
        adapter_args: vec![RequestGatewayAdapterArg {
            param: "request".to_string(),
            source: RequestGatewayAdapterSource::HttpRequest,
        }],
    });
    BytecodeRequestExecutionInput {
        target: fixture.target(),
        request,
        observer: noop_observer(),
        cancellation,
        execution_budget,
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
            max_response_bytes,
        },
        http_client: None,
        server_stream_writer: Some(writer),
        heap: None,
    }
}

fn expected_server_stream_frames() -> Vec<BytecodeServerStreamFrame> {
    vec![
        BytecodeServerStreamFrame::Start {
            status: 207,
            headers: Vec::new(),
        },
        BytecodeServerStreamFrame::Chunk {
            sequence: 0,
            payload: b"chunk".to_vec(),
        },
        BytecodeServerStreamFrame::End,
    ]
}

struct ServerStreamHeapTrace {
    item_type_tag: u32,
    item_releases: AtomicUsize,
    fail_item_decode: AtomicBool,
}

impl ServerStreamHeapTrace {
    fn new(item_type_tag: u32, fail_item_decode: bool) -> Arc<Self> {
        Arc::new(Self {
            item_type_tag,
            item_releases: AtomicUsize::new(0),
            fail_item_decode: AtomicBool::new(fail_item_decode),
        })
    }
}

struct RecordingServerStreamHeap {
    inner: RequestVmHeap,
    trace: Arc<ServerStreamHeapTrace>,
}

impl RecordingServerStreamHeap {
    fn new(trace: Arc<ServerStreamHeapTrace>) -> Self {
        Self {
            inner: RequestVmHeap::new(RequestHeapLimits::default()),
            trace,
        }
    }
}

impl VmHeap for RecordingServerStreamHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.validate_live(value)
    }

    fn admit_resource_ref(
        &mut self,
        route: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner
            .admit_resource_ref(route, compact_type_tag, flags)
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.snapshot_share(source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.transfer_owner(source)
    }

    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.release_snapshot(owner)?;
        if owner.compact_type_tag().get() == self.trace.item_type_tag {
            self.trace.item_releases.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }

    fn release_resource(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.release_resource(owner)
    }

    fn allocate_array(
        &mut self,
        elements: &[ValueSlot],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.allocate_array(elements, compact_type_tag, flags)
    }

    fn allocate_map(
        &mut self,
        entries: &[VmMapEntry],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.allocate_map(entries, compact_type_tag, flags)
    }

    fn allocate_record(
        &mut self,
        fields: &[VmRecordField],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.allocate_record(fields, compact_type_tag, flags)
    }

    fn allocate_representation(
        &mut self,
        payload: &ValueSlot,
        identity: CatchIdentity,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner
            .allocate_representation(payload, identity, compact_type_tag, flags)
    }

    fn alloc_bytes(&mut self, value: Vec<u8>) -> Result<ValueSlot, VmHeapError> {
        self.inner.alloc_bytes(value)
    }

    fn alloc_typed_bytes(
        &mut self,
        value: Vec<u8>,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.alloc_typed_bytes(value, compact_type_tag, flags)
    }

    fn alloc_string(&mut self, value: String) -> Result<ValueSlot, VmHeapError> {
        self.inner.alloc_string(value)
    }

    fn alloc_typed_string(
        &mut self,
        value: String,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner
            .alloc_typed_string(value, compact_type_tag, flags)
    }

    fn string_value(&self, value: &ValueSlot) -> Result<String, VmHeapError> {
        self.inner.string_value(value)
    }

    fn bytes_value(&self, value: &ValueSlot) -> Result<Vec<u8>, VmHeapError> {
        self.inner.bytes_value(value)
    }

    fn array_get(&self, array: &ValueSlot, index: usize) -> Result<ValueSlot, VmHeapError> {
        self.inner.array_get(array, index)
    }

    fn array_len(&self, array: &ValueSlot) -> Result<usize, VmHeapError> {
        self.inner.array_len(array)
    }

    fn map_get(&self, map: &ValueSlot, key: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.map_get(map, key)
    }

    fn map_len(&self, map: &ValueSlot) -> Result<usize, VmHeapError> {
        self.inner.map_len(map)
    }

    fn map_entry_at(&self, map: &ValueSlot, ordinal: usize) -> Result<VmMapEntry, VmHeapError> {
        self.inner.map_entry_at(map, ordinal)
    }

    fn record_field(&self, record: &ValueSlot, field: &str) -> Result<ValueSlot, VmHeapError> {
        if record.compact_type_tag().get() == self.trace.item_type_tag
            && field == "tag"
            && self.trace.fail_item_decode.swap(false, Ordering::AcqRel)
        {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::RecordField,
                message: "injected server-stream decode failure".to_string(),
            });
        }
        self.inner.record_field(record, field)
    }

    fn representation_payload(&self, representation: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.representation_payload(representation)
    }

    fn array_push_owned(&mut self, array: &ValueSlot, value: ValueSlot) -> Result<(), VmHeapError> {
        self.inner.array_push_owned(array, value)
    }

    fn map_put_owned(
        &mut self,
        map: &ValueSlot,
        key: ValueSlot,
        value: ValueSlot,
    ) -> Result<bool, VmHeapError> {
        self.inner.map_put_owned(map, key, value)
    }

    fn prepare_writable_path(
        &mut self,
        root: &ValueSlot,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
    ) -> Result<WritablePathPreparation, VmHeapError> {
        self.inner.prepare_writable_path(root, segments, selectors)
    }

    fn commit_writable_path(
        &mut self,
        prepared: WritablePathPreparation,
        value: ValueSlot,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.commit_writable_path(prepared, value)
    }

    fn get_dense_field(
        &self,
        record: &ValueSlot,
        field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.get_dense_field(record, field_ordinal)
    }

    fn take_dense_field(
        &mut self,
        record: &ValueSlot,
        field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.take_dense_field(record, field_ordinal)
    }

    fn container_elements(
        &self,
        container: &ValueSlot,
    ) -> Result<VmContainerElements, VmHeapError> {
        self.inner.container_elements(container)
    }
}

fn server_stream_item_type_tag() -> u32 {
    let target = server_stream_fixture().target();
    let function = target
        .image()
        .functions()
        .get(target.function().get() as usize)
        .filter(|row| row.index() == target.function())
        .expect("server-stream entry function remains exact");
    function
        .stream_result_type_ref()
        .expect("server-stream function has exact stream authority");
    let item_type_tag = function
        .instructions()
        .iter()
        .enumerate()
        .find(|(_, instruction)| instruction.opcode() == skiff_artifact_model::Opcode::EmitStream)
        .and_then(|(position, _)| function.stack_map().entries().get(position))
        .and_then(|entry| entry.stack_before().last())
        .map(|value| value.ty().get())
        .expect("linked EmitStream has one exact item type");
    assert_ne!(
        item_type_tag, 0,
        "typed server-stream items never use the legacy zero tag"
    );
    item_type_tag
}

struct TestMonotonicClock(Mutex<Instant>);

impl TestMonotonicClock {
    fn new(now: Instant) -> Self {
        Self(Mutex::new(now))
    }

    fn set(&self, now: Instant) {
        *self.0.lock().unwrap() = now;
    }
}

impl TrustedMonotonicClock for TestMonotonicClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn pending_sleep_input(
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
) -> BytecodeRequestExecutionInput {
    let fixture = pending_sleep_fixture();
    let mut request = request_envelope();
    request.request_id = "phase-5-pending-sleep-race".to_string();
    request.service_id = Some("test.skiff/bytecode-vm-phase-4".to_string());
    request.ingress_selector = Some(fixture.selector.clone());
    request.binary_http = Some(BinaryHttpRequest {
        metadata: BinaryHttpRequestMetadata {
            method: "POST".to_string(),
            url: "https://example.test/phase-4/vcp".to_string(),
            path: "/phase-4/vcp".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        body: b"1".to_vec(),
    });
    request.http_adapter = Some(HttpAdapter {
        kind: HttpAdapterKind::TypedJson,
        handler: HttpAdapterCallable::PackageFunction {
            package_id: "test.skiff/bytecode-vm-phase-4".to_string(),
            symbol_path: "main.run".to_string(),
        },
        guard: None,
        pre: None,
        adapter_args: vec![RequestGatewayAdapterArg {
            param: "seed".to_string(),
            source: RequestGatewayAdapterSource::HttpBody,
        }],
    });
    BytecodeRequestExecutionInput {
        target: fixture.target(),
        request,
        observer: noop_observer(),
        cancellation,
        execution_budget,
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
            max_response_bytes: NonZeroUsize::new(1024).unwrap(),
        },
        http_client: None,
        server_stream_writer: None,
        heap: None,
    }
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
        let actual = skiff_artifact_identity::service_deployment_ref(&self.deployment);
        anyhow::ensure!(&actual == reference, "deployment reference mismatch");
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &skiff_artifact_model::ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        let actual = skiff_artifact_identity::service_contract_ref(&self.contract).unwrap();
        anyhow::ensure!(&actual == reference, "contract reference mismatch");
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        reference: &skiff_artifact_model::PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        let actual = skiff_artifact_identity::package_artifact_ref(&self.package).unwrap();
        anyhow::ensure!(&actual == reference, "package reference mismatch");
        Ok(Arc::clone(&self.package))
    }

    fn resolve_package_bytecode(
        &self,
        package: &skiff_artifact_model::PackageArtifactRef,
        reference: &skiff_artifact_model::BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        let actual_package = skiff_artifact_identity::package_artifact_ref(&self.package).unwrap();
        anyhow::ensure!(&actual_package == package, "bytecode package mismatch");
        anyhow::ensure!(
            self.bytecode.reference() == reference,
            "bytecode reference mismatch"
        );
        Ok(Arc::clone(&self.bytecode))
    }
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

fn execution_image(hydrated: HydratedDeploymentBytecode) -> Arc<DeploymentExecutionImage> {
    Arc::new(link_deployment_execution_image(hydrated, &generous_link_limits()).unwrap())
}

fn request_envelope() -> RequestEnvelope {
    RequestEnvelope {
        request_id: "scalar-bytecode-request".to_string(),
        mode: "unary".to_string(),
        target: "display-only".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some("example.com/vm-scalar".to_string()),
        build_id: "legacy-build".to_string(),
        service_protocol_identity: "legacy-protocol".to_string(),
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

fn scalar_gateway_request(
    name: &str,
    kind: HttpAdapterKind,
    body: &[u8],
    adapter_args: Vec<RequestGatewayAdapterArg>,
) -> RequestEnvelope {
    let mut request = request_envelope();
    let path = format!("/{name}");
    request.ingress_selector = Some(IngressSelector {
        protocol: IngressProtocol::Http,
        method: Some("POST".to_string()),
        path: path.clone(),
    });
    request.binary_http = Some(BinaryHttpRequest {
        metadata: BinaryHttpRequestMetadata {
            method: "POST".to_string(),
            url: format!("https://example.test{path}"),
            path,
            query: Vec::new(),
            headers: Vec::new(),
        },
        body: body.to_vec(),
    });
    request.http_adapter = Some(HttpAdapter {
        kind,
        handler: HttpAdapterCallable::PackageFunction {
            package_id: "example.com/vm-scalar".to_string(),
            symbol_path: name.to_string(),
        },
        guard: None,
        pre: None,
        adapter_args,
    });
    request
}

fn http_body_argument() -> RequestGatewayAdapterArg {
    RequestGatewayAdapterArg {
        param: "value".to_string(),
        source: RequestGatewayAdapterSource::HttpBody,
    }
}

fn noop_observer() -> BytecodeExecutionObserver {
    BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
        router_session_id: "request-test-session".to_string(),
        request_id: "request-test".to_string(),
    })
}

fn run_synchronous_request(
    input: BytecodeRequestExecutionInput,
) -> Result<BoundaryResponse, RequestError> {
    let driven = drive_runtime_bytecode_request(input);
    let snapshot = driven.owner_inventory.into_snapshot();
    assert_synchronous_owner_inventory(&snapshot);
    drop(driven.retention);
    driven.result
}

fn assert_synchronous_owner_inventory(snapshot: &RequestExecutionOwnerInventorySnapshot) {
    for (domain_name, domain) in [
        ("pending", snapshot.pending),
        ("resource", snapshot.resource),
        ("child", snapshot.child),
    ] {
        assert_eq!(
            domain.current, 0,
            "synchronous Phase 1 request created a live {domain_name} owner"
        );
        assert!(
            !domain.ever_created,
            "synchronous Phase 1 request ever created a {domain_name} owner"
        );
    }
}

fn execute_scalar_gateway(
    name: &str,
    kind: HttpAdapterKind,
    body: &[u8],
    adapter_args: Vec<RequestGatewayAdapterArg>,
) -> Result<BoundaryResponse, RequestError> {
    run_synchronous_request(BytecodeRequestExecutionInput {
        target: scalar_gateway_fixture().target(name),
        request: scalar_gateway_request(name, kind, body, adapter_args),
        observer: noop_observer(),
        cancellation: CancellationToken::new(),
        execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
            max_response_bytes: NonZeroUsize::new(1024).unwrap(),
        },
        http_client: None,
        server_stream_writer: None,
        heap: None,
    })
}

fn response_payload(response: BoundaryResponse) -> Vec<u8> {
    let BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload))) = response
    else {
        panic!("scalar gateway returned a non-payload response: {response:?}");
    };
    payload
}

fn assert_phase_1_compiler_rejection(
    error: PackageCompileError,
    expected_capability: Phase1UnsupportedCapability,
    expected_location: &str,
) {
    let PackageCompileError::BytecodeEmission {
        source:
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability,
                module_path,
                function_key,
                location,
            },
    } = error
    else {
        panic!("expected typed Phase 1 compiler containment, got {error:?}");
    };
    assert_eq!(capability, expected_capability);
    assert_eq!(module_path, "main");
    assert_eq!(function_key.as_deref(), Some("main::run"));
    assert_eq!(location, expected_location);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_request_freezes_started_all_zero_inventory() {
        let driven = drive_runtime_bytecode_request(BytecodeRequestExecutionInput {
            target: scalar_gateway_fixture().target("number"),
            request: scalar_gateway_request(
                "number",
                HttpAdapterKind::TypedJson,
                b"2",
                vec![http_body_argument()],
            ),
            observer: noop_observer(),
            cancellation: CancellationToken::new(),
            execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
            handles: BytecodeRequestExecutionHandles {
                request_heap_limits: RequestHeapLimits::default(),
                max_response_bytes: NonZeroUsize::new(1024).unwrap(),
            },
            http_client: None,
            server_stream_writer: None,
            heap: None,
        });

        match driven.owner_inventory {
            DrivenBytecodeRequestOwnerInventory::Started(snapshot) => {
                assert_synchronous_owner_inventory(&snapshot);
            }
            DrivenBytecodeRequestOwnerInventory::NotStarted(_) => {
                panic!("completed drive must freeze a Started owner inventory");
            }
        }
        let response = driven.result.expect("scalar gateway request must complete");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response_payload(response)).unwrap(),
            serde_json::json!(3.0)
        );
        drop(driven.retention);
    }

    #[test]
    fn malformed_typed_json_body_freezes_not_started_all_zero_inventory() {
        let driven = drive_runtime_bytecode_request(BytecodeRequestExecutionInput {
            target: scalar_gateway_fixture().target("number"),
            request: scalar_gateway_request(
                "number",
                HttpAdapterKind::TypedJson,
                b"{",
                vec![http_body_argument()],
            ),
            observer: noop_observer(),
            cancellation: CancellationToken::new(),
            execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
            handles: BytecodeRequestExecutionHandles {
                request_heap_limits: RequestHeapLimits::default(),
                max_response_bytes: NonZeroUsize::new(1024).unwrap(),
            },
            http_client: None,
            server_stream_writer: None,
            heap: None,
        });

        match driven.owner_inventory {
            DrivenBytecodeRequestOwnerInventory::NotStarted(snapshot) => {
                assert_synchronous_owner_inventory(&snapshot);
            }
            DrivenBytecodeRequestOwnerInventory::Started(_) => {
                panic!("start failure must freeze a NotStarted owner inventory");
            }
        }
        assert!(matches!(
            driven.result,
            Err(RequestError::Decode(ref message)) if message.contains("not valid JSON")
        ));
        drop(driven.retention);
    }

    #[test]
    fn typed_json_number_body_materializes_against_pinned_entry() {
        let response = execute_scalar_gateway(
            "number",
            HttpAdapterKind::TypedJson,
            b"2",
            vec![http_body_argument()],
        )
        .unwrap();

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response_payload(response)).unwrap(),
            serde_json::json!(3.0)
        );
    }

    #[test]
    fn typed_json_bool_and_null_bodies_materialize_as_immediates() {
        for (name, body, expected) in [
            ("bool", b"true".as_slice(), serde_json::json!(true)),
            ("null", b"null".as_slice(), serde_json::Value::Null),
        ] {
            let response = execute_scalar_gateway(
                name,
                HttpAdapterKind::TypedJson,
                body,
                vec![http_body_argument()],
            )
            .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&response_payload(response)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn typed_json_malformed_body_fails_closed_before_vm_start() {
        let error = execute_scalar_gateway(
            "number",
            HttpAdapterKind::TypedJson,
            b"{",
            vec![http_body_argument()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Decode(message) if message.contains("not valid JSON")
        ));
    }

    #[test]
    fn typed_json_body_must_match_exact_pinned_parameter_type() {
        let error = execute_scalar_gateway(
            "number",
            HttpAdapterKind::TypedJson,
            b"true",
            vec![http_body_argument()],
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Decode(message)
                if message.contains("exact pinned number type for parameter 0")
        ));
    }

    #[test]
    fn typed_json_non_scalar_bodies_fail_closed() {
        for body in [b"[]".as_slice(), b"{\"value\":2}".as_slice()] {
            let error = execute_scalar_gateway(
                "number",
                HttpAdapterKind::TypedJson,
                body,
                vec![http_body_argument()],
            )
            .unwrap_err();
            assert!(matches!(
                error,
                RequestError::Unsupported(message) if message.contains("is non-scalar")
            ));
        }
    }

    #[test]
    fn typed_json_adapter_arity_must_match_exact_pinned_entry() {
        let error = execute_scalar_gateway("number", HttpAdapterKind::TypedJson, b"2", Vec::new())
            .unwrap_err();

        assert!(matches!(
            error,
            RequestError::Decode(message)
                if message.contains("0 arguments") && message.contains("1 parameters")
        ));
    }

    #[test]
    fn raw_http_body_requires_the_exact_linked_bytes_parameter() {
        let error = execute_scalar_gateway(
            "number",
            HttpAdapterKind::RawHttp,
            b"2",
            vec![http_body_argument()],
        )
        .unwrap_err();

        assert!(
            matches!(
                &error,
                RequestError::Decode(message)
                    if message.contains("is not exact builtin \"bytes\"")
            ),
            "unexpected raw HTTP body result: {error:?}"
        );
    }

    #[test]
    fn request_heap_scalar_returns_payload() {
        let (package, bytecode) = compile_scalar_package();
        let (contract, operation_id) = service_contract(package.package_id.as_str());
        let (deployment, deployment_reference) =
            service_deployment(&package, &contract, operation_id.clone());
        let resolver = TestResolver {
            deployment,
            contract,
            package,
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .unwrap();
        let image = execution_image(hydrated);
        let target = image.operation_entry(&operation_id).unwrap();
        let response = run_synchronous_request(BytecodeRequestExecutionInput {
            target,
            request: request_envelope(),
            observer: noop_observer(),
            cancellation: CancellationToken::new(),
            execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
            handles: BytecodeRequestExecutionHandles {
                request_heap_limits: RequestHeapLimits::default(),
                max_response_bytes: NonZeroUsize::new(1024).unwrap(),
            },
            http_client: None,
            server_stream_writer: None,
            heap: None,
        })
        .unwrap();

        let BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload))) = response
        else {
            panic!("bytecode request returned a non-payload response: {response:?}");
        };
        assert_eq!(serde_json::from_slice::<f64>(&payload).unwrap(), 2.0);
    }

    #[test]
    fn phase_5_stream_ready_flushes_never_mint_pending_owner() {
        let writer = Arc::new(ControlledServerStreamWriter::new([
            WriterPlan::Ready(Ok(())),
            WriterPlan::Ready(Ok(())),
            WriterPlan::Ready(Ok(())),
        ]));
        let driven = drive_runtime_bytecode_request(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(64).unwrap(),
        ));

        assert!(
            matches!(&driven.result, Ok(BoundaryResponse::StreamSent)),
            "unexpected server-stream result: {:?}",
            driven.result
        );
        assert_eq!(writer.frames(), expected_server_stream_frames());
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(!snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert_eq!(snapshot.child.current, 0);
        drop(driven.retention);
    }

    #[test]
    fn phase_5_stream_typed_decode_success_releases_every_emitted_item_once() {
        let trace = ServerStreamHeapTrace::new(server_stream_item_type_tag(), false);
        let writer = Arc::new(ControlledServerStreamWriter::new([
            WriterPlan::Ready(Ok(())),
            WriterPlan::Ready(Ok(())),
            WriterPlan::Ready(Ok(())),
        ]));
        let mut input = server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(64).unwrap(),
        );
        input.heap = Some(Box::new(RecordingServerStreamHeap::new(Arc::clone(&trace))));

        let driven = drive_runtime_bytecode_request(input);

        assert!(
            matches!(&driven.result, Ok(BoundaryResponse::StreamSent)),
            "unexpected server-stream result: {:?}",
            driven.result
        );
        assert_eq!(writer.frames(), expected_server_stream_frames());
        assert_eq!(trace.item_releases.load(Ordering::Acquire), 3);
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert_eq!(snapshot.child.current, 0);
        drop(driven.retention);
    }

    #[test]
    fn phase_5_stream_typed_decode_failure_releases_item_before_terminal() {
        let trace = ServerStreamHeapTrace::new(server_stream_item_type_tag(), true);
        let writer = Arc::new(ControlledServerStreamWriter::new([]));
        let mut input = server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(64).unwrap(),
        );
        input.heap = Some(Box::new(RecordingServerStreamHeap::new(Arc::clone(&trace))));

        let driven = drive_runtime_bytecode_request(input);

        assert!(driven.result.is_err());
        assert!(writer.frames().is_empty());
        assert_eq!(trace.item_releases.load(Ordering::Acquire), 1);
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert_eq!(snapshot.child.current, 0);
        drop(driven.retention);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn phase_5_stream_pending_flush_uses_shared_wake_and_commits_after_ack() {
        let ack = PendingWriterAck::default();
        let writer = Arc::new(ControlledServerStreamWriter::new([
            WriterPlan::Ready(Ok(())),
            WriterPlan::Pending(ack.clone()),
            WriterPlan::Ready(Ok(())),
        ]));
        let drive = drive_runtime_bytecode_request_controlled(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(64).unwrap(),
        ));
        let parked = match drive {
            ControlledBytecodeDrive::Parked(parked) => parked,
            ControlledBytecodeDrive::Complete(driven) => {
                panic!("actual-Pending writer must park: {:?}", driven.result)
            }
        };

        assert_eq!(writer.frames(), expected_server_stream_frames()[..2]);
        assert!(
            !parked.pending_completion().complete(),
            "server-stream Pending cannot use the Sleep-only Empty authority"
        );
        assert!(
            ack.complete(Ok(())),
            "the real flush future owns the waiter"
        );
        assert!(
            !ack.complete(Ok(())),
            "the exact flush acknowledgement has one completion winner"
        );
        let resumed = tokio::task::spawn_blocking(move || parked.resume())
            .await
            .unwrap();
        let ControlledBytecodeDrive::Complete(driven) = resumed else {
            panic!("one ACK resumes the exact EmitStream site")
        };

        assert!(matches!(driven.result, Ok(BoundaryResponse::StreamSent)));
        assert_eq!(writer.frames(), expected_server_stream_frames());
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert_eq!(snapshot.child.current, 0);
        drop(driven.retention);
    }

    #[test]
    fn phase_5_stream_forged_early_writer_deadline_cannot_win_request_budget() {
        let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let writer = Arc::new(ControlledServerStreamWriter::new([WriterPlan::Ready(Err(
            BytecodeServerStreamWriteFailure::DeadlineExceeded,
        ))]));
        let driven = drive_runtime_bytecode_request(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::clone(&execution_budget),
            NonZeroUsize::new(64).unwrap(),
        ));

        assert!(driven.result.is_err());
        assert!(!matches!(
            &driven.result,
            Err(RequestError::ExecutionBudgetExceeded { .. }) | Err(RequestError::Cancelled)
        ));
        assert_eq!(writer.frames().len(), 1);
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(!snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        drop(driven.retention);
    }

    #[test]
    fn phase_5_stream_writer_failure_terminates_resource_and_clears_inventory() {
        let writer = Arc::new(ControlledServerStreamWriter::new([WriterPlan::Ready(Err(
            BytecodeServerStreamWriteFailure::WriterFailed("ack failed".to_string()),
        ))]));
        let driven = drive_runtime_bytecode_request(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(64).unwrap(),
        ));

        assert!(driven.result.is_err());
        assert_eq!(writer.frames().len(), 1);
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        drop(driven.retention);
    }

    #[test]
    fn phase_5_stream_response_limit_rejects_chunk_before_transport_poll() {
        let writer = Arc::new(ControlledServerStreamWriter::new([WriterPlan::Ready(Ok(
            (),
        ))]));
        let driven = drive_runtime_bytecode_request(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(4).unwrap(),
        ));

        assert!(driven.result.is_err());
        assert_eq!(
            writer.frames(),
            vec![BytecodeServerStreamFrame::Start {
                status: 207,
                headers: Vec::new(),
            }],
            "the over-limit chunk never reaches the transport port"
        );
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(!snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        drop(driven.retention);
    }

    async fn assert_cancelled_stream_retains_waiter_until_late_ack(
        late_result: Result<(), BytecodeServerStreamWriteFailure>,
    ) {
        let cancellation = CancellationToken::new();
        let ack = PendingWriterAck::default();
        let writer = Arc::new(ControlledServerStreamWriter::new([WriterPlan::Pending(
            ack.clone(),
        )]));
        let drive = drive_runtime_bytecode_request_controlled(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            cancellation.clone(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(64).unwrap(),
        ));
        let parked = match drive {
            ControlledBytecodeDrive::Parked(parked) => parked,
            ControlledBytecodeDrive::Complete(driven) => {
                panic!("actual-Pending start flush must park: {:?}", driven.result)
            }
        };

        cancellation.cancel();
        let resumed = tokio::task::spawn_blocking(move || parked.resume())
            .await
            .unwrap();
        let ControlledBytecodeDrive::Complete(driven) = resumed else {
            panic!("cancellation must settle the shared pending cell")
        };
        assert!(matches!(driven.result, Err(RequestError::Cancelled)));
        assert_eq!(ack.dropped_count(), 0);
        assert!(
            ack.has_live_waiter(),
            "the irrevocably enqueued frame retains its real ACK waiter after request terminal"
        );
        assert_eq!(writer.frames().len(), 1);
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert!(
            ack.complete(late_result),
            "the Router/session terminal still resolves the retained writer future"
        );
        for _ in 0..100 {
            if !ack.has_live_waiter() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!ack.has_live_waiter());
        assert_eq!(ack.dropped_count(), 0);
        assert!(
            !ack.complete(Ok(())),
            "a late transport result has exactly one completion"
        );
        drop(driven.retention);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn phase_5_stream_cancel_retains_writer_until_late_ok_without_reviving_request() {
        assert_cancelled_stream_retains_waiter_until_late_ack(Ok(())).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn phase_5_stream_cancel_retains_writer_until_late_error_without_reviving_request() {
        assert_cancelled_stream_retains_waiter_until_late_ack(Err(
            BytecodeServerStreamWriteFailure::RouterDisconnected,
        ))
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn phase_5_stream_due_deadline_retains_writer_until_late_ack_without_reviving_request() {
        let start = Instant::now();
        let deadline_at = start.checked_add(Duration::from_secs(30)).unwrap();
        let clock = Arc::new(TestMonotonicClock::new(start));
        let execution_budget = Arc::new(ExecutionBudget::new(
            ExecutionBudgetPolicy::runtime_default(),
            Some(AdmittedRequestDeadline::new(deadline_at)),
            clock.clone(),
        ));
        let ack = PendingWriterAck::default();
        let writer = Arc::new(ControlledServerStreamWriter::new([WriterPlan::Pending(
            ack.clone(),
        )]));
        let drive = drive_runtime_bytecode_request_controlled(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::clone(&execution_budget),
            NonZeroUsize::new(64).unwrap(),
        ));
        let parked = match drive {
            ControlledBytecodeDrive::Parked(parked) => parked,
            ControlledBytecodeDrive::Complete(driven) => {
                panic!("actual-Pending start flush must park: {:?}", driven.result)
            }
        };

        clock.set(deadline_at.checked_add(Duration::from_millis(1)).unwrap());
        assert_eq!(
            execution_budget.pending_terminal_winner(),
            Some(ExecutionWinner::DeadlineExceeded)
        );
        let resumed = tokio::task::spawn_blocking(move || parked.resume())
            .await
            .unwrap();
        let ControlledBytecodeDrive::Complete(driven) = resumed else {
            panic!("deadline must settle the shared pending cell")
        };
        assert!(matches!(
            driven.result,
            Err(RequestError::ExecutionBudgetExceeded {
                reason: ExecutionBudgetReason::DeadlineExceeded,
                ..
            })
        ));
        assert_eq!(ack.dropped_count(), 0);
        assert!(ack.has_live_waiter());
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert!(ack.complete(Ok(())));
        for _ in 0..100 {
            if !ack.has_live_waiter() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!ack.has_live_waiter());
        assert_eq!(ack.dropped_count(), 0);
        assert!(!ack.complete(Ok(())));
        drop(driven.retention);
    }

    #[test]
    fn phase_5_stream_begin_pending_failure_closes_in_flight_resource() {
        let ack = PendingWriterAck::default();
        let writer = Arc::new(ControlledServerStreamWriter::new([WriterPlan::Pending(
            ack.clone(),
        )]));
        let drive = drive_runtime_bytecode_request_controlled(server_stream_input(
            Arc::clone(&writer) as Arc<dyn BytecodeServerStreamWriterPort>,
            CancellationToken::new(),
            Arc::new(ExecutionBudget::for_runtime_request(None)),
            NonZeroUsize::new(64).unwrap(),
        ));
        let ControlledBytecodeDrive::Complete(driven) = drive else {
            panic!("Pending outside Tokio must fail before publication")
        };

        assert!(driven.result.is_err());
        assert_eq!(ack.dropped_count(), 1);
        assert!(!ack.complete(Ok(())));
        assert_eq!(writer.frames().len(), 1);
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(!snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        drop(driven.retention);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn phase_5_first_poll_queued_sleep_wake_loses_to_cancel_before_resume() {
        let cancellation = CancellationToken::new();
        let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let drive = drive_runtime_bytecode_request_controlled(pending_sleep_input(
            cancellation.clone(),
            execution_budget,
        ));
        let parked = match drive {
            ControlledBytecodeDrive::Parked(parked) => parked,
            ControlledBytecodeDrive::Complete(driven) => panic!(
                "typed sleep must park after its real future first-polls Pending: {:?}",
                driven.result
            ),
        };

        let completion = parked.pending_completion();
        assert!(completion.complete());
        assert!(!completion.complete(), "queued wake has one host winner");
        cancellation.cancel();
        let ControlledBytecodeDrive::Complete(driven) = parked.resume() else {
            panic!("cancelled queued sleep wake must reach one terminal")
        };

        assert!(matches!(driven.result, Err(RequestError::Cancelled)));
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert_eq!(snapshot.child.current, 0);
        drop(driven.retention);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn phase_5_first_poll_queued_sleep_wake_loses_to_due_deadline_before_resume() {
        let start = Instant::now();
        let deadline_at = start.checked_add(Duration::from_secs(30)).unwrap();
        let clock = Arc::new(TestMonotonicClock::new(start));
        let execution_budget = Arc::new(ExecutionBudget::new(
            ExecutionBudgetPolicy::runtime_default(),
            Some(AdmittedRequestDeadline::new(deadline_at)),
            clock.clone(),
        ));
        let drive = drive_runtime_bytecode_request_controlled(pending_sleep_input(
            CancellationToken::new(),
            execution_budget,
        ));
        let parked = match drive {
            ControlledBytecodeDrive::Parked(parked) => parked,
            ControlledBytecodeDrive::Complete(driven) => panic!(
                "typed sleep must park after its real future first-polls Pending: {:?}",
                driven.result
            ),
        };

        let completion = parked.pending_completion();
        assert!(completion.complete());
        assert!(!completion.complete(), "queued wake has one host winner");
        clock.set(deadline_at.checked_add(Duration::from_millis(1)).unwrap());
        let ControlledBytecodeDrive::Complete(driven) = parked.resume() else {
            panic!("due-deadline queued sleep wake must reach one terminal")
        };

        assert!(matches!(
            driven.result,
            Err(RequestError::ExecutionBudgetExceeded {
                reason: ExecutionBudgetReason::DeadlineExceeded,
                ..
            })
        ));
        let snapshot = driven.owner_inventory.into_snapshot();
        assert_eq!(snapshot.pending.current, 0);
        assert!(snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert_eq!(snapshot.child.current, 0);
        drop(driven.retention);
    }

    #[test]
    fn string_shape_is_rejected_by_typed_compiler_containment() {
        let error = compile_test_package_with_source(
            "function run() -> string {
  return \"disabled\"
}
",
        )
        .unwrap_err();

        assert_phase_1_compiler_rejection(
            error,
            Phase1UnsupportedCapability::ValueShape,
            "return type",
        );
    }

    #[test]
    fn aggregate_shape_is_rejected_by_typed_compiler_containment() {
        let error = compile_test_package_with_source(
            "function run() -> Array<string> {
  return [\"a\", \"b\"]
}
",
        )
        .unwrap_err();

        let PackageCompileError::BytecodeEmission {
            source:
                BytecodeEmissionError::UnsupportedConstruct {
                    construct,
                    location,
                    ..
                },
        } = error
        else {
            panic!("expected typed Phase 2 aggregate containment, got {error:?}");
        };
        assert_eq!(construct, "phase 2 record/array value shape");
        assert!(location.contains("return type"), "{location}");
        assert!(location.contains("ValueShape"), "{location}");
    }

    #[test]
    fn native_bytes_wrapper_is_rejected_by_typed_compiler_containment() {
        let error = compile_test_package_with_source(
            "function run() -> bytes {
  return bytes.fromUtf8(\"disabled\")
}
",
        )
        .unwrap_err();

        assert_phase_1_compiler_rejection(
            error,
            Phase1UnsupportedCapability::HostTarget,
            "native executable",
        );
    }
}
