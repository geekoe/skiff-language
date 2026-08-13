use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use crate::loader::bytecode_admission::BytecodeRouteSelector;
use skiff_artifact_identity::{package_artifact_ref, service_deployment_ref};
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, DeploymentArtifactIdentity, DeploymentRevision,
    GatewayEntryIdentity, IngressProtocol, IngressSelector, WebSocketEntryId,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionCorrelation, BytecodeExecutionEventSink, BytecodeExecutionObservation,
    BytecodeExecutionObserver,
};
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{
        decode_binary_frame, decode_response_end_frame, decode_response_error_frame,
        ValidatedResponseErrorFrame, RUNTIME_FRAME_SCHEMA_VERSION,
    },
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

use super::phase_0_proof_support::PublishedFixture;
use crate::host::request_supervisor::RouterSessionEpoch;
use crate::host::{RuntimeConfig, RuntimeHost};

#[derive(Default)]
struct RecordingExecutionSink(Mutex<Vec<BytecodeExecutionObservation>>);

impl BytecodeExecutionEventSink for RecordingExecutionSink {
    fn observe(&self, observation: BytecodeExecutionObservation) {
        self.0
            .lock()
            .expect("bytecode request-lane recording sink lock")
            .push(observation);
    }
}

impl RecordingExecutionSink {
    fn assert_empty(&self, scenario: &str) {
        let observations = self
            .0
            .lock()
            .expect("bytecode request-lane recording sink lock");
        assert!(
            observations.is_empty(),
            "{scenario} must be rejected before route observation or VM dispatch: {observations:?}"
        );
    }
}

static FIXTURE: OnceLock<PublishedFixture> = OnceLock::new();
static LEGACY_DEPLOYMENT: OnceLock<skiff_artifact_model::ServiceDeploymentRef> = OnceLock::new();

fn fixture() -> &'static PublishedFixture {
    FIXTURE.get_or_init(|| PublishedFixture::build("host-bytecode-http-fixture"))
}

fn legacy_deployment(
    fixture: &PublishedFixture,
) -> &'static skiff_artifact_model::ServiceDeploymentRef {
    LEGACY_DEPLOYMENT.get_or_init(|| {
        let store =
            CanonicalArtifactStore::open(fixture.artifact_root.path()).expect("artifact store");
        let mut legacy_package = fixture.package_artifact.as_ref().clone();
        legacy_package.bytecode = None;
        legacy_package.bytecode_statement_manifest_identity =
            derive_bytecode_statement_manifest_identity(&legacy_package.package_id, &[])
                .expect("empty bytecode statement manifest is canonical");
        legacy_package.synthetic_callback_owners.clear();
        legacy_package.bytecode_schema_records.clear();
        skiff_artifact_identity::assign_package_artifact_identities(&mut legacy_package)
            .expect("legacy package identities");
        let legacy_package_ref =
            package_artifact_ref(&legacy_package).expect("legacy package reference");
        store
            .write_package_artifact(&legacy_package)
            .expect("write legacy package");

        let mut deployment = fixture.deployment_artifact.as_ref().clone();
        deployment.deployment_revision = DeploymentRevision::new("revision-host-legacy-http");
        deployment.deployment_artifact_identity = DeploymentArtifactIdentity::new("unassigned");
        deployment.implementation = legacy_package_ref;
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
            .expect("legacy deployment identity");
        let reference = service_deployment_ref(&deployment);
        store
            .write_service_deployment(&deployment)
            .expect("write legacy deployment");
        reference
    })
}

fn http_route_selector(fixture: &PublishedFixture) -> BytecodeRouteSelector {
    BytecodeRouteSelector::Gateway {
        ingress: IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/phase-0/vcp".to_string(),
        },
        gateway_entry_identity: fixture.gateway_identity.clone(),
    }
}

fn canonical_header(
    fixture: &PublishedFixture,
    request_id: &str,
) -> BytecodeRequestStartFrameHeader {
    canonical_header_for_deployment(&fixture.deployment, request_id, &fixture.gateway_identity)
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
            build_id: Some(deployment.deployment_artifact_identity.as_str().to_string()),
            gateway_entry_identity: gateway_entry_identity.clone(),
            ingress: BytecodeRequestIngressFrameHeader {
                protocol: BytecodeRequestIngressProtocol::Http,
                method: "POST".to_string(),
                path: "/phase-0/vcp".to_string(),
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
            url: "http://api.example.test/phase-0/vcp".to_string(),
            path: "/phase-0/vcp".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: false,
        test_case_capability: None,
        test_case_parent_request_id: None,
    }
}

fn task_header(
    fixture: &PublishedFixture,
    request_id: &str,
) -> BytecodeTaskRequestStartFrameHeader {
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
    fixture: &PublishedFixture,
    request_id: &str,
) -> BytecodeWebSocketConnectRequestStartFrameHeader {
    let gateway_entry_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "0".repeat(64)))
            .expect("mismatched WebSocket gateway identity");
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
                path: "/phase-0/vcp".to_string(),
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
            url: "ws://api.example.test/phase-0/vcp".to_string(),
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

async fn assert_bytecode_control_error(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    expected_request_id: &str,
    expected_code: &str,
    expected_message: &str,
) {
    let message = timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("bytecode control failure response timeout")
        .expect("bytecode control failure response channel closed");
    let RouterWriterMessage::Binary(frame) = message else {
        panic!("bytecode control failure must be a binary transport frame")
    };
    let (header, error) =
        decode_response_error_frame(&frame).expect("typed bytecode response.error");
    assert_eq!(header.request_id(), expected_request_id);
    let ValidatedResponseErrorFrame::Control(error) = error else {
        panic!("bytecode request-lane admission must return a typed control error")
    };
    assert_eq!(error.code, expected_code);
    assert_eq!(error.message, expected_message);
}

fn connection_bootstrap(
    fixture: &PublishedFixture,
) -> crate::host::router_session::ConnectionBootstrap {
    fixture.connection_bootstrap()
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

fn start_test_session(host: &RuntimeHost, id: &str) -> RouterSessionEpoch {
    let epoch = RouterSessionEpoch::from_connection_id(id.to_string()).unwrap();
    assert!(host.request_supervisor.start_session(epoch.clone()));
    epoch
}

async fn assert_disabled_request_lane(
    fixture: &PublishedFixture,
    scenario: &str,
    request_id: &str,
    header: BytecodeRequestStartFrameWireHeader,
    body: Vec<u8>,
    expected_message: &str,
) {
    let mut host = test_host();
    let sink = Arc::new(RecordingExecutionSink::default());
    host.bytecode_execution_event_sink = sink.clone();
    let build_id = fixture.deployment.deployment_artifact_identity.as_str();
    assert!(
        !host.bytecode_deployments.is_loaded_build_id(build_id).await,
        "{scenario} starts with an unloaded deployment"
    );
    let bootstrap = connection_bootstrap(fixture);
    let (sender, mut receiver) = mpsc::unbounded_channel();

    let router_session = start_test_session(&host, "phase-1-request-lane-session");
    host.spawn_bytecode_request(&router_session, header, body, &bootstrap, sender)
        .await;

    assert_bytecode_control_error(
        &mut receiver,
        request_id,
        "UnsupportedRuntimeFeature",
        expected_message,
    )
    .await;
    sink.assert_empty(scenario);
    assert!(
        !host.bytecode_deployments.is_loaded_build_id(build_id).await,
        "{scenario} must fail before deployment load"
    );
    let next = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("disabled request lane must close its response channel");
    assert!(next.is_none(), "{scenario} emitted a second response frame");
    sink.assert_empty(scenario);
}

fn noop_observer() -> BytecodeExecutionObserver {
    BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
        router_session_id: "bytecode-http-test-session".to_string(),
        request_id: "bytecode-http-test-request".to_string(),
    })
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_http_bytecode_request_executes_through_scalar_vm() {
    let fixture = fixture();
    let host = test_host_with_bytecode_only(true);
    let route = host
        .bytecode_deployments
        .route(
            &fixture.deployment,
            fixture.artifact_root.path(),
            http_route_selector(fixture),
            &noop_observer(),
        )
        .await
        .expect("bytecode route should load")
        .expect("fixture must carry a bytecode deployment");
    let target = route
        .execution_entry()
        .expect("gateway target should pin the admitted image");
    assert_eq!(route.owner(), target.image().owner());
    assert_eq!(route.deployment(), &fixture.deployment);
    assert_eq!(
        route.build_id(),
        fixture.deployment.deployment_artifact_identity.as_str()
    );
    assert_ne!(
        route.build_id(),
        fixture.package_ref.package_build_id.as_str(),
        "route identity must not substitute the implementation package build"
    );

    let bootstrap = connection_bootstrap(fixture);
    let header = canonical_header(fixture, "bytecode-http-42");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let router_session = start_test_session(&host, "bytecode-http-session");
    host.spawn_bytecode_request(
        &router_session,
        BytecodeRequestStartFrameWireHeader::Http(header),
        b"2".to_vec(),
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
    assert_eq!(payload, b"3.0");

    let duration_event = host
        .telemetry_producer()
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .find(|event| {
            event.name.as_deref() == Some("request.duration")
                && event.request_id.as_deref() == Some("bytecode-http-42")
        })
        .expect("request envelope identity should reach request telemetry");
    assert_eq!(
        duration_event.build_id.as_deref(),
        Some(fixture.deployment.deployment_artifact_identity.as_str())
    );
    assert_ne!(
        duration_event.build_id.as_deref(),
        Some(fixture.package_ref.package_build_id.as_str())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn admitted_route_and_adapter_remain_pinned_after_store_withdrawal() {
    let fixture = PublishedFixture::build("host-bytecode-pinned-route-fixture");
    let host = test_host_with_bytecode_only(true);
    let selector = http_route_selector(&fixture);
    let route = host
        .bytecode_deployments
        .route(
            &fixture.deployment,
            fixture.artifact_root.path(),
            selector.clone(),
            &noop_observer(),
        )
        .await
        .expect("gateway route should load")
        .expect("fixture must carry a bytecode deployment");
    let adapter = route.http_adapter().expect("HTTP adapter should be pinned");
    let target = route
        .execution_entry()
        .expect("gateway target should pin the admitted image");
    assert_eq!(route.owner(), target.image().owner());
    assert_eq!(
        route.build_id(),
        fixture.deployment.deployment_artifact_identity.as_str()
    );

    let bootstrap = connection_bootstrap(&fixture);
    let withdrawn_root = PathBuf::from(format!(
        "{}.withdrawn",
        fixture.artifact_root.path().display()
    ));
    std::fs::rename(fixture.artifact_root.path(), &withdrawn_root)
        .expect("withdraw admitted artifact store");

    assert_eq!(
        route
            .http_adapter()
            .expect("pinned adapter after withdrawal"),
        adapter
    );
    let cached_route = host
        .bytecode_deployments
        .route(
            &fixture.deployment,
            fixture.artifact_root.path(),
            selector,
            &noop_observer(),
        )
        .await
        .expect("cached route must not reopen the withdrawn store")
        .expect("cached deployment image should remain admitted");
    assert_eq!(cached_route.owner(), route.owner());
    assert_eq!(
        cached_route
            .http_adapter()
            .expect("cached pinned adapter after withdrawal"),
        adapter
    );

    let header = canonical_header(&fixture, "bytecode-http-pinned-store");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let router_session = start_test_session(&host, "bytecode-http-pinned-store-session");
    host.spawn_bytecode_request(
        &router_session,
        BytecodeRequestStartFrameWireHeader::Http(header),
        b"2".to_vec(),
        &bootstrap,
        sender,
    )
    .await;
    let message = timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("pinned-store response timeout")
        .expect("pinned-store response channel closed");
    let RouterWriterMessage::Binary(frame) = message else {
        panic!("pinned-store response must be a binary transport frame")
    };
    let (_header, payload) = decode_response_end_frame(&frame).expect("pinned-store response.end");
    assert_eq!(payload, b"3.0");

    std::fs::remove_dir_all(&withdrawn_root).expect("clean withdrawn artifact store");
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_route_fails_closed_on_missing_or_mismatched_pinned_facts() {
    let fixture = fixture();
    let host = test_host_with_bytecode_only(true);

    let missing_ingress = host
        .bytecode_deployments
        .route(
            &fixture.deployment,
            fixture.artifact_root.path(),
            BytecodeRouteSelector::Gateway {
                ingress: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("POST".to_string()),
                    path: "/missing".to_string(),
                },
                gateway_entry_identity: fixture.gateway_identity.clone(),
            },
            &noop_observer(),
        )
        .await
        .expect_err("missing ingress must fail closed");
    assert!(missing_ingress
        .to_string()
        .contains("has no ingress binding"));

    let mismatched_identity = host
        .bytecode_deployments
        .route(
            &fixture.deployment,
            fixture.artifact_root.path(),
            BytecodeRouteSelector::Gateway {
                ingress: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("POST".to_string()),
                    path: "/phase-0/vcp".to_string(),
                },
                gateway_entry_identity: GatewayEntryIdentity::parse(format!(
                    "skiff-gateway-entry-v2:sha256:{}",
                    "0".repeat(64)
                ))
                .expect("mismatched gateway identity"),
            },
            &noop_observer(),
        )
        .await
        .expect_err("mismatched gateway identity must fail closed");
    assert!(mismatched_identity
        .to_string()
        .contains("identity mismatch"));
}

#[tokio::test(flavor = "current_thread")]
async fn canonical_http_bytecode_only_rejects_non_bytecode_deployment_before_legacy() {
    let fixture = fixture();
    let legacy_deployment = legacy_deployment(fixture);
    let host = test_host_with_bytecode_only(true);
    let legacy_route = host
        .bytecode_deployments
        .route(
            legacy_deployment,
            fixture.artifact_root.path(),
            http_route_selector(fixture),
            &noop_observer(),
        )
        .await
        .expect("legacy deployment bytecode lookup should succeed");
    assert!(
        legacy_route.is_none(),
        "fixture must carry a deployment without a bytecode record"
    );
    let bootstrap = connection_bootstrap(fixture);
    let header = canonical_header_for_deployment(
        legacy_deployment,
        "bytecode-only-legacy-http",
        &fixture.gateway_identity,
    );
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let router_session = start_test_session(&host, "bytecode-only-legacy-session");
    host.spawn_bytecode_request(
        &router_session,
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
    let mut header = canonical_header(fixture, "bytecode-http-server-stream");
    header.mode = "serverStream".to_string();
    assert_disabled_request_lane(
        fixture,
        "server-stream HTTP",
        &header.request_id,
        BytecodeRequestStartFrameWireHeader::Http(header.clone()),
        b"2".to_vec(),
        "bytecode HTTP ingress only supports unary request.start, got serverStream",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn phase_1_request_lane_containment() {
    let fixture = fixture();

    let header = task_header(fixture, "bytecode-task-42");
    assert_disabled_request_lane(
        fixture,
        "task",
        &header.request_id,
        BytecodeRequestStartFrameWireHeader::Task(header.clone()),
        b"{}".to_vec(),
        "bytecode request admission supports only exact unary HTTP gateway requests; the task request lane is disabled",
    )
    .await;

    let header = websocket_connect_header(fixture, "bytecode-websocket-connect-42");
    assert_disabled_request_lane(
        fixture,
        "WebSocket connect",
        &header.request_id,
        BytecodeRequestStartFrameWireHeader::WebSocketConnect(header.clone()),
        Vec::new(),
        "bytecode request admission supports only exact unary HTTP gateway requests; the WebSocket request lane is disabled",
    )
    .await;

    let mut header = canonical_header(fixture, "bytecode-host-test-effect");
    header.test_effects_enabled = true;
    header.test_case_capability = Some("test-case:phase_1_host_lane".to_string());
    assert_disabled_request_lane(
        fixture,
        "host test effect",
        &header.request_id,
        BytecodeRequestStartFrameWireHeader::Http(header.clone()),
        b"2".to_vec(),
        "bytecode request admission supports only the synchronous unary HTTP gateway lane; host test-effect requests are disabled",
    )
    .await;

    let mut header = canonical_header(fixture, "bytecode-child-request");
    header.test_effects_enabled = true;
    header.test_case_capability = Some("test-case:phase_1_child_lane".to_string());
    header.test_case_parent_request_id = Some("request:phase_1_parent".to_string());
    assert_disabled_request_lane(
        fixture,
        "child request",
        &header.request_id,
        BytecodeRequestStartFrameWireHeader::Http(header.clone()),
        b"2".to_vec(),
        "bytecode request admission supports only the synchronous unary HTTP gateway lane; child requests are disabled",
    )
    .await;
}
