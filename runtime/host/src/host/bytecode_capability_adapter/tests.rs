use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    task::{Context, Poll, Wake, Waker},
};

use skiff_artifact_model::{
    FileIrRef, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
};
use skiff_runtime_boundary::recoverable::FailClosedRecoverableBehaviorHooks;
use skiff_runtime_capability_context::{
    CancellationToken, DbCapabilitySource, DbCapabilityTarget, DbCapabilityTargetId,
    DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans,
};
use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
};
use skiff_runtime_request::{
    BytecodeDbChildComposition, BytecodeHttpFailure, BytecodeHttpRequest, ExecutionBudget,
    ExecutionControl,
};
use skiff_runtime_service_db::ServiceDbRuntime;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

use super::*;
use crate::{
    capability_context::stream_runtime_streams_active,
    host::{RuntimeConfig, RuntimeHost},
};

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

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn test_host() -> RuntimeHost {
    RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::new(TestDbProviderFactory),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-phase-5-bytecode-http".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-phase-5-bytecode-http-home"),
        profile: "test".to_string(),
        bytecode_only: true,
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("Phase 5 bytecode HTTP host")
}

fn exact_db_target() -> DbCapabilityTarget {
    DbCapabilityTarget::new(
        DbCapabilityTargetId {
            package_artifact_ref: PackageArtifactRef {
                package_id: "test.local/db".to_string(),
                package_version: "1.0.0".to_string(),
                package_build_id: PackageBuildId::new("build:db"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi:db"),
            },
            file_ir_ref: FileIrRef::new("file:db", "test/main.skiff"),
            type_index: 0,
        },
        "Doc",
    )
}

fn recoverable_db_context() -> DbRecoverableRuntimeContext {
    DbRecoverableRuntimeContext {
        behavior_hooks: Arc::new(FailClosedRecoverableBehaviorHooks),
        expected_plans: DbRecoverableRuntimeExpectedPlans::default(),
        artifact_identity: "artifact:test".to_string(),
        build_id: "build:test".to_string(),
        boundary_context: RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::DbValue,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        ),
        retention_expires_at_epoch_millis: None,
    }
}

fn real_db_capability_source() -> DbCapabilitySource {
    let runtime = Arc::new(
        ServiceDbRuntime::new(
            "test".to_string(),
            "test.local/db".to_string(),
            "mongodb://127.0.0.1:1/?directConnection=true".to_string(),
            &[],
        )
        .expect("empty serviceDb runtime builds without connecting"),
    );
    DbCapabilitySource::new(Some(runtime.capability_factory()))
}

#[test]
fn host_composition_injects_real_db_child_composition() {
    let host = test_host();
    let db_child = BytecodeDbChildComposition {
        capability_context: Some(
            real_db_capability_source().context_for_request("test.local/db", "request"),
        ),
        recoverable_context: Some(recoverable_db_context()),
        exact_target: Some(exact_db_target()),
    };
    let composition =
        bytecode_request_child_composition_with_db_child(&host, db_child, "test-request");

    assert!(composition.db_child.is_available());
    assert!(composition.db_child.exact_target().is_ok());
    assert!(composition.callback_hooks.is_some());
    assert!(composition.callback_projector.is_some());
    assert_eq!(
        composition.callback_child.runtime_replica_id,
        host.base_runtime_id.as_str()
    );
    assert!(composition.callback_child.is_available());
    assert!(!composition.actor_child.is_available());
}

#[test]
fn host_composition_db_child_fails_closed_when_capability_is_missing() {
    let host = test_host();
    let db_child = BytecodeDbChildComposition {
        capability_context: None,
        recoverable_context: Some(recoverable_db_context()),
        exact_target: Some(exact_db_target()),
    };
    let composition =
        bytecode_request_child_composition_with_db_child(&host, db_child, "test-request");

    assert!(!composition.db_child.is_available());
}

#[test]
fn host_composition_db_child_fails_closed_when_exact_target_is_missing() {
    let host = test_host();
    let db_child = BytecodeDbChildComposition {
        capability_context: Some(
            real_db_capability_source().context_for_request("test.local/db", "request"),
        ),
        recoverable_context: Some(recoverable_db_context()),
        exact_target: None,
    };
    let composition =
        bytecode_request_child_composition_with_db_child(&host, db_child, "test-request");

    assert!(!composition.db_child.is_available());
}

fn execution_control(
    cancellation: CancellationToken,
) -> (
    skiff_runtime_request::OwnedExecutionControl,
    skiff_runtime_capability_context::ExecutionScope,
) {
    let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
    let control = ExecutionControl::new(cancellation, &budget).owned();
    let scope = control.execution_scope().clone();
    (control, scope)
}

fn request(url: String) -> BytecodeHttpRequest {
    BytecodeHttpRequest {
        method: "POST".to_string(),
        url,
        headers: vec![HttpNameValue {
            name: "content-type".to_string(),
            value: "application/octet-stream".to_string(),
        }],
        body: Some(b"request-body".to_vec()),
        timeout_ms: None,
    }
}

#[test]
fn phase_5_bytecode_http_production_ingress_injects_typed_port() {
    assert_eq!(stream_runtime_streams_active(), 0);
    let mut absent_body = request("http://example.invalid/".to_string());
    absent_body.body = None;
    assert_eq!(
        request_value(absent_body).get("body"),
        Some(&Value::Null),
        "nullable HTTP body absence must remain language null"
    );
    let mut present_empty_body = request("http://example.invalid/".to_string());
    present_empty_body.body = Some(Vec::new());
    assert_eq!(
        request_value(present_empty_body)
            .get("body")
            .and_then(bytes_payload),
        Some(Vec::new()),
        "present zero-length HTTP bytes must not collapse into null"
    );
    let host = test_host();
    let cancellation = CancellationToken::new();
    let port = host.bytecode_http_client_port(cancellation.clone(), 1024);
    let (execution, scope) = execution_control(cancellation);
    let mut future = port.request(
        BytecodeHttpRequest {
            method: "TRACE".to_string(),
            url: "http://example.invalid/".to_string(),
            headers: Vec::new(),
            body: None,
            timeout_ms: None,
        },
        execution,
    );

    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Err(BytecodeHttpFailure::InvalidInput(_)))
    ));
    assert_eq!(
        scope.lifecycle_snapshot(),
        Default::default(),
        "pre-I/O validation must not acquire a pending scope lease"
    );
    assert_eq!(
        stream_runtime_streams_active(),
        0,
        "bytecode composition must never activate the legacy stream registry"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_bytecode_http_request_reuses_production_lower_and_same_scope() {
    assert_eq!(stream_runtime_streams_active(), 0);
    let (url, accepted, release, server) = gated_http_server(
        201,
        vec![("x-phase-5", "request")],
        b"response-body".to_vec(),
    )
    .await;
    let cancellation = CancellationToken::new();
    let port = ProductionBytecodeHttpClientPort::new(
        cancellation.clone(),
        1024,
        HttpRuntimeOptions::explicit(true),
    );
    let (execution, scope) = execution_control(cancellation);
    let task = tokio::spawn(port.request(request(url), execution));

    let observed_request = accepted.await.expect("production lower reached TCP server");
    assert_exact_request(&observed_request);
    let snapshot = scope.lifecycle_snapshot();
    assert_eq!(snapshot.active_leases, 1);
    assert_eq!(snapshot.active_waiters, 1);
    assert_eq!(snapshot.active_timers, 0);
    release.send(()).expect("release HTTP response");

    let response = task
        .await
        .expect("bytecode HTTP task")
        .expect("bytecode HTTP response");
    assert_eq!(response.status, 201);
    assert!(response
        .headers
        .iter()
        .any(|header| header.name == "x-phase-5" && header.value == "request"));
    assert_eq!(response.body, b"response-body");
    server.await.expect("HTTP test server");
    assert_eq!(scope.lifecycle_snapshot(), Default::default());
    assert_eq!(stream_runtime_streams_active(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_bytecode_http_stream_requires_table_runtime_without_legacy_fallback() {
    assert_eq!(stream_runtime_streams_active(), 0);
    let (url, accepted, release, server) = gated_http_server(200, Vec::new(), Vec::new()).await;
    let cancellation = CancellationToken::new();
    let port = ProductionBytecodeHttpClientPort::new(
        cancellation.clone(),
        1024,
        HttpRuntimeOptions::explicit(true),
    );
    let (execution, scope) = execution_control(cancellation);
    let input = request_value(request(url));
    let context = port.context.clone();
    let task = tokio::spawn(async move {
        context
            .dispatch_http_stream_with_execution_scope(
                &input,
                Some(&leaf_bytes_plan()),
                execution.execution_scope().clone(),
            )
            .await
    });

    accepted
        .await
        .expect("production stream lower reached TCP server");
    assert_eq!(scope.lifecycle_snapshot().active_leases, 1);
    release.send(()).expect("release HTTP stream head");
    let error = task
        .await
        .expect("bytecode HTTP stream task")
        .expect_err("stream context without registrar must fail closed");
    let CurrentScopeHttpFailure::Runtime(error) = error else {
        panic!("missing ResourceTable runtime must be a provider-contract failure")
    };
    let payload = error
        .ordinary_payload()
        .expect("missing stream runtime is ordinary provider-contract evidence");
    assert_eq!(payload.code, "InvalidArtifact");
    assert!(payload.message.contains("no injected stream runtime"));
    server.await.expect("HTTP stream test server");
    assert_eq!(scope.lifecycle_snapshot(), Default::default());
    assert_eq!(
        stream_runtime_streams_active(),
        0,
        "missing ResourceTable runtime must not fall back to legacy"
    );
}

async fn gated_http_server(
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: Vec<u8>,
) -> (
    String,
    oneshot::Receiver<Vec<u8>>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Phase 5 HTTP server");
    let address = listener.local_addr().expect("HTTP server address");
    let (accepted_sender, accepted) = oneshot::channel();
    let (release, release_receiver) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept HTTP request");
        let request = read_http_request(&mut stream).await;
        accepted_sender.send(request).expect("observe HTTP request");
        release_receiver.await.expect("HTTP response release");

        let mut response = format!(
            "HTTP/1.1 {status} Phase5\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str("\r\n");
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write HTTP response head");
        stream
            .write_all(&body)
            .await
            .expect("write HTTP response body");
    });
    (
        format!("http://{address}/phase-5"),
        accepted,
        release,
        server,
    )
}

fn assert_exact_request(request: &[u8]) {
    let header_end = request
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .map(|index| index + 4)
        .expect("observed HTTP request header terminator");
    let headers = std::str::from_utf8(&request[..header_end]).expect("HTTP request headers UTF-8");
    assert_eq!(headers.lines().next(), Some("POST /phase-5 HTTP/1.1"));
    assert!(headers.lines().any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        name.eq_ignore_ascii_case("content-type")
            && value
                .trim()
                .eq_ignore_ascii_case("application/octet-stream")
    }));
    assert_eq!(&request[header_end..], b"request-body");
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.expect("read HTTP request");
        assert!(read > 0, "HTTP request ended before its headers");
        request.extend_from_slice(&chunk[..read]);
        if let Some(index) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&request[..header_end]).expect("HTTP request headers UTF-8");
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while request.len() - header_end < content_length {
        let read = stream
            .read(&mut chunk)
            .await
            .expect("read HTTP request body");
        assert!(read > 0, "HTTP request ended before its body");
        request.extend_from_slice(&chunk[..read]);
    }
    request.truncate(header_end + content_length);
    request
}

fn callback_interface(package_id: &str) -> skiff_artifact_model::InterfaceInstantiationRef {
    skiff_artifact_model::InterfaceInstantiationRef {
        interface_abi_id: format!(
            "{{\"symbol\":{{\"package\":{{\"packageId\":\"{package_id}\"}},\"symbolPath\":\"Reader\"}}}}"
        ),
        canonical_type_args: Vec::new(),
    }
}

fn callback_local_table(
    method_abi: &str,
    signature: &skiff_runtime_linked_bytecode::LinkedCallableSignature,
) -> skiff_runtime_linked_bytecode::LinkedLocalInterfaceTable {
    use skiff_artifact_model::ReceiverCallAbi;
    use skiff_runtime_linked_bytecode::{
        FunctionIndex, LinkedInterfaceMethodAbiId, LinkedLocalInterfaceMethod,
        LinkedLocalInterfaceTable, TypeIndex,
    };
    let method = LinkedLocalInterfaceMethod::new(
        0,
        "handle",
        LinkedInterfaceMethodAbiId::parse(method_abi).expect("fixture method ABI"),
        signature.clone(),
        FunctionIndex::new(1),
        ReceiverCallAbi::ExplicitSelfFirst,
    )
    .expect("fixture local method is canonical");
    LinkedLocalInterfaceTable::new(TypeIndex::new(0), Box::new([method]))
        .expect("fixture local table is canonical")
}

fn callback_provider_table(
    interface: &skiff_artifact_model::InterfaceInstantiationRef,
    method_abi: &str,
    signature: &skiff_runtime_linked_bytecode::LinkedCallableSignature,
) -> skiff_runtime_linked_bytecode::LinkedInterfaceTable {
    use skiff_runtime_linked_bytecode::{
        InterfaceTableIndex, LinkedInterfaceInstantiation, LinkedInterfaceMethodAbiId,
        LinkedInterfaceRequirementMethod, LinkedInterfaceRequirementTable, LinkedInterfaceTable,
        LinkedInterfaceTableKind,
    };
    let method = LinkedInterfaceRequirementMethod::new(
        0,
        LinkedInterfaceMethodAbiId::parse(method_abi).expect("fixture method ABI"),
        signature.clone(),
    );
    let requirement = LinkedInterfaceRequirementTable::new(Box::new([method]))
        .expect("fixture callback requirement is canonical");
    let instantiation = LinkedInterfaceInstantiation::new(interface.clone(), Box::new([]))
        .expect("fixture interface instantiation is canonical");
    LinkedInterfaceTable::new(
        InterfaceTableIndex::new(0),
        instantiation,
        LinkedInterfaceTableKind::Callback(requirement),
    )
}

fn callback_provider_facts(
    image: &DeploymentExecutionImage,
) -> (
    &skiff_artifact_model::InterfaceInstantiationRef,
    &skiff_runtime_linked_bytecode::LinkedInterfaceRequirementMethod,
) {
    image
        .interface_tables()
        .iter()
        .find_map(|row| match row.kind() {
            skiff_runtime_linked_bytecode::LinkedInterfaceTableKind::Callback(requirement) => {
                requirement
                    .methods()
                    .first()
                    .map(|method| (row.interface().artifact(), method))
            }
            _ => None,
        })
        .expect("callback provider fixture image has one callback method")
}

#[test]
fn callback_methods_correlates_exact_interface_and_method_abi() {
    let images = callback_images();
    let (interface, method) = callback_provider_facts(images.provider.as_ref());
    let local = callback_local_table(method.method_abi_id().as_str(), method.signature());
    let provider = callback_provider_table(
        interface,
        method.method_abi_id().as_str(),
        method.signature(),
    );
    let correlation = callback_methods(
        &local,
        interface,
        &[provider],
        images.caller.as_ref(),
        images.provider.as_ref(),
    )
    .expect("exact provider ABI should correlate");
    assert_eq!(&correlation.provider_interface, interface);
    let binding = correlation
        .methods
        .get(&(0, method.method_abi_id().as_str().to_string()))
        .expect("provider ABI key is exact");
    assert_eq!(
        binding.function,
        skiff_runtime_linked_bytecode::FunctionIndex::new(1)
    );
    assert_eq!(binding.source_abi, method.method_abi_id().as_str());
}

#[test]
fn callback_methods_rejects_same_method_name_with_different_abi() {
    let images = callback_images();
    let (interface, method) = callback_provider_facts(images.provider.as_ref());
    let local = callback_local_table("method-abi:caller-handle", method.signature());
    let provider =
        callback_provider_table(interface, "method-abi:provider-handle", method.signature());
    // The provider method has the same slot and semantic name as the caller
    // method but a different exact ABI; suffix/name correlation must fail.
    assert!(matches!(
        callback_methods(
            &local,
            interface,
            &[provider],
            images.caller.as_ref(),
            images.provider.as_ref(),
        ),
        Err(skiff_runtime_request::BytecodeCallbackChildError::WrongOperation { .. })
    ));
}

#[test]
fn callback_methods_rejects_cross_package_interface_even_with_same_stable_key() {
    let images = callback_images();
    let local_interface = callback_interface("example.com/caller");
    let (provider_interface, method) = callback_provider_facts(images.provider.as_ref());
    let local = callback_local_table(method.method_abi_id().as_str(), method.signature());
    let provider = callback_provider_table(
        provider_interface,
        method.method_abi_id().as_str(),
        method.signature(),
    );
    assert!(matches!(
        callback_methods(
            &local,
            &local_interface,
            &[provider],
            images.caller.as_ref(),
            images.provider.as_ref(),
        ),
        Err(skiff_runtime_request::BytecodeCallbackChildError::MissingFacts { .. })
    ));
}

#[test]
fn callback_methods_rejects_provider_signature_drift() {
    use skiff_artifact_model::{CallableEffectSummary, ParamModeIr};
    use skiff_runtime_linked_bytecode::{
        InterfaceTableIndex, LinkedCallableSignature, LinkedInterfaceInstantiation,
        LinkedInterfaceMethodAbiId, LinkedInterfaceRequirementMethod,
        LinkedInterfaceRequirementTable, LinkedInterfaceTable, LinkedInterfaceTableKind,
        LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
    };
    let images = callback_images();
    let (interface, provider_method) = callback_provider_facts(images.provider.as_ref());
    let local = callback_local_table(
        provider_method.method_abi_id().as_str(),
        provider_method.signature(),
    );
    let drifted = LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0), TypeIndex::new(1)]),
        Box::new([ParamModeIr::Value, ParamModeIr::Value]),
        Box::new([
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            },
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            },
        ]),
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect("drifted fixture signature is valid");
    let method = LinkedInterfaceRequirementMethod::new(
        0,
        LinkedInterfaceMethodAbiId::parse(provider_method.method_abi_id().as_str())
            .expect("fixture method ABI"),
        drifted,
    );
    let requirement = LinkedInterfaceRequirementTable::new(Box::new([method]))
        .expect("drifted fixture requirement is canonical");
    let instantiation = LinkedInterfaceInstantiation::new(interface.clone(), Box::new([]))
        .expect("fixture interface instantiation is canonical");
    let provider = LinkedInterfaceTable::new(
        InterfaceTableIndex::new(0),
        instantiation,
        LinkedInterfaceTableKind::Callback(requirement),
    );
    assert!(matches!(
        callback_methods(
            &local,
            interface,
            &[provider],
            images.caller.as_ref(),
            images.provider.as_ref(),
        ),
        Err(skiff_runtime_request::BytecodeCallbackChildError::SignatureMismatch { .. })
    ));
}

struct CallbackImagePair {
    caller: Arc<DeploymentExecutionImage>,
    provider: Arc<DeploymentExecutionImage>,
    _root: CallbackFixtureRoot,
}

struct CallbackFixtureRoot(PathBuf);

impl CallbackFixtureRoot {
    fn new(prefix: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let ordinal = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p6-callback-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create callback adapter fixture root");
        Self(path)
    }
}

impl Drop for CallbackFixtureRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn callback_images() -> &'static CallbackImagePair {
    static IMAGES: OnceLock<CallbackImagePair> = OnceLock::new();
    IMAGES.get_or_init(build_callback_images)
}

fn build_callback_images() -> CallbackImagePair {
    use skiff_compiler::{authoring::seed_official_std_package, CompilerPlatformSources};
    use skiff_deployment::storage::CanonicalArtifactStore;

    let root = CallbackFixtureRoot::new("callback-adapter");
    let repository = repository_root();
    let sources = CompilerPlatformSources::new(&repository)
        .expect("open repository compiler platform sources");
    seed_official_std_package(&sources, &root.0)
        .expect("seed canonical std into callback adapter fixture store");
    let provider = publish_callback_package(
        &sources,
        &repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-6/callback-provider"),
        &root.0,
    );
    let caller = publish_callback_package(
        &sources,
        &repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-6/callback-positive"),
        &root.0,
    );
    let store = CanonicalArtifactStore::open(&root.0).expect("open callback adapter fixture store");
    CallbackImagePair {
        caller: callback_image(&store, &caller),
        provider: callback_image(&store, &provider),
        _root: root,
    }
}

fn publish_callback_package(
    sources: &skiff_compiler::CompilerPlatformSources,
    fixture: &Path,
    root: &Path,
) -> skiff_artifact_model::ServiceDeploymentRef {
    use skiff_compiler::authoring::{build_authoring_object, AuthoringObject};

    let receipt = build_authoring_object(
        sources,
        AuthoringObject::Package,
        fixture,
        root,
        "skiff-test",
        true,
    )
    .expect("callback adapter fixture publishes through production authoring");
    serde_json::from_value(
        receipt
            .pointer("/serviceDeploymentReceipt/deployment")
            .cloned()
            .expect("callback adapter authoring receipt has deployment"),
    )
    .expect("callback adapter deployment receipt remains typed")
}

fn callback_image(
    store: &skiff_deployment::storage::CanonicalArtifactStore,
    deployment: &skiff_artifact_model::ServiceDeploymentRef,
) -> Arc<DeploymentExecutionImage> {
    use skiff_runtime_linker::link_deployment_execution_image;
    use skiff_runtime_loader::load_deployment_bytecode_from_store;

    let hydrated = load_deployment_bytecode_from_store(store, deployment)
        .expect("hydrate callback adapter fixture deployment");
    Arc::new(
        link_deployment_execution_image(hydrated, &callback_link_limits())
            .expect("link callback adapter fixture deployment"),
    )
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host is below repository root")
        .to_path_buf()
}

fn callback_link_limits() -> skiff_runtime_linker::LinkLimits {
    use skiff_runtime_linker::LinkLimits;

    LinkLimits {
        max_packages: 256,
        max_root_specializations: 100_000,
        max_specializations: 1_000_000,
        max_code_words_per_function: 1_000_000,
        max_total_code_words: 100_000_000,
        max_relocations_per_function: 100_000,
        max_total_relocations: 10_000_000,
        max_image_table_entries: 1_000_000,
        max_total_image_table_entries: 10_000_000,
        max_total_function_table_entries: 10_000_000,
        max_type_nesting_depth: 64,
        max_expanded_type_nodes: 1_000_000,
        max_expanded_type_bytes: 64 * 1024 * 1024,
        max_constant_graph_nodes: 1_000_000,
        max_constant_graph_edges: 1_000_000,
    }
}
