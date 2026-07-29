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
    AssemblyActivationServiceDb, FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef,
    PackageSchemaIndex, PackageSchemaIndexRef, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageSchemaTypeRecordRef, PackageTypeRef, PublicationResourceRef, RuntimeAssembly,
    RuntimeAssemblyRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef, ServiceIngressKey, TypeRefIr,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::{assembly::resolve_runtime_assembly, storage::CanonicalArtifactStore};
use skiff_runtime_loader::{
    FilesystemRuntimeAssemblyContentResolver, RuntimeAssemblyContentResolver,
    RuntimeAssemblyRecordResolver,
};
use skiff_test_runner::canonical_std_seed::seed_canonical_std;
use skiff_test_runner::package_service_host_fixture::prepare_package_service_host_fixture;

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
    deployment_ref: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    original_root_ref: PackageArtifactRef,
    root_artifact: Arc<PackageArtifact>,
    original_std_ref: PackageArtifactRef,
    std_artifact: Arc<PackageArtifact>,
    std_schema_index: Arc<PackageSchemaIndex>,
    _temp: TempFixture,
}

struct CurrentScopeCompiledFixture {
    assembly: Arc<RuntimeAssembly>,
    artifact_root: PathBuf,
    _temp: TempFixture,
}

impl CompiledGatewayFixture {
    fn resolver(&self) -> CompiledGatewayResolver<'_> {
        CompiledGatewayResolver {
            inner: FilesystemRuntimeAssemblyContentResolver::open(&self.artifact_root)
                .expect("gateway filesystem resolver"),
            fixture: self,
        }
    }
}

struct CompiledGatewayResolver<'a> {
    inner: FilesystemRuntimeAssemblyContentResolver,
    fixture: &'a CompiledGatewayFixture,
}

impl RuntimeAssemblyContentResolver for CompiledGatewayResolver<'_> {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        if reference == &self.fixture.deployment_ref {
            return Ok(Arc::clone(&self.fixture.deployment));
        }
        self.inner.resolve_deployment(reference)
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.inner.resolve_contract(reference)
    }

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        if reference == &self.fixture.std_artifact.package_schema_index {
            return Ok(Arc::clone(&self.fixture.std_schema_index));
        }
        self.inner.resolve_package_schema_index(reference)
    }

    fn resolve_package_schema_type(
        &self,
        reference: &PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<PackageSchemaTypeRecord>> {
        self.inner.resolve_package_schema_type(reference)
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        if reference == &skiff_artifact_identity::package_artifact_ref(&self.fixture.root_artifact)?
        {
            return Ok(Arc::clone(&self.fixture.root_artifact));
        }
        if reference == &skiff_artifact_identity::package_artifact_ref(&self.fixture.std_artifact)?
        {
            return Ok(Arc::clone(&self.fixture.std_artifact));
        }
        self.inner.resolve_package(reference)
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        let package = if package.package_id == self.fixture.root_artifact.package_id {
            &self.fixture.original_root_ref
        } else if package.package_id == self.fixture.std_artifact.package_id {
            &self.fixture.original_std_ref
        } else {
            package
        };
        self.inner.resolve_file_ir(package, reference)
    }

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        let package = if package.package_id == self.fixture.root_artifact.package_id {
            &self.fixture.original_root_ref
        } else if package.package_id == self.fixture.std_artifact.package_id {
            &self.fixture.original_std_ref
        } else {
            package
        };
        self.inner.resolve_static_resource(package, reference)
    }
}

impl RuntimeAssemblyRecordResolver for CompiledGatewayResolver<'_> {
    fn resolve_runtime_assembly(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<RuntimeAssembly>> {
        let fixture_reference =
            skiff_artifact_identity::runtime_assembly_ref(&self.fixture.assembly)?;
        if reference == &fixture_reference {
            return Ok(Arc::clone(&self.fixture.assembly));
        }
        self.inner.resolve_runtime_assembly(reference)
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
        "skiff-runtime-assembly-v3:sha256:ce8c979de4c6786ee9c2fbf2ad01fbfa2271b33a074682e2e66f5a77654f6688"
    );
    assert_eq!(
        receipt.consumer_package.package_build_id.as_str(),
        "skiff-package-build-v10:sha256:9b03476e93f5ccb66dc69ff899f4a8fb9c68593e70c5aeda94d4e865aab688ad"
    );
    assert_eq!(
        receipt
            .consumer_deployment
            .deployment_artifact_identity
            .as_str(),
        "skiff-deployment-artifact-v4:sha256:bfa01d12d90d7a9e5af9da153b63862270a52eaffe59383a4563cff2a0dde2a4"
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
    let original_deployment = store
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
    for binding in &original_deployment.package_bindings {
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
    let original_std = packages
        .iter()
        .find(|package| package.package_id == "skiff.run/std")
        .expect("gateway fixture std package");
    let original_std_ref = skiff_artifact_identity::package_artifact_ref(original_std)
        .expect("canonical std package ref");
    let mut std_artifact = original_std.as_ref().clone();
    let original_std_schema = store
        .read_package_schema_index(&std_artifact.package_schema_index)
        .expect("canonical std schema index");
    localize_std_package_abi_schema_refs(&mut std_artifact, &original_std_schema);
    let std_schema_types = BTreeMap::new();
    let std_schema_identity = skiff_artifact_identity::package_schema_index_identity(
        &std_artifact.package_id,
        &std_schema_types,
    )
    .expect("empty std schema identity");
    std_artifact.package_schema_index = PackageSchemaIndexRef {
        package_id: std_artifact.package_id.clone(),
        package_schema_index_identity: std_schema_identity.clone(),
    };
    std_artifact.package_schema_type_records.clear();
    skiff_artifact_identity::assign_package_artifact_identities(&mut std_artifact)
        .expect("HTTP fixture std identity");
    let std_artifact = Arc::new(std_artifact);
    let std_ref = skiff_artifact_identity::package_artifact_ref(&std_artifact)
        .expect("schema-neutral std fixture ref");
    let std_schema_index = Arc::new(PackageSchemaIndex {
        package_id: std_artifact.package_id.clone(),
        package_schema_index_identity: std_schema_identity,
        types: std_schema_types,
    });

    // The canonical std publication schema currently disagrees with several of its own File IR
    // declarations. These HTTP probes need std execution code, not its independent service-error
    // schema roots, so the test resolver retains exact File IR and rewrites only the Package-ABI
    // nominal references to their equivalent local publication declarations.
    let original_root_ref =
        skiff_artifact_identity::package_artifact_ref(&implementation).expect("gateway root ref");
    let mut root_artifact = implementation.as_ref().clone();
    for requirement in &mut root_artifact.package_requirements {
        if requirement.package_id == std_artifact.package_id {
            requirement.expected_local_abi =
                std_artifact.package_local_abi.local_abi_identity.clone();
            if requirement.expected_package_build.is_some() {
                requirement.expected_package_build = Some(std_artifact.package_build_id.clone());
            }
        }
    }
    skiff_artifact_identity::assign_package_artifact_identities(&mut root_artifact)
        .expect("gateway root fixture identity");
    let root_artifact = Arc::new(root_artifact);
    let root_ref = skiff_artifact_identity::package_artifact_ref(&root_artifact)
        .expect("gateway root fixture ref");

    let mut deployment = original_deployment.as_ref().clone();
    deployment.implementation = root_ref.clone();
    for binding in &mut deployment.package_bindings {
        if binding.key.caller_package_build_id == original_root_ref.package_build_id {
            binding.key.caller_package_build_id = root_ref.package_build_id.clone();
        }
        if binding.package == original_std_ref {
            binding.package = std_ref.clone();
        }
    }
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
        .expect("schema-neutral gateway deployment identity");
    let deployment = Arc::new(deployment);
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
    let package_values = packages
        .iter()
        .map(|package| {
            if package.package_id == std_artifact.package_id {
                std_artifact.as_ref().clone()
            } else if package.package_id == root_artifact.package_id {
                root_artifact.as_ref().clone()
            } else {
                package.as_ref().clone()
            }
        })
        .collect::<Vec<_>>();
    let root = deployment_ref.clone();
    let assembly = Arc::new(
        resolve_runtime_assembly(
            std::slice::from_ref(&root),
            std::slice::from_ref(deployment.as_ref()),
            std::slice::from_ref(contract.as_ref()),
            &package_values,
        )
        .expect("gateway RuntimeAssembly"),
    );
    CompiledGatewayFixture {
        assembly,
        artifact_root,
        deployment_ref,
        deployment,
        original_root_ref,
        root_artifact,
        original_std_ref,
        std_artifact,
        std_schema_index,
        _temp: temp,
    }
}

fn localize_std_package_abi_schema_refs(
    artifact: &mut PackageArtifact,
    schema: &PackageSchemaIndex,
) {
    let local_types = schema
        .types
        .values()
        .map(|entry| {
            let public_path = entry
                .public_path
                .as_deref()
                .expect("canonical std schema public path");
            let source_path = public_path.strip_prefix("std.").unwrap_or(public_path);
            let export =
                artifact
                    .implementation_links
                    .types
                    .get(source_path)
                    .or_else(|| artifact.implementation_links.types.get(public_path))
                    .or_else(|| {
                        artifact.implementation_links.types.values().find(|export| {
                            export.symbol == source_path || export.symbol == public_path
                        })
                    })
                    .unwrap_or_else(|| panic!("std schema path {public_path} implementation link"));
            (
                entry.package_schema_type_id.clone(),
                TypeRefIr::PublicationType {
                    module_path: export.file.module_path.clone(),
                    type_index: export.type_index,
                },
            )
        })
        .collect::<BTreeMap<PackageSchemaTypeId, TypeRefIr>>();
    for symbol in artifact
        .package_local_abi
        .public_symbols
        .values_mut()
        .chain(
            artifact
                .package_local_abi
                .implementation_symbols
                .values_mut(),
        )
    {
        match symbol {
            skiff_artifact_model::PackageLocalAbiSymbol::Callable { signature, .. } => {
                for parameter in &mut signature.parameters {
                    localize_package_type_ref(
                        &mut parameter.ty,
                        &artifact.package_id,
                        &local_types,
                    );
                }
                localize_package_type_ref(
                    &mut signature.return_type,
                    &artifact.package_id,
                    &local_types,
                );
            }
            skiff_artifact_model::PackageLocalAbiSymbol::Constant { ty, .. } => {
                localize_package_type_ref(ty, &artifact.package_id, &local_types);
            }
            skiff_artifact_model::PackageLocalAbiSymbol::Type { .. }
            | skiff_artifact_model::PackageLocalAbiSymbol::PublicInstance { .. } => {}
        }
    }
}

fn localize_package_type_ref(
    ty: &mut PackageTypeRef,
    package_id: &str,
    local_types: &BTreeMap<PackageSchemaTypeId, TypeRefIr>,
) {
    match ty {
        PackageTypeRef::PackageSchema {
            package_id: owner,
            package_schema_type_id,
            ..
        } if owner == package_id => {
            *ty = PackageTypeRef::Local {
                local_type: local_types
                    .get(package_schema_type_id)
                    .unwrap_or_else(|| {
                        panic!("std Package ABI schema type {package_schema_type_id} local link")
                    })
                    .clone(),
            };
        }
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            localize_package_type_ref(interface, package_id, local_types);
            for argument in arguments {
                localize_package_type_ref(argument, package_id, local_types);
            }
        }
        PackageTypeRef::Container { arguments, .. } => {
            for argument in arguments {
                localize_package_type_ref(argument, package_id, local_types);
            }
        }
        PackageTypeRef::Nullable { inner } => {
            localize_package_type_ref(inner, package_id, local_types);
        }
        PackageTypeRef::Local { .. } | PackageTypeRef::PackageSchema { .. } => {}
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
            "state:\n  database:\n    kind: database\n    namespace: pinned-route-b\ntimeout: 2200\nquota: { cpuMillis: 200, memoryBytes: 2097152 }\nprincipal: service:pinned-route-b\nlifecycle: { maxConcurrency: 2 }\n"
        }
        (true, false) => {
            "state:\n  database:\n    kind: database\n    namespace: pinned-route-a\ntimeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:host-http-gateway\nlifecycle: { maxConcurrency: 1 }\n"
        }
        (false, _) => {
            "timeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:host-http-gateway\nlifecycle: { maxConcurrency: 1 }\n"
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
