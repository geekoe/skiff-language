use std::{
    sync::Arc,
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
    assert_eq!(
        composition.callback_child.runtime_replica_id,
        host.base_runtime_id.as_str()
    );
    assert!(!composition.callback_child.is_available());
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
