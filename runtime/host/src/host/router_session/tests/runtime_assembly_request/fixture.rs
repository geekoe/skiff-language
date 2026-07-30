use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_model::{
    AssemblyActivationServiceDb, PackageArtifactRef, RuntimeAssembly, ServiceContractRef,
    ServiceDeploymentRef, ServiceIngressKey,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::{assembly::resolve_runtime_assembly, storage::CanonicalArtifactStore};
use skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver;
use skiff_test_runner::canonical_std_seed::seed_canonical_std;
use skiff_test_runner::package_service_host_fixture::prepare_package_service_host_fixture;
use skiff_test_runner::{
    canonical_package::compile_package_project_for_test,
    canonical_store::CanonicalBaseAssembly,
    test_discovery::discover_test_service_cases,
    test_service_fixture::{assemble_test_service_fixture_for_run, CanonicalTestServiceEntrypoint},
};

use crate::{host::RuntimeHost, loader::assembly_admission::ActiveAssemblyRoute};

const PACKAGE_ID: &str = "example.com/host-http-gateway";
const SERVICE_ID: &str = "example.com/host-http-gateway-service";
const VERSION: &str = "1.0.0";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FIXTURE: OnceLock<CompiledGatewayFixture> = OnceLock::new();
static PATH_ONLY_FIXTURE: OnceLock<CompiledGatewayFixture> = OnceLock::new();
static PINNED_ROUTE_FIXTURE_A: OnceLock<CompiledGatewayFixture> = OnceLock::new();
static PINNED_ROUTE_FIXTURE_B: OnceLock<CompiledGatewayFixture> = OnceLock::new();
static PACKAGE_DIRECT_STREAM_FIXTURE: OnceLock<CurrentScopeCompiledFixture> = OnceLock::new();
static STREAM_ARGUMENT_FIXTURE: OnceLock<StreamArgumentCompiledFixture> = OnceLock::new();
static SPAWN_SUBMIT_FIXTURE: OnceLock<CurrentScopeCompiledFixture> = OnceLock::new();

pub(super) async fn admitted_gateway_host() -> (RuntimeHost, HashMap<String, ActiveAssemblyRoute>) {
    let fixture = fixture();
    let resolver = fixture.resolver();
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("compiled HTTP gateway assembly should admit");
    let routes = fixture
        .assembly
        .gateway_ingress
        .iter()
        .map(|binding| {
            (
                binding.selector.path.clone(),
                host.lookup_active_assembly_request_route(&binding.service_ingress_key())
                    .expect("compiled HTTP gateway route"),
            )
        })
        .collect();
    (host, routes)
}

pub(super) async fn admitted_current_scope_gateway_host(
) -> (RuntimeHost, HashMap<String, ActiveAssemblyRoute>) {
    let fixture = compile_current_scope_fixture();
    let resolver = FilesystemRuntimeAssemblyContentResolver::open(&fixture.artifact_root)
        .expect("current-scope filesystem resolver");
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("exact current-scope source assembly should admit");
    let routes = fixture
        .assembly
        .gateway_ingress
        .iter()
        .map(|binding| {
            (
                binding.selector.path.clone(),
                host.lookup_active_assembly_request_route(&binding.service_ingress_key())
                    .expect("exact current-scope gateway route"),
            )
        })
        .collect();
    (host, routes)
}

pub(super) async fn admitted_package_direct_stream_gateway_host(
) -> (RuntimeHost, HashMap<String, ActiveAssemblyRoute>) {
    let fixture = PACKAGE_DIRECT_STREAM_FIXTURE.get_or_init(|| {
        compile_package_service_fixture(
            "host-package-direct-http-stream-registry",
            "test-runner/fixtures/package-direct-http-stream-registry",
        )
    });
    let resolver = FilesystemRuntimeAssemblyContentResolver::open(&fixture.artifact_root)
        .expect("package-direct HTTP stream filesystem resolver");
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("package-direct HTTP stream assembly should admit");
    let routes = fixture
        .assembly
        .gateway_ingress
        .iter()
        .map(|binding| {
            (
                binding.selector.path.clone(),
                host.lookup_active_assembly_request_route(&binding.service_ingress_key())
                    .expect("package-direct HTTP stream gateway route"),
            )
        })
        .collect();
    (host, routes)
}

pub(super) async fn admitted_spawn_submit_host() -> (RuntimeHost, ActiveAssemblyRoute) {
    let fixture = SPAWN_SUBMIT_FIXTURE.get_or_init(compile_spawn_submit_fixture_with_stack);
    let resolver = FilesystemRuntimeAssemblyContentResolver::open(&fixture.artifact_root)
        .expect("spawn-submit filesystem resolver");
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("spawn-submit assembly should admit");
    let binding = fixture
        .assembly
        .gateway_ingress
        .iter()
        .find(|binding| binding.selector.path == "/probe")
        .expect("spawn-submit probe ingress");
    let route = host
        .lookup_active_assembly_request_route(&binding.service_ingress_key())
        .expect("spawn-submit probe route");
    (host, route)
}

fn compile_spawn_submit_fixture() -> CurrentScopeCompiledFixture {
    let temp = TempFixture::new("host-direct-spawn-submit");
    let source_artifacts = temp.child("source-artifacts");
    let runtime_artifacts = temp.child("runtime-artifacts");
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host must live below the Skiff root")
        .to_path_buf();
    let fixture_root = repository.join("test-runner/fixtures/package-service-i02-spawn-submit");
    let platform = CompilerPlatformSources::new(&repository).expect("repository platform sources");
    seed_canonical_std(&platform, &source_artifacts).expect("canonical std seed");
    let project = compile_package_project_for_test(&platform, &fixture_root, &source_artifacts)
        .expect("spawn-submit test service production package");
    let cases = discover_test_service_cases(&fixture_root, &fixture_root, false)
        .expect("spawn-submit test discovery");
    assert_eq!(cases.len(), 1);
    let test_fixture = assemble_test_service_fixture_for_run(
        &project,
        &cases,
        CanonicalBaseAssembly::default(),
        "host-direct-spawn-submit",
    )
    .expect("spawn-submit test-service assembly");
    test_fixture
        .publish(&source_artifacts, &runtime_artifacts)
        .expect("spawn-submit runtime records");
    let case = test_fixture
        .cases
        .into_iter()
        .next()
        .expect("spawn-submit fixture has one case");
    CurrentScopeCompiledFixture {
        assembly: Arc::new(case.records.assembly),
        artifact_root: runtime_artifacts,
        _temp: temp,
    }
}

fn compile_spawn_submit_fixture_with_stack() -> CurrentScopeCompiledFixture {
    std::thread::Builder::new()
        .name("host-direct-spawn-submit-fixture".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(compile_spawn_submit_fixture)
        .expect("spawn-submit fixture compiler thread")
        .join()
        .expect("spawn-submit fixture compiler thread should not panic")
}

pub(super) async fn admitted_stream_argument_gateway_host(
) -> (RuntimeHost, HashMap<String, ActiveAssemblyRoute>) {
    let fixture = STREAM_ARGUMENT_FIXTURE.get_or_init(compile_stream_argument_fixture_with_stack);
    let resolver = FilesystemRuntimeAssemblyContentResolver::open(&fixture.artifact_root)
        .expect("stream-argument HTTP stream filesystem resolver");
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("stream-argument HTTP stream assembly should admit");
    let route_specs = [
        ("normal", "/package-direct/stream-argument/normal"),
        (
            "producer-error",
            "/package-direct/stream-argument/producer-error",
        ),
        (
            "consumer-cancel",
            "/package-direct/stream-argument/consumer-cancel",
        ),
        (
            "response-sink-normal",
            "/package-direct/response-sink/normal",
        ),
        (
            "response-sink-producer-error",
            "/package-direct/response-sink/producer-error",
        ),
        (
            "response-sink-consumer-cancel",
            "/package-direct/response-sink/consumer-cancel",
        ),
    ];
    let routes = route_specs
        .into_iter()
        .map(|(role, path)| {
            let binding = fixture
                .assembly
                .gateway_ingress
                .iter()
                .find(|binding| {
                    binding.deployment == fixture.entrypoint.deployment
                        && binding.selector.path == path
                })
                .expect("stream-argument HTTP ingress binding");
            (
                role.to_string(),
                host.lookup_active_assembly_request_route(&binding.service_ingress_key())
                    .expect("stream-argument HTTP gateway route"),
            )
        })
        .collect();
    (host, routes)
}

pub(crate) async fn reloaded_gateway_host(
) -> (RuntimeHost, ActiveAssemblyRoute, ActiveAssemblyRoute) {
    let fixture = fixture();
    let resolver = fixture.resolver();
    let key = fixture
        .assembly
        .gateway_ingress
        .iter()
        .find(|binding| binding.selector.path == "/typed")
        .expect("typed gateway selector")
        .service_ingress_key();
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("gateway generation one should admit");
    let pinned = host
        .lookup_active_assembly_request_route(&key)
        .expect("generation one route");
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("gateway generation two should admit");
    let current = host
        .lookup_active_assembly_request_route(&key)
        .expect("generation two route");
    (host, pinned, current)
}

pub(crate) async fn admitted_websocket_gateway_host(
) -> (RuntimeHost, ActiveAssemblyRoute, ActiveAssemblyRoute) {
    let fixture = fixture();
    let resolver = fixture.resolver();
    let (physical_key, method_key) = websocket_ingress_keys(&fixture.assembly);
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("compiled WebSocket gateway assembly should admit");
    let physical = host
        .lookup_active_assembly_request_route(&physical_key)
        .expect("physical WebSocket route");
    let method = host
        .lookup_active_assembly_request_route(&method_key)
        .expect("WebSocket JSON-RPC method route");
    (host, physical, method)
}

pub(crate) async fn admitted_path_only_websocket_gateway_host() -> (RuntimeHost, ActiveAssemblyRoute)
{
    let fixture = path_only_fixture();
    let resolver = fixture.resolver();
    let physical_key = fixture
        .assembly
        .gateway_ingress
        .iter()
        .find(|binding| binding.selector.path == "/socket" && binding.selector.method.is_none())
        .expect("path-only physical WebSocket selector")
        .service_ingress_key();
    let host = super::super::test_host();
    host.assembly_admission
        .admit(Arc::clone(&fixture.assembly), &resolver)
        .await
        .expect("path-only WebSocket gateway assembly should admit");
    let physical = host
        .lookup_active_assembly_request_route(&physical_key)
        .expect("path-only physical WebSocket route");
    (host, physical)
}

pub(crate) struct ReloadedWebSocketGatewayHost {
    pub(crate) host: RuntimeHost,
    pub(crate) physical_a: ActiveAssemblyRoute,
    pub(crate) method_a: ActiveAssemblyRoute,
    pub(crate) methods_a: HashMap<String, ActiveAssemblyRoute>,
    pub(crate) physical_b: ActiveAssemblyRoute,
    pub(crate) method_b: ActiveAssemblyRoute,
}

pub(crate) async fn reloaded_websocket_gateway_host() -> ReloadedWebSocketGatewayHost {
    let fixture_a = pinned_route_fixture(false);
    let fixture_b = pinned_route_fixture(true);
    let resolver_a = fixture_a.resolver();
    let resolver_b = fixture_b.resolver();
    let (physical_key_a, method_key_a) = websocket_ingress_keys(&fixture_a.assembly);
    let (physical_key_b, method_key_b) = websocket_ingress_keys(&fixture_b.assembly);
    assert_eq!(physical_key_a.selector, physical_key_b.selector);
    assert_eq!(method_key_a.selector, method_key_b.selector);
    let host = pinned_route_test_host();
    let service_db = AssemblyActivationServiceDb {
        mongo_url: "mongodb://pinned-route.invalid".to_string(),
    };
    let assembly_a = skiff_artifact_identity::runtime_assembly_ref(&fixture_a.assembly).unwrap();
    let assembly_b = skiff_artifact_identity::runtime_assembly_ref(&fixture_b.assembly).unwrap();
    host.assembly_admission
        .recover_committed(
            "pinned-route",
            1,
            &assembly_a,
            &resolver_a,
            Some(&service_db),
        )
        .await
        .expect("WebSocket generation one should admit");
    let physical_a = host
        .lookup_active_assembly_request_route(&physical_key_a)
        .expect("generation one physical WebSocket route");
    let method_a = host
        .lookup_active_assembly_request_route(&method_key_a)
        .expect("generation one WebSocket method route");
    let methods_a = websocket_method_routes(&host, &fixture_a.assembly);
    host.assembly_admission
        .recover_committed(
            "pinned-route",
            2,
            &assembly_b,
            &resolver_b,
            Some(&service_db),
        )
        .await
        .expect("WebSocket generation two should admit");
    let physical_b = host
        .lookup_active_assembly_request_route(&physical_key_b)
        .expect("generation two physical WebSocket route");
    let method_b = host
        .lookup_active_assembly_request_route(&method_key_b)
        .expect("generation two WebSocket method route");
    ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        method_a,
        methods_a,
        physical_b,
        method_b,
    }
}

#[derive(Clone, Default)]
struct PinnedRouteDbProvider {
    next_source: Arc<AtomicU64>,
}

impl skiff_runtime_capability_context::DbProviderFactory for PinnedRouteDbProvider {
    fn build(
        &self,
        _input: skiff_runtime_capability_context::DbProviderBuildInput,
    ) -> skiff_runtime_capability_context::DbCapabilityResult<
        skiff_runtime_capability_context::DbCapabilitySource,
    > {
        let sequence = self.next_source.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(skiff_runtime_capability_context::DbCapabilitySource::new(
            Some(PinnedRouteDbFactory {
                marker: format!("pinned-route-db-source-{sequence}"),
            }),
        ))
    }
}

#[derive(Clone)]
struct PinnedRouteDbFactory {
    marker: String,
}

impl skiff_runtime_capability_context::DbCapabilityFactory for PinnedRouteDbFactory {
    fn context_for_request(
        &self,
        _owner: String,
        _request_id: String,
    ) -> skiff_runtime_capability_context::DbCapabilityContext {
        skiff_runtime_capability_context::DbCapabilityContext::new(PinnedRouteDbContext {
            marker: self.marker.clone(),
        })
    }
}

struct PinnedRouteDbContext {
    marker: String,
}

impl skiff_runtime_capability_context::DbCapabilityContextApi for PinnedRouteDbContext {
    fn require_store(
        &self,
        _target: &str,
        _unavailable_reason: &str,
    ) -> skiff_runtime_capability_context::DbCapabilityResult<
        skiff_runtime_capability_context::DbCapabilityStore,
    > {
        Err(skiff_runtime_capability_context::DbCapabilityError::decode(
            self.marker.clone(),
        ))
    }
}

fn pinned_route_test_host() -> RuntimeHost {
    RuntimeHost::new(crate::host::RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::new(
            PinnedRouteDbProvider::default(),
        ),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-pinned-route".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-pinned-route-test-home"),
        environment: "test".to_string(),
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("pinned route runtime host should build")
}

fn websocket_ingress_keys(assembly: &RuntimeAssembly) -> (ServiceIngressKey, ServiceIngressKey) {
    let physical = assembly
        .gateway_ingress
        .iter()
        .find(|binding| binding.selector.path == "/socket" && binding.selector.method.is_none())
        .expect("physical WebSocket selector")
        .service_ingress_key();
    let method = assembly
        .gateway_ingress
        .iter()
        .find(|binding| {
            binding.selector.path == "/socket"
                && binding.selector.method.as_deref() == Some("status.get")
        })
        .expect("WebSocket JSON-RPC method selector")
        .service_ingress_key();
    (physical, method)
}

fn websocket_method_routes(
    host: &RuntimeHost,
    assembly: &RuntimeAssembly,
) -> HashMap<String, ActiveAssemblyRoute> {
    assembly
        .gateway_ingress
        .iter()
        .filter_map(|binding| {
            let method = binding.selector.method.clone()?;
            let route = host
                .lookup_active_assembly_request_route(&binding.service_ingress_key())
                .expect("compiled WebSocket JSON-RPC route");
            Some((method, route))
        })
        .collect()
}

struct CompiledGatewayFixture {
    assembly: Arc<RuntimeAssembly>,
    artifact_root: PathBuf,
    _temp: TempFixture,
}

struct CurrentScopeCompiledFixture {
    assembly: Arc<RuntimeAssembly>,
    artifact_root: PathBuf,
    _temp: TempFixture,
}

struct StreamArgumentCompiledFixture {
    assembly: Arc<RuntimeAssembly>,
    artifact_root: PathBuf,
    entrypoint: CanonicalTestServiceEntrypoint,
    _temp: TempFixture,
}

impl CompiledGatewayFixture {
    fn resolver(&self) -> FilesystemRuntimeAssemblyContentResolver {
        FilesystemRuntimeAssemblyContentResolver::open(&self.artifact_root)
            .expect("gateway filesystem resolver")
    }
}

fn fixture() -> &'static CompiledGatewayFixture {
    FIXTURE.get_or_init(|| compile_fixture(true))
}

fn compile_current_scope_fixture() -> CurrentScopeCompiledFixture {
    let temp = TempFixture::new("host-current-scope-source-artifact");
    let artifact_root = temp.child("artifacts");
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host must live below the Skiff root")
        .to_path_buf();
    let platform = CompilerPlatformSources::new(&repository).expect("repository platform sources");
    seed_canonical_std(&platform, &artifact_root).expect("canonical std seed");
    let receipt = prepare_package_service_host_fixture(
        &platform,
        &repository.join("test-runner/fixtures/package-service-current-scope"),
        &temp.child("authoring"),
        &artifact_root,
        "current-scope",
    )
    .expect("exact current-scope fixture authoring");
    let store = CanonicalArtifactStore::open(&artifact_root).expect("current-scope artifact store");
    let assembly = store
        .read_runtime_assembly(&receipt.base_assembly)
        .expect("exact current-scope RuntimeAssembly");
    assert_eq!(
        receipt.base_assembly.assembly_identity.as_str(),
        "skiff-runtime-assembly-v3:sha256:a06e9806093074f986212d0feb1646be6a77ba69fb0fb42ae9067924e2d6b9ee"
    );
    assert_eq!(
        receipt.consumer_package.package_build_id.as_str(),
        "skiff-package-build-v10:sha256:aae3bc279027081667992b0881772f5fcee397c7443156403a5c1651c6f57c54"
    );
    assert_eq!(
        receipt
            .consumer_deployment
            .deployment_artifact_identity
            .as_str(),
        "skiff-deployment-artifact-v4:sha256:5507d8173d99e3bfd6fbf4e6c6a82be178ffcad59ed0fde27475f87d5bf99b02"
    );
    CurrentScopeCompiledFixture {
        assembly,
        artifact_root,
        _temp: temp,
    }
}

fn compile_package_service_fixture(
    temp_name: &str,
    repository_relative_fixture: &str,
) -> CurrentScopeCompiledFixture {
    let temp = TempFixture::new(temp_name);
    let artifact_root = temp.child("artifacts");
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host must live below the Skiff root")
        .to_path_buf();
    let platform = CompilerPlatformSources::new(&repository).expect("repository platform sources");
    seed_canonical_std(&platform, &artifact_root).expect("canonical std seed");
    let receipt = prepare_package_service_host_fixture(
        &platform,
        &repository.join(repository_relative_fixture),
        &temp.child("authoring"),
        &artifact_root,
        "current-scope",
    )
    .expect("package-service HTTP stream fixture authoring");
    let store =
        CanonicalArtifactStore::open(&artifact_root).expect("HTTP stream fixture artifact store");
    let assembly = store
        .read_runtime_assembly(&receipt.base_assembly)
        .expect("HTTP stream fixture RuntimeAssembly");
    CurrentScopeCompiledFixture {
        assembly,
        artifact_root,
        _temp: temp,
    }
}

fn compile_stream_argument_fixture() -> StreamArgumentCompiledFixture {
    let temp = TempFixture::new("host-package-direct-stream-argument");
    let source_artifacts = temp.child("source-artifacts");
    let runtime_artifacts = temp.child("runtime-artifacts");
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host must live below the Skiff root")
        .to_path_buf();
    let fixture_root = repository.join("test-runner/fixtures/package-direct-http-stream-registry");
    let platform = CompilerPlatformSources::new(&repository).expect("repository platform sources");
    seed_canonical_std(&platform, &source_artifacts).expect("canonical std seed");
    build_authoring_object(
        &platform,
        AuthoringObject::Package,
        &fixture_root.join("argument-provider"),
        &source_artifacts,
        "skiff-test",
        true,
    )
    .expect("stream-argument provider publication");
    let test_service = fixture_root.join("argument-tests");
    let project = compile_package_project_for_test(&platform, &test_service, &source_artifacts)
        .expect("stream-argument test service production package");
    let cases = discover_test_service_cases(&test_service, &test_service, false)
        .expect("stream-argument test discovery");
    assert_eq!(cases.len(), 6);
    let test_fixture = assemble_test_service_fixture_for_run(
        &project,
        &cases,
        CanonicalBaseAssembly::default(),
        "p8-s2-stream-argument",
    )
    .expect("stream-argument test-service assemblies");
    test_fixture
        .publish(&source_artifacts, &runtime_artifacts)
        .expect("stream-argument runtime records");
    let first_case = test_fixture
        .cases
        .into_iter()
        .next()
        .expect("stream-argument fixture has six cases");
    StreamArgumentCompiledFixture {
        assembly: Arc::new(first_case.records.assembly),
        artifact_root: runtime_artifacts,
        entrypoint: first_case.entrypoint,
        _temp: temp,
    }
}

fn compile_stream_argument_fixture_with_stack() -> StreamArgumentCompiledFixture {
    std::thread::Builder::new()
        .name("p8-s2-stream-argument-fixture".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(compile_stream_argument_fixture)
        .expect("stream-argument fixture compiler thread")
        .join()
        .expect("stream-argument fixture compiler thread should not panic")
}

fn path_only_fixture() -> &'static CompiledGatewayFixture {
    PATH_ONLY_FIXTURE.get_or_init(|| compile_fixture(false))
}

fn pinned_route_fixture(replacement: bool) -> &'static CompiledGatewayFixture {
    if replacement {
        PINNED_ROUTE_FIXTURE_B.get_or_init(|| compile_pinned_route_fixture(true))
    } else {
        PINNED_ROUTE_FIXTURE_A.get_or_init(|| compile_pinned_route_fixture(false))
    }
}

fn compile_fixture(with_jsonrpc_method: bool) -> CompiledGatewayFixture {
    compile_fixture_variant(with_jsonrpc_method, false, false)
}

fn compile_pinned_route_fixture(replacement: bool) -> CompiledGatewayFixture {
    compile_fixture_variant(true, true, replacement)
}

fn compile_fixture_variant(
    with_jsonrpc_method: bool,
    with_database: bool,
    replacement: bool,
) -> CompiledGatewayFixture {
    let temp = TempFixture::new(if with_jsonrpc_method {
        if with_database {
            "host-pinned-websocket-route"
        } else {
            "host-http-websocket-gateway"
        }
    } else {
        "host-http-path-only-websocket-gateway"
    });
    let service_root = temp.child("service");
    let artifact_root = temp.child("artifacts");
    write_service_fixture(
        &service_root,
        with_jsonrpc_method,
        with_database,
        replacement,
    );
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
    .expect("Host gateway service authoring");
    let root_package_ref: PackageArtifactRef =
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
    let root = deployment_ref.clone();
    let assembly = resolve_runtime_assembly(
        std::slice::from_ref(&root),
        std::slice::from_ref(deployment.as_ref()),
        std::slice::from_ref(contract.as_ref()),
        &package_values,
    )
    .expect("gateway RuntimeAssembly");
    store
        .write_runtime_assembly(&assembly)
        .expect("gateway RuntimeAssembly record");
    let assembly = Arc::new(assembly);
    CompiledGatewayFixture {
        assembly,
        artifact_root,
        _temp: temp,
    }
}

fn write_service_fixture(
    root: &Path,
    with_jsonrpc_method: bool,
    with_database: bool,
    replacement: bool,
) {
    fs::create_dir_all(root).expect("gateway fixture directory");
    let state = if with_database {
        "state:\n  database:\n    kind: database\n"
    } else {
        ""
    };
    fs::write(
        root.join("package.yml"),
        format!("id: {PACKAGE_ID}\nversion: {VERSION}\n{state}"),
    )
    .expect("gateway package manifest");
    let api = if replacement {
        "health: main.health\nversion: main.version\n"
    } else {
        "health: main.health\n"
    };
    fs::write(root.join("api.yml"), api).expect("gateway API");
    let service_id = if replacement {
        "example.com/host-http-gateway-service-replacement"
    } else {
        SERVICE_ID
    };
    fs::write(root.join("service.yml"), format!("id: {service_id}\n"))
        .expect("gateway service manifest");
    fs::write(
        root.join("http.yml"),
        format!(
            r#"typed:
  method: POST
  path: /typed
  kind: typedJson
  handler: main.typed
  adapterArgs:
    - param: body
      source: {{ kind: http.body }}
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
slow:
  method: POST
  path: /slow
  kind: typedJson
  handler: main.slow
  adapterArgs:
    - param: body
      source: {{ kind: http.body }}
"#
        ),
    )
    .expect("gateway HTTP manifest");
    let websocket = if with_jsonrpc_method {
        r#"path: /socket
jsonRpc:
  status:
    method: status.get
    handler: main.websocketStatus
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  record:
    method: result.record
    handler: main.websocketRecord
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  array:
    method: params.array
    handler: main.websocketArray
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  void:
    method: result.void
    handler: main.websocketVoid
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  expectedFailure:
    method: result.expectedFailure
    handler: main.websocketExpectedFailure
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  throws:
    method: result.throw
    handler: main.websocketThrow
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  rpcSlow:
    method: result.slow
    handler: main.websocketSlow
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
  identity:
    method: identity.read
    handler: main.websocketIdentity
    adapterArgs:
      - param: params
        source: { kind: websocket.jsonRpcParams }
      - param: connectionId
        source: { kind: websocket.connectionId }
      - param: businessIdentity
        source: { kind: websocket.businessIdentity }
"#
    } else {
        "path: /socket\n"
    };
    fs::write(root.join("websocket.yml"), websocket).expect("gateway WebSocket manifest");
    let config = match (with_database, replacement) {
        (true, true) => {
            "state:\n  database:\n    kind: database\n    namespace: pinned-route-b\ntimeout: 2200\nquota: { cpuMillis: 200, memoryBytes: 2097152 }\nprincipal: service:pinned-route-b\n"
        }
        (true, false) => {
            "state:\n  database:\n    kind: database\n    namespace: pinned-route-a\ntimeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:host-http-gateway\n"
        }
        (false, _) => {
            "timeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:host-http-gateway\n"
        }
    };
    fs::write(root.join("config.dev.yml"), config).expect("gateway config");
    let database_source = if with_database {
        "\ntype PinnedRouteRecord { id: string }\ndb object PinnedRouteRecord { primary key(id) }\n"
    } else {
        ""
    };
    let replacement_source = if replacement {
        "\nfunction version() -> string {\n  return \"replacement\"\n}\n"
    } else {
        ""
    };
    let websocket_generation_source = if replacement {
        "\nfunction websocketGenerationResult() -> string {\n  return \"new\"\n}\n"
    } else {
        "\nfunction websocketGenerationResult() -> string {\n  return \"old\"\n}\n"
    };
    fs::write(
        root.join("main.skiff"),
        format!(
            "{}{}{}{}",
            r#"import std

function health() -> string {
  return "healthy"
}

type WebSocketStatusParams { value: string }
type WebSocketRecordResult { value: string, accepted: boolean }
type WebSocketIdentityParams { connectionId: string, businessIdentity: string }
type WebSocketIdentityResult {
  connectionId: string,
  businessIdentity: string?,
  peerConnectionId: string,
  peerBusinessIdentity: string,
}
type WebSocketResultUnion discriminator "tag" =
  { tag: "ok", value: string }
  | { tag: "expectedFailure", reason: string }
type WebSocketPrivateFailure { message: string }

function websocketStatus(params: WebSocketStatusParams) -> string {
  return websocketGenerationResult()
}

function websocketRecord(params: WebSocketStatusParams) -> WebSocketRecordResult {
  return { value: params.value, accepted: true }
}

function websocketArray(params: Array<string>) -> Array<string> {
  return params
}

function websocketVoid(params: WebSocketStatusParams) -> void {}

function websocketExpectedFailure(params: WebSocketStatusParams) -> WebSocketResultUnion {
  return { tag: "expectedFailure", reason: params.value }
}

function websocketThrow(params: WebSocketStatusParams) -> string {
  throw WebSocketPrivateFailure { message: "private-websocket-jsonrpc-secret" }
}

function websocketSlow(params: WebSocketStatusParams) -> string {
  std.time.sleep(Duration.milliseconds(200))
  return params.value
}

function websocketIdentity(
  params: WebSocketIdentityParams,
  connectionId: string,
  businessIdentity: string?
) -> WebSocketIdentityResult {
  return {
    connectionId: connectionId,
    businessIdentity: businessIdentity,
    peerConnectionId: params.connectionId,
    peerBusinessIdentity: params.businessIdentity,
  }
}

function typed(body: string) -> string {
  return body
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

function slow(body: string) -> string {
  std.time.sleep(Duration.milliseconds(200))
  return body
}
"#,
            websocket_generation_source,
            database_source,
            replacement_source,
        ),
    )
    .expect("gateway source");
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host must live below the Skiff root")
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
            "skiff-runtime-host-{name}-{}-{timestamp}-{sequence}",
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
