use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityFactory, DbCapabilityResult, DbCapabilitySource,
    DbProviderBuildInput, DbProviderFactory, DbProviderSource,
};
use skiff_runtime_host::{RuntimeConfig, RuntimeHost};
use skiff_runtime_transport::protocol::{
    decode_response_chunk_frame, decode_response_end_frame, decode_response_start_frame,
    decode_runtime_capabilities_frame, decode_runtime_health_frame, decode_typed_binary_frame,
    encode_binary_frame, encode_router_bootstrap_frame, encode_runtime_registered_frame,
    BytecodeHttpRequestFrameHeader, BytecodeRequestCallerFrameHeader,
    BytecodeRequestIngressFrameHeader, BytecodeRequestIngressProtocol,
    BytecodeRequestRoutingFrameHeader, BytecodeRequestStartFrameHeader,
    BytecodeRequestTraceFrameHeader, ResponseEndFrameMetadata,
    RouterBootstrapActivationFrameHeader, RouterBootstrapFrameHeader,
    RouterBootstrapHttpFrameHeader, RouterBootstrapServiceDbFrameHeader,
    RuntimeDispatchModeCapability, RuntimeHealthCountersFrameHeader, RuntimeRegisteredFrameHeader,
    TypedEnvelope, RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

use super::{
    stages::published_positive,
    tcp_server::{Phase5TcpServer, RequestObservation},
};

const VCP_PATH: &str = "/phase-5/vcp";
const IO_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_RUNTIME_HOME: AtomicU64 = AtomicU64::new(0);

struct TopLevelEvidence {
    response_status: u16,
    response_headers: Vec<(String, String)>,
    response_chunks: Vec<Vec<u8>>,
    outbound: Vec<RequestObservation>,
    active_health: RuntimeHealthCountersFrameHeader,
    terminal_health: RuntimeHealthCountersFrameHeader,
}

pub async fn scheduler_to_request_response() {
    let evidence = drive_top_level_vcp("s6-response", "phase-5-s6").await;
    assert_eq!(evidence.response_status, 207);
    assert!(evidence.response_headers.is_empty());
    assert_eq!(
        evidence.response_chunks.concat(),
        b"U=UNARY|A=LEFT-1LEFT-2|B=RIGHT-1RIGHT-2"
    );
}

pub async fn vcp_production_composition() {
    let evidence = drive_top_level_vcp("vcp-host", "phase-5-vcp").await;
    assert_eq!(evidence.response_status, 207);
    assert_eq!(evidence.response_chunks.len(), 6);
    assert_eq!(
        evidence.response_chunks.concat(),
        b"U=UNARY|A=LEFT-1LEFT-2|B=RIGHT-1RIGHT-2"
    );
    assert_eq!(
        evidence
            .outbound
            .iter()
            .map(|entry| (entry.method.as_str(), entry.path.as_str()))
            .collect::<Vec<_>>(),
        [
            ("GET", "/request"),
            ("GET", "/stream/left"),
            ("GET", "/stream/right"),
        ]
    );
}

pub async fn structural_no_bypass() {
    let evidence = drive_top_level_vcp("structural", "phase-5-structural").await;
    assert_eq!(evidence.response_status, 207);
    assert_eq!(
        evidence.response_chunks.concat(),
        b"U=UNARY|A=LEFT-1LEFT-2|B=RIGHT-1RIGHT-2",
        "the structural proof must execute the production HTTP provider"
    );
    assert_eq!(
        evidence
            .outbound
            .iter()
            .map(|entry| (entry.method.as_str(), entry.path.as_str()))
            .collect::<Vec<_>>(),
        [
            ("GET", "/request"),
            ("GET", "/stream/left"),
            ("GET", "/stream/right"),
        ],
        "no test executor may replace the exact production outbound routes"
    );
    assert_eq!(
        evidence.active_health.outbound_stream_leases_active, 2,
        "the one request table must own exactly the two coexisting stream handles"
    );
    assert_eq!(
        evidence.active_health.stream_runtime_streams_active, 0,
        "the legacy StreamRuntime registry must stay inactive on the bytecode path"
    );
    assert!(
        health_counters_all_zero(&evidence.terminal_health),
        "the single production owner inventory did not return to zero: {:?}",
        evidence.terminal_health
    );
}

/// A deterministic in-process Router-side peer around the public production
/// RuntimeHost. It owns no dispatcher, executor, resource handle, response
/// event, or VM frame: it sends only frames built by the canonical transport
/// encoders and observes only frames emitted by RuntimeHost over a real
/// WebSocket. The Router production chain is exercised separately by G7.
async fn drive_top_level_vcp(prefix: &str, request_id: &str) -> TopLevelEvidence {
    let fixture = published_positive(prefix);
    let upstream = Phase5TcpServer::start();
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind Phase 5 RuntimeHost peer");
    let router_address = listener.local_addr().expect("RuntimeHost peer address");
    let runtime_home = RuntimeHome::new(prefix);
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::new(TestDbProviderFactory),
        router_url: format!("ws://{router_address}"),
        base_runtime_id: format!("runtime-phase-5-{prefix}"),
        runtime_home: runtime_home.path().to_path_buf(),
        profile: "skiff-test".to_string(),
        bytecode_only: true,
        http_response_max_bytes: 64 * 1024,
        http_egress_proxy: None,
    })
    .expect("construct production RuntimeHost");
    let host_task = AbortOnDrop(tokio::spawn(host.run_forever()));

    let (socket, _) = tokio::time::timeout(IO_TIMEOUT, listener.accept())
        .await
        .expect("RuntimeHost did not connect to its Router peer")
        .expect("accept RuntimeHost TCP connection");
    let mut websocket = accept_async(socket)
        .await
        .expect("accept RuntimeHost WebSocket");
    let bootstrap = RouterBootstrapFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "router.bootstrap".to_string(),
        artifacts_path: fixture.artifact_root.display().to_string(),
        service_db: RouterBootstrapServiceDbFrameHeader {
            mongo_url: "mongodb://127.0.0.1:1/phase5-proof".to_string(),
        },
        http: RouterBootstrapHttpFrameHeader {
            max_response_bytes: 64 * 1024,
        },
        activation: RouterBootstrapActivationFrameHeader {
            profile: "skiff-test".to_string(),
        },
    };
    websocket
        .send(Message::Binary(
            encode_router_bootstrap_frame(&bootstrap)
                .expect("encode production router.bootstrap")
                .into(),
        ))
        .await
        .expect("send router.bootstrap");

    let capabilities_bytes = next_binary_of_type(&mut websocket, "runtime.capabilities").await;
    let capabilities = decode_runtime_capabilities_frame(&capabilities_bytes)
        .expect("decode production runtime.capabilities");
    assert_eq!(
        capabilities.capabilities.artifact_root.as_deref(),
        Some(fixture.artifact_root.to_string_lossy().as_ref())
    );
    assert!(capabilities.capabilities.lazy_load);
    assert!(capabilities
        .capabilities
        .dispatch_modes
        .contains(&RuntimeDispatchModeCapability::ServerStream));
    websocket
        .send(Message::Binary(
            encode_runtime_registered_frame(&RuntimeRegisteredFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "runtime.registered".to_string(),
                runtime_id: capabilities.runtime_id,
            })
            .expect("encode production runtime.registered")
            .into(),
        ))
        .await
        .expect("send runtime.registered");
    let initial_health = next_health(&mut websocket).await;
    assert!(
        health_counters_all_zero(&initial_health),
        "RuntimeHost registration began with residual authority: {initial_health:?}"
    );

    let gateway = fixture.gateway(VCP_PATH);
    let request_header = BytecodeRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "serverStream".to_string(),
        caller: BytecodeRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: BytecodeRequestRoutingFrameHeader {
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
            gateway_entry_identity: gateway.identity,
            ingress: BytecodeRequestIngressFrameHeader {
                protocol: BytecodeRequestIngressProtocol::Http,
                method: "POST".to_string(),
                path: VCP_PATH.to_string(),
            },
        },
        client_session: None,
        deadline: None,
        trace: BytecodeRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: format!("span-{request_id}"),
            parent_span_id: None,
            sampled: None,
        },
        http_request: BytecodeHttpRequestFrameHeader {
            method: "POST".to_string(),
            url: format!("http://phase-5.invalid{VCP_PATH}"),
            path: VCP_PATH.to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: false,
        test_case_capability: None,
        test_case_parent_request_id: None,
    };
    let request_frame = encode_binary_frame(&request_header, upstream.base_url().as_bytes())
        .expect("encode canonical request.start");
    websocket
        .send(Message::Binary(request_frame.into()))
        .await
        .expect("send canonical request.start");

    assert!(
        upstream.wait_for_path_async("/request", IO_TIMEOUT).await,
        "RuntimeHost never dispatched the pinned unary HTTP target"
    );
    upstream.release("/request");
    for path in ["/stream/left", "/stream/right"] {
        assert!(
            upstream
                .wait_for_response_head_async(path, IO_TIMEOUT)
                .await,
            "RuntimeHost never opened exact stream target {path}"
        );
    }
    let active_health = next_health(&mut websocket).await;
    assert_eq!(
        active_health.outbound_stream_leases_active, 2,
        "both response heads are Ready but the request table does not own two stream leases"
    );
    upstream.release("/stream/left");
    upstream.release("/stream/right");

    let start_bytes = next_binary_of_type(&mut websocket, "response.start").await;
    let start = decode_response_start_frame(&start_bytes)
        .expect("RuntimeHost emitted canonical response.start");
    assert_eq!(start.request_id, request_id);
    let mut response_chunks = Vec::new();
    let mut expected_seq = 0_u64;
    loop {
        let bytes = next_response_binary(&mut websocket, request_id).await;
        let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&bytes)
            .expect("decode RuntimeHost response envelope");
        match typed.envelope_type.as_str() {
            "response.chunk" => {
                let (header, payload) = decode_response_chunk_frame(&bytes)
                    .expect("RuntimeHost emitted canonical response.chunk");
                assert_eq!(header.request_id, request_id);
                assert_eq!(header.seq, expected_seq, "response chunk sequence gap");
                expected_seq += 1;
                response_chunks.push(payload);
            }
            "response.end" => {
                let (header, payload) = decode_response_end_frame(&bytes)
                    .expect("RuntimeHost emitted canonical response.end");
                assert_eq!(header.request_id, request_id);
                assert!(!header.payload_present);
                assert!(payload.is_empty());
                assert_eq!(header.metadata, ResponseEndFrameMetadata::None);
                break;
            }
            other => panic!("unexpected RuntimeHost response frame {other}"),
        }
    }
    assert!(expected_seq > 0, "serverStream emitted no chunks");
    let terminal_health = loop {
        let counters = next_health(&mut websocket).await;
        if health_counters_all_zero(&counters) {
            break counters;
        }
    };
    websocket.close(None).await.expect("close RuntimeHost peer");
    drop(host_task);

    TopLevelEvidence {
        response_status: start.http_response.status,
        response_headers: start
            .http_response
            .headers
            .into_iter()
            .map(|header| (header.name, header.value))
            .collect(),
        response_chunks,
        outbound: upstream.snapshot(),
        active_health,
        terminal_health,
    }
}

async fn next_health<S>(websocket: &mut WebSocketStream<S>) -> RuntimeHealthCountersFrameHeader
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let bytes = next_binary(websocket).await;
    let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&bytes)
        .expect("decode RuntimeHost health envelope");
    assert_eq!(
        typed.envelope_type, "runtime.health",
        "unexpected RuntimeHost frame while waiting for owner inventory"
    );
    decode_runtime_health_frame(&bytes)
        .expect("RuntimeHost emitted canonical runtime.health")
        .counters
}

fn health_counters_all_zero(counters: &RuntimeHealthCountersFrameHeader) -> bool {
    counters.outbound_requests_pending == 0
        && counters.outbound_stream_leases_active == 0
        && counters.stream_runtime_streams_active == 0
        && counters.flag_backed_cancel_waiters_active == 0
        && counters.task_requests_active == 0
}

async fn next_binary_of_type<S>(websocket: &mut WebSocketStream<S>, wanted: &str) -> Vec<u8>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let bytes = next_binary(websocket).await;
        let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&bytes)
            .expect("decode RuntimeHost typed frame");
        if typed.envelope_type == wanted {
            return bytes;
        }
        assert_eq!(
            typed.envelope_type, "runtime.health",
            "unexpected RuntimeHost frame before {wanted}"
        );
    }
}

async fn next_response_binary<S>(websocket: &mut WebSocketStream<S>, request_id: &str) -> Vec<u8>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let bytes = next_binary(websocket).await;
        let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&bytes)
            .expect("decode RuntimeHost typed frame");
        match typed.envelope_type.as_str() {
            "runtime.health" => continue,
            "response.error" => panic!(
                "RuntimeHost emitted response.error for {request_id}: {:?}",
                typed.rest
            ),
            "response.chunk" | "response.end" => return bytes,
            other => panic!("unexpected RuntimeHost frame {other} for {request_id}"),
        }
    }
}

async fn next_binary<S>(websocket: &mut WebSocketStream<S>) -> Vec<u8>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = tokio::time::timeout(IO_TIMEOUT, websocket.next())
            .await
            .expect("timed out waiting for RuntimeHost WebSocket frame")
            .expect("RuntimeHost WebSocket closed before response")
            .expect("read RuntimeHost WebSocket frame");
        match message {
            Message::Binary(bytes) => return bytes.to_vec(),
            Message::Ping(payload) => websocket
                .send(Message::Pong(payload))
                .await
                .expect("reply to RuntimeHost ping"),
            Message::Pong(_) => {}
            Message::Close(frame) => panic!("RuntimeHost closed WebSocket early: {frame:?}"),
            Message::Text(text) => panic!("RuntimeHost emitted forbidden text frame: {text}"),
            Message::Frame(_) => {}
        }
    }
}

struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct RuntimeHome(PathBuf);

impl RuntimeHome {
    fn new(prefix: &str) -> Self {
        let ordinal = NEXT_RUNTIME_HOME.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p5-r1-runtime-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create Phase 5 runtime home");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for RuntimeHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Default)]
struct TestDbCapabilityFactory;

impl DbCapabilityFactory for TestDbCapabilityFactory {
    fn context_for_request(&self, _owner: String, _request_id: String) -> DbCapabilityContext {
        DbCapabilityContext::unavailable()
    }
}

#[derive(Clone, Default)]
struct TestDbProviderFactory;

impl DbProviderFactory for TestDbProviderFactory {
    fn build(&self, _input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        Ok(DbCapabilitySource::new(Some(TestDbCapabilityFactory)))
    }
}
