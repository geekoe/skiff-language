use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterPlan, GatewayDispatchMode, GatewayEntryIdentity,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayProtocolSurface,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketRpcProfile, GatewayWebSocketShapeVersion, OperationTargetRef, PackageArtifact,
    PackageCallableId, PackageCallableSignature, PackageLocalAbiSymbol, PackageSchemaIndex,
    RuntimeAssembly, ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::{assembly::resolve_runtime_assembly, storage::CanonicalArtifactStore};
use skiff_runtime_activation::{ActivationContext, ActivationId, RequestActivationContext};
use skiff_runtime_boundary::http::HttpBoundaryResponseStreamEvent;
use skiff_runtime_capability_context::{
    BinaryHttpRequestContext, DbCapabilityContext, RequestPayloadContext,
};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, HydratedPackageCode, PublicationResourceTable,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_test_runner::canonical_std_seed::seed_canonical_std;

use crate::{
    actor_executor_test_runtime as test_runtime,
    capabilities::TimeCapabilityContext,
    error::RuntimeError,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    AdmittedPackageSchemaRecords, Interpreter, RuntimeAssemblyEvalResolver,
    RuntimeAssemblyEvalTarget, RuntimeHttpGatewayCallable, RuntimeHttpGatewayExecutionTarget,
};

const PACKAGE_ID: &str = "example.com/runtime-http-gateway-execution";
const SERVICE_ID: &str = "example.com/runtime-http-gateway-execution-service";
const VERSION: &str = "1.0.0";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FIXTURE: OnceLock<Arc<CompiledGatewayFixture>> = OnceLock::new();

#[tokio::test]
async fn runtime_http_gateway_typed_unary_runs_exact_guard_pre_and_private_handler() {
    let fixture = fixture();
    let target = fixture.target_for_path("/typed");
    assert_ne!(target.handler.addr, target.pre.as_ref().unwrap().addr);
    assert_ne!(target.handler.addr, target.guard.as_ref().unwrap().addr);

    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let body = br#""body-remains-opaque-until-handler-adaptation""#;
    let response = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, target.eval.clone()),
            request(&target.key, "POST", "/typed", body),
            &target,
        )
        .await
        .expect("typed gateway should execute its exact linked private callables");

    assert_eq!(response.status, 200);
    assert_eq!(response.body, br#""/typed""#);
    assert_eq!(response.headers[0].value, "application/json; charset=utf-8");
}

#[tokio::test]
async fn runtime_http_gateway_guard_short_circuits_before_typed_body_decode_and_pre() {
    let fixture = fixture();
    let target = fixture.target_for_path("/blocked");
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let invalid_utf8 = [0xff, 0xfe];

    let response = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, target.eval.clone()),
            request(&target.key, "POST", "/blocked", &invalid_utf8),
            &target,
        )
        .await
        .expect("exact guard response must bypass typed body decoding and handler execution");

    assert_eq!(response.status, 204);
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn runtime_http_gateway_raw_unary_preserves_binary_http_context_and_body() {
    let fixture = fixture();
    let target = fixture.target_for_path("/raw");
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let body = [0, 1, 2, 0xff];

    let response = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, target.eval.clone()),
            request(&target.key, "POST", "/raw", &body),
            &target,
        )
        .await
        .expect("raw gateway should execute the exact private handler");

    assert_eq!(response.status, 201);
    assert_eq!(response.body, body);
}

#[tokio::test]
async fn runtime_http_gateway_raw_server_stream_uses_exact_start_chunk_end_sequence() {
    let fixture = fixture();
    let target = fixture.target_for_path("/stream");
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let body = b"stream-body";
    let mut events = Vec::new();

    interpreter
        .execute_runtime_http_gateway_server_stream(
            execution_context(&interpreter, target.eval.clone()),
            request(&target.key, "POST", "/stream", body),
            &target,
            |event| {
                events.push(event);
                Ok(())
            },
        )
        .await
        .expect("raw gateway stream should share the canonical stream producer/consumer path");

    assert_eq!(
        events,
        vec![
            HttpBoundaryResponseStreamEvent::Start {
                status: 202,
                headers: Vec::new(),
            },
            HttpBoundaryResponseStreamEvent::Chunk(body.to_vec()),
            HttpBoundaryResponseStreamEvent::End,
        ]
    );
}

#[tokio::test]
async fn runtime_http_gateway_stream_cancellation_cleans_up_and_next_stream_completes() {
    let fixture = fixture();
    let target = fixture.target_for_path("/stream");
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let body = b"cancel-me";

    let error = interpreter
        .execute_runtime_http_gateway_server_stream(
            execution_context(&interpreter, target.eval.clone()),
            request(&target.key, "POST", "/stream", body),
            &target,
            |_| Err(RuntimeError::Cancelled),
        )
        .await
        .expect_err("consumer cancellation must cancel the shared deferred producer");
    assert!(error.is_cancelled());

    let fresh_target = fixture.target_for_path("/stream");
    let mut events = Vec::new();
    interpreter
        .execute_runtime_http_gateway_server_stream(
            execution_context(&interpreter, fresh_target.eval.clone()),
            request(&fresh_target.key, "POST", "/stream", b"after-cancel"),
            &fresh_target,
            |event| {
                events.push(event);
                Ok(())
            },
        )
        .await
        .expect("cancelled producer must be removed before the next gateway stream");
    assert!(matches!(
        events.as_slice(),
        [
            HttpBoundaryResponseStreamEvent::Start { .. },
            HttpBoundaryResponseStreamEvent::Chunk(_),
            HttpBoundaryResponseStreamEvent::End
        ]
    ));
}

#[tokio::test]
async fn runtime_http_gateway_wrong_target_signature_mode_and_adapter_fail_closed() {
    let fixture = fixture();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());

    let exact_target = fixture.target_for_path("/typed");
    let wrong_key = GatewayEntryKey::parse("http:wrong").unwrap();
    let error = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, exact_target.eval.clone()),
            request(&wrong_key, "POST", "/typed", br#""value""#),
            &exact_target,
        )
        .await
        .expect_err("a different request target key must not enter the exact gateway plan");
    assert!(error.to_string().contains("exact gateway entry key"));

    let mut wrong_target = fixture.target_for_path("/typed");
    wrong_target.handler.addr = fixture.callable("main.health").addr;
    let error = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, wrong_target.eval.clone()),
            request(&wrong_target.key, "POST", "/typed", br#""value""#),
            &wrong_target,
        )
        .await
        .expect_err("a different executable address must not satisfy the exact handler signature");
    assert!(error.to_string().contains("exact linked signature"));

    let mut wrong_signature = fixture.target_for_path("/typed");
    wrong_signature.handler.signature.parameters[0].name = "forged".to_string();
    let error = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, wrong_signature.eval.clone()),
            request(&wrong_signature.key, "POST", "/typed", br#""value""#),
            &wrong_signature,
        )
        .await
        .expect_err("a mutated callable signature must fail before execution");
    assert!(error.to_string().contains("exact linked signature"));

    let mut wrong_mode = fixture.target_for_path("/typed");
    let skiff_artifact_model::GatewayProtocolSurface::Http(http) = &mut wrong_mode.surface.protocol
    else {
        panic!("typed fixture must use an HTTP surface");
    };
    http.dispatch_mode = GatewayDispatchMode::ServerStream;
    let error = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, wrong_mode.eval.clone()),
            request(&wrong_mode.key, "POST", "/typed", br#""value""#),
            &wrong_mode,
        )
        .await
        .expect_err("typedJson serverStream mutation must fail closed");
    assert!(error.to_string().contains("unary dispatch"));

    let mut wrong_adapter = fixture.target_for_path("/typed");
    wrong_adapter.plan.kind = GatewayAdapterKind::RawHttp;
    let error = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, wrong_adapter.eval.clone()),
            request(&wrong_adapter.key, "POST", "/typed", br#""value""#),
            &wrong_adapter,
        )
        .await
        .expect_err("adapter plan/surface mutation must fail closed");
    assert!(error.to_string().contains("adapter kind"));
}

#[tokio::test]
async fn runtime_http_gateway_refuses_websocket_connect_surface_before_execution() {
    let fixture = fixture();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let mut target = fixture.target_for_path("/typed");
    target.surface.protocol =
        GatewayProtocolSurface::WebSocketConnect(GatewayWebSocketConnectProtocolSurface {
            connect_request_shape: GatewayWebSocketShapeVersion::V1,
            connect_result_shape: GatewayWebSocketShapeVersion::V1,
            connection_policy_shape: GatewayWebSocketShapeVersion::V1,
            external_sources: vec![
                skiff_artifact_model::GatewayAdapterSource::WebSocketConnectRequest,
                skiff_artifact_model::GatewayAdapterSource::WebSocketConnectionId,
            ],
            downlink_frames: vec![
                GatewayWebSocketDownlinkFrame::Binary,
                GatewayWebSocketDownlinkFrame::Text,
            ],
            rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
        });

    let error = interpreter
        .execute_runtime_http_gateway_unary(
            execution_context(&interpreter, target.eval.clone()),
            request(&target.key, "POST", "/typed", br#""value""#),
            &target,
        )
        .await
        .expect_err("HTTP execution must refuse a websocketConnect surface");
    assert!(error
        .to_string()
        .contains("requires an HTTP protocol surface"));
}

#[tokio::test]
async fn native_http_gateway_refuses_websocket_jsonrpc_only_sources_before_execution() {
    let fixture = fixture();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());

    for source in [
        skiff_artifact_model::GatewayAdapterSource::WebSocketJsonRpcParams,
        skiff_artifact_model::GatewayAdapterSource::WebSocketBusinessIdentity,
    ] {
        let mut target = fixture.target_for_path("/typed");
        target.plan.args[0].source = source;
        let error = interpreter
            .execute_runtime_http_gateway_unary(
                execution_context(&interpreter, target.eval.clone()),
                request(&target.key, "POST", "/typed", br#""value""#),
                &target,
            )
            .await
            .expect_err("HTTP execution must reject WebSocket JSON-RPC-only sources");
        assert!(
            error
                .to_string()
                .contains("WebSocket JSON-RPC-only adapter sources"),
            "{error}"
        );
    }
}

#[derive(Clone)]
struct TestCallable {
    id: PackageCallableId,
    signature: PackageCallableSignature,
    addr: ExecutableAddr,
}

#[derive(Clone)]
struct TestGatewayTarget {
    eval: RuntimeAssemblyEvalTarget,
    key: GatewayEntryKey,
    identity: GatewayEntryIdentity,
    surface: GatewayEntryProtocolSurface,
    plan: GatewayAdapterPlan,
    handler: TestCallable,
    pre: Option<TestCallable>,
    guard: Option<TestCallable>,
}

impl RuntimeHttpGatewayExecutionTarget for TestGatewayTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    fn gateway_entry_key(&self) -> &GatewayEntryKey {
        &self.key
    }

    fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        &self.identity
    }

    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        &self.surface
    }

    fn adapter_plan(&self) -> &GatewayAdapterPlan {
        &self.plan
    }

    fn handler(&self) -> RuntimeHttpGatewayCallable<'_> {
        callable_view(&self.handler)
    }

    fn pre(&self) -> Option<RuntimeHttpGatewayCallable<'_>> {
        self.pre.as_ref().map(callable_view)
    }

    fn guard(&self) -> Option<RuntimeHttpGatewayCallable<'_>> {
        self.guard.as_ref().map(callable_view)
    }
}

fn callable_view(callable: &TestCallable) -> RuntimeHttpGatewayCallable<'_> {
    RuntimeHttpGatewayCallable {
        callable_id: &callable.id,
        signature: &callable.signature,
        addr: &callable.addr,
    }
}

struct CompiledGatewayFixture {
    assembly: Arc<RuntimeAssembly>,
    deployment: Arc<ServiceDeployment>,
    implementation: Arc<PackageArtifact>,
    image: Arc<AssemblyExecutionImage>,
}

impl CompiledGatewayFixture {
    fn target_for_path(&self, path: &str) -> TestGatewayTarget {
        let binding = self
            .deployment
            .ingress
            .iter()
            .find(|binding| binding.selector.path == path)
            .unwrap_or_else(|| panic!("missing fixture gateway path {path}"));
        let entry = self
            .deployment
            .gateway_entries
            .get(&binding.gateway_entry_key)
            .expect("fixture gateway entry");
        let eval = self.eval_target();
        TestGatewayTarget {
            eval,
            key: binding.gateway_entry_key.clone(),
            identity: entry.gateway_entry_identity.clone(),
            surface: entry.protocol_surface.clone(),
            plan: entry.adapter_plan.clone(),
            handler: self.callable(
                entry
                    .handler
                    .as_ref()
                    .expect("HTTP gateway fixture entry requires a handler")
                    .as_str(),
            ),
            pre: entry.pre.as_ref().map(|id| self.callable(id.as_str())),
            guard: entry.guard.as_ref().map(|id| self.callable(id.as_str())),
        }
    }

    fn callable(&self, selector_or_id: &str) -> TestCallable {
        let (id, signature) = self
            .implementation
            .package_local_abi
            .implementation_symbols
            .iter()
            .find_map(|(selector, symbol)| match symbol {
                PackageLocalAbiSymbol::Callable {
                    callable_id,
                    signature,
                } if selector == selector_or_id || callable_id.as_str() == selector_or_id => {
                    Some((callable_id.clone(), signature.clone()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing fixture callable {selector_or_id}"));
        let target = self
            .implementation
            .callable_links
            .get(&id)
            .map(|fact| &fact.target)
            .expect("fixture callable target");
        let addr = self
            .image
            .entry_executable(&self.implementation.package_build_id, target)
            .expect("fixture executable address")
            .addr()
            .clone();
        TestCallable {
            id,
            signature,
            addr,
        }
    }

    fn eval_target(&self) -> RuntimeAssemblyEvalTarget {
        let activation_template = self
            .assembly
            .activation_templates
            .iter()
            .find(|template| template.deployment == service_deployment_ref(&self.deployment))
            .expect("fixture activation template");
        let binding_template = self
            .assembly
            .service_binding_templates
            .iter()
            .find(|template| template.activation == activation_template.deployment)
            .expect("fixture service binding template");
        let activation = ActivationContext::from_assembly_templates(
            self.assembly.assembly_identity.clone(),
            1,
            "runtime-http-gateway-test",
            activation_template,
            binding_template,
        )
        .expect("fixture activation context");
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(FixtureEvalResolver {
            activation: Arc::clone(&activation),
        });
        let request =
            RequestActivationContext::begin(activation).expect("fixture request activation");
        RuntimeAssemblyEvalTarget::new(Arc::clone(&self.image), request, resolver)
            .expect("fixture eval target")
    }
}

struct FixtureEvalResolver {
    activation: Arc<ActivationContext>,
}

impl RuntimeAssemblyEvalResolver for FixtureEvalResolver {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id() == activation_id).then(|| Arc::clone(&self.activation))
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id().as_str() == activation_id)
            .then(|| Arc::clone(&self.activation))
    }

    fn contract(&self, _contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        None
    }

    fn admitted_schema_records(
        &self,
        _contract: &ServiceContractRef,
    ) -> Option<AdmittedPackageSchemaRecords> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &skiff_artifact_model::ContractOperationId,
    ) -> Option<OperationTargetRef> {
        None
    }
}

fn fixture() -> Arc<CompiledGatewayFixture> {
    Arc::clone(FIXTURE.get_or_init(|| Arc::new(compile_fixture())))
}

fn compile_fixture() -> CompiledGatewayFixture {
    let temp = TempFixture::new("runtime-http-gateway");
    let service_root = temp.child("service");
    let artifact_root = temp.child("artifacts");
    write_service_fixture(&service_root);
    let platform = repository_platform_sources();
    seed_canonical_std(&platform, &artifact_root).expect("canonical std seed");
    let output = build_authoring_object(
        &platform,
        AuthoringObject::Package,
        &service_root,
        &artifact_root,
        "dev",
        true,
    )
    .expect("gateway service authoring");
    let root_package_ref =
        serde_json::from_value(output["packageArtifactReceipt"]["artifact"].clone())
            .expect("gateway package ref");
    let deployment_ref: ServiceDeploymentRef =
        serde_json::from_value(output["serviceDeploymentReceipt"]["deployment"].clone())
            .expect("gateway deployment ref");
    let contract_ref: ServiceContractRef =
        serde_json::from_value(output["serviceContractReceipt"]["contract"].clone())
            .expect("gateway contract ref");
    let store = CanonicalArtifactStore::open(&artifact_root).expect("gateway artifact store");
    let deployment = store
        .read_service_deployment(&deployment_ref)
        .expect("gateway deployment");
    let contract = store
        .read_service_contract(&contract_ref)
        .expect("gateway contract");
    let implementation = store
        .read_package_artifact(&root_package_ref)
        .expect("gateway implementation");
    let mut package_refs =
        BTreeMap::from([(implementation.package_build_id.clone(), root_package_ref)]);
    for binding in &deployment.package_bindings {
        package_refs.insert(
            binding.package.package_build_id.clone(),
            binding.package.clone(),
        );
    }
    let packages = package_refs
        .values()
        .map(|reference| store.read_package_artifact(reference))
        .collect::<Result<Vec<_>, _>>()
        .expect("gateway package closure");
    let package_values = packages
        .iter()
        .map(|package| package.as_ref().clone())
        .collect::<Vec<_>>();
    let root = service_deployment_ref(&deployment);
    let assembly = Arc::new(
        resolve_runtime_assembly(
            std::slice::from_ref(&root),
            std::slice::from_ref(deployment.as_ref()),
            std::slice::from_ref(contract.as_ref()),
            &package_values,
        )
        .expect("gateway runtime assembly"),
    );
    let hydrated = assembly
        .package_link_plan
        .code_slots
        .iter()
        .map(|slot| hydrate_package(&store, &slot.package))
        .collect::<Vec<_>>();
    let image =
        skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, hydrated)
            .expect("gateway execution image");
    CompiledGatewayFixture {
        assembly,
        deployment,
        implementation,
        image,
    }
}

fn hydrate_package(
    store: &CanonicalArtifactStore,
    reference: &skiff_artifact_model::PackageArtifactRef,
) -> HydratedPackageCode {
    let artifact = store
        .read_package_artifact(reference)
        .expect("fixture package artifact");
    let files = artifact
        .files
        .iter()
        .map(|file| store.read_file_ir(reference, file))
        .collect::<Result<Vec<_>, _>>()
        .expect("fixture File IR closure");
    // These execution tests do not exercise public service-error projection. Keep the canonical
    // schema-index identity pin while omitting that independent closure, as the existing eval
    // package fixture helper does.
    let schema_index = Arc::new(PackageSchemaIndex {
        package_id: artifact.package_schema_index.package_id.clone(),
        package_schema_index_identity: artifact
            .package_schema_index
            .package_schema_index_identity
            .clone(),
        types: BTreeMap::new(),
    });
    HydratedPackageCode::new(artifact, files, PublicationResourceTable::default())
        .with_schema_index(schema_index)
}

fn service_deployment_ref(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn execution_context<'a>(
    interpreter: &Interpreter,
    target: RuntimeAssemblyEvalTarget,
) -> ProgramExecutionContext<'a> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    let request = test_runtime::request_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(
            interpreter.stream_runtime.clone(),
        ),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            interpreter.stream_runtime.clone(),
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        actor: actor.clone(),
        request,
        request_heap_limits: RequestHeapLimits::default(),
    })
    .with_runtime_assembly_target(target)
}

fn request<'a>(
    key: &'a GatewayEntryKey,
    method: &'a str,
    path: &'a str,
    body: &'a [u8],
) -> RequestPayloadContext<'a> {
    RequestPayloadContext::new(
        key.as_str(),
        body,
        Some(BinaryHttpRequestContext::new(
            method,
            path,
            path,
            Vec::new(),
            Vec::new(),
            body,
        )),
    )
}

fn write_service_fixture(root: &Path) {
    fs::create_dir_all(root).expect("gateway fixture directory");
    fs::write(
        root.join("package.yml"),
        format!("id: {PACKAGE_ID}\nversion: {VERSION}\n"),
    )
    .expect("gateway package manifest");
    fs::write(root.join("api.yml"), "health: main.health\n").expect("gateway API");
    fs::write(root.join("service.yml"), format!("id: {SERVICE_ID}\n"))
        .expect("gateway service manifest");
    fs::write(
        root.join("http.yml"),
        format!(
            r#"typed:
  method: POST
  path: /typed
  kind: typedJson
  handler: main.typed
  guard: main.allow
  pre: main.prepare
  adapterArgs:
    - param: body
      source: {{ kind: http.body }}
    - param: context
      source: {{ kind: http.context }}
blocked:
  method: POST
  path: /blocked
  kind: typedJson
  handler: main.typed
  guard: main.block
  pre: main.prepare
  adapterArgs:
    - param: body
      source: {{ kind: http.body }}
    - param: context
      source: {{ kind: http.context }}
raw:
  method: POST
  path: /raw
  kind: rawHttp
  handler: main.raw
  adapterArgs:
    - param: request
      source: {{ kind: http.request }}
stream:
  method: POST
  path: /stream
  kind: rawHttp
  handler: main.stream
  adapterArgs:
    - param: request
      source: {{ kind: http.request }}
"#
        ),
    )
    .expect("gateway HTTP manifest");
    fs::write(
        root.join("config.dev.yml"),
        "timeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:runtime-http-gateway\n",
    )
    .expect("gateway config");
    fs::write(
        root.join("main.skiff"),
        r#"import std

function health() -> string {
  return "healthy"
}

function allow(request: std.http.HttpRequest) -> std.http.HttpResponse? {
  return null
}

function block(request: std.http.HttpRequest) -> std.http.HttpResponse? {
  return std.http.noContent()
}

function prepare(request: std.http.HttpRequest) -> string {
  return request.path
}

function typed(body: string, context: string) -> string {
  return context
}

function raw(request: std.http.HttpRequest) -> std.http.HttpResponse {
  return std.http.HttpResponse {
    status: 201,
    headers: Array.empty<std.http.HttpHeader>(),
    body: request.body,
  }
}

function stream(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {
  emit(std.http.streamStart(202, Array.empty<std.http.HttpHeader>()))
  emit(std.http.streamChunk(request.body))
  emit(std.http.streamEnd())
  return null
}
"#,
    )
    .expect("gateway source");
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval must live below the Skiff root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("repository platform sources")
}

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-runtime-eval-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("gateway temp fixture root");
        Self { root }
    }

    fn child(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
