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
    decode_runtime_capabilities_frame, decode_typed_binary_frame, encode_binary_frame,
    encode_router_bootstrap_frame, encode_runtime_registered_frame, BytecodeHttpRequestFrameHeader,
    BytecodeRequestCallerFrameHeader, BytecodeRequestIngressFrameHeader,
    BytecodeRequestIngressProtocol, BytecodeRequestRoutingFrameHeader,
    BytecodeRequestStartFrameHeader, BytecodeRequestTraceFrameHeader, ResponseEndFrameMetadata,
    RouterBootstrapActivationFrameHeader, RouterBootstrapFrameHeader,
    RouterBootstrapHttpFrameHeader, RouterBootstrapServiceDbFrameHeader,
    RuntimeDispatchModeCapability, RuntimeRegisteredFrameHeader, TypedEnvelope,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::Instant,
};
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

use super::fixture::PublishedFixture;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_RUNTIME_HOME: AtomicU64 = AtomicU64::new(0);

pub(super) struct RuntimeHostHarness {
    fixture: PublishedFixture,
    websocket: WebSocketStream<TcpStream>,
    _host_task: AbortOnDrop,
    _runtime_home: RuntimeHome,
}

pub(super) struct HostResponse {
    pub status: u16,
    pub chunks: Vec<Vec<u8>>,
}

impl RuntimeHostHarness {
    pub async fn start(prefix: &str, fixture: PublishedFixture) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind Phase 6 RuntimeHost peer");
        let router_address = listener.local_addr().expect("RuntimeHost peer address");
        let runtime_home = RuntimeHome::new(prefix);
        let host = RuntimeHost::new(RuntimeConfig {
            db_provider: DbProviderSource::new(TestDbProviderFactory),
            router_url: format!("ws://{router_address}"),
            base_runtime_id: format!("runtime-phase-6-{prefix}"),
            runtime_home: runtime_home.path().to_path_buf(),
            profile: "skiff-test".to_string(),
            bytecode_only: true,
            http_response_max_bytes: 64 * 1024,
            http_egress_proxy: None,
        })
        .expect("construct production RuntimeHost");
        let host_task = AbortOnDrop(Some(tokio::spawn(async move {
            host.run_forever()
                .await
                .expect("production RuntimeHost run loop")
        })));

        let websocket = accept_registered_session(&listener, &fixture).await;
        Self {
            fixture,
            websocket,
            _host_task: host_task,
            _runtime_home: runtime_home,
        }
    }

    pub async fn send_http_request(&mut self, request_id: &str, ingress_path: &str, body: &[u8]) {
        let gateway = self.fixture.gateway(ingress_path);
        let header = BytecodeRequestStartFrameHeader {
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
                deployment: self.fixture.deployment.clone(),
                build_id: Some(
                    self.fixture
                        .deployment
                        .deployment_artifact_identity
                        .as_str()
                        .to_string(),
                ),
                gateway_entry_identity: gateway.identity,
                ingress: BytecodeRequestIngressFrameHeader {
                    protocol: BytecodeRequestIngressProtocol::Http,
                    method: "POST".to_string(),
                    path: ingress_path.to_string(),
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
                url: format!("http://phase-6.invalid{ingress_path}"),
                path: ingress_path.to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
            test_effects_enabled: false,
            test_case_capability: None,
            test_case_parent_request_id: None,
        };
        let frame = encode_binary_frame(&header, body).expect("encode canonical request.start");
        self.websocket
            .send(Message::Binary(frame.into()))
            .await
            .expect("send canonical request.start");
    }

    pub async fn response(&mut self, request_id: &str) -> HostResponse {
        let deadline = Instant::now() + IO_TIMEOUT;
        let start_bytes =
            next_binary_of_type(&mut self.websocket, "response.start", deadline).await;
        let start = decode_response_start_frame(&start_bytes)
            .expect("RuntimeHost emitted canonical response.start");
        assert_eq!(start.request_id, request_id);

        let mut chunks = Vec::new();
        let mut expected_seq = 0_u64;
        loop {
            let bytes = next_binary(&mut self.websocket, deadline, request_id).await;
            let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&bytes)
                .expect("decode RuntimeHost response envelope");
            match typed.envelope_type.as_str() {
                "response.chunk" => {
                    let (header, payload) = decode_response_chunk_frame(&bytes)
                        .expect("RuntimeHost emitted canonical response.chunk");
                    assert_eq!(header.request_id, request_id);
                    assert_eq!(header.seq, expected_seq, "response chunk sequence gap");
                    expected_seq += 1;
                    chunks.push(payload);
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
        HostResponse {
            status: start.http_response.status,
            chunks,
        }
    }

    pub async fn close(mut self) {
        self.websocket
            .close(None)
            .await
            .expect("close RuntimeHost peer");
        self._host_task.stop().await;
    }
}

async fn accept_registered_session(
    listener: &TcpListener,
    fixture: &PublishedFixture,
) -> WebSocketStream<TcpStream> {
    let (socket, _) = tokio::time::timeout(IO_TIMEOUT, listener.accept())
        .await
        .expect("RuntimeHost did not connect to its Router peer")
        .expect("accept RuntimeHost TCP connection");
    let mut websocket = accept_async(socket)
        .await
        .expect("accept RuntimeHost WebSocket");
    websocket
        .send(Message::Binary(
            encode_router_bootstrap_frame(&RouterBootstrapFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "router.bootstrap".to_string(),
                artifacts_path: fixture.artifact_root.display().to_string(),
                service_db: RouterBootstrapServiceDbFrameHeader {
                    mongo_url: "mongodb://127.0.0.1:1/phase6-proof".to_string(),
                },
                http: RouterBootstrapHttpFrameHeader {
                    max_response_bytes: 64 * 1024,
                },
                activation: RouterBootstrapActivationFrameHeader {
                    profile: "skiff-test".to_string(),
                },
            })
            .expect("encode production router.bootstrap")
            .into(),
        ))
        .await
        .expect("send router.bootstrap");

    let deadline = Instant::now() + IO_TIMEOUT;
    let capabilities_bytes =
        next_binary_of_type(&mut websocket, "runtime.capabilities", deadline).await;
    let capabilities = decode_runtime_capabilities_frame(&capabilities_bytes)
        .expect("decode production runtime.capabilities");
    let canonical_artifact_root = fs::canonicalize(&fixture.artifact_root)
        .expect("canonicalize published Phase 6 artifact root");
    assert_eq!(
        capabilities.capabilities.artifact_root.as_deref(),
        Some(canonical_artifact_root.to_string_lossy().as_ref())
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
    websocket
}

async fn next_binary_of_type(
    websocket: &mut WebSocketStream<TcpStream>,
    wanted: &str,
    deadline: Instant,
) -> Vec<u8> {
    loop {
        let bytes = next_binary(websocket, deadline, wanted).await;
        let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&bytes)
            .expect("decode RuntimeHost typed frame");
        if typed.envelope_type == wanted {
            return bytes;
        }
        assert!(
            matches!(
                typed.envelope_type.as_str(),
                "runtime.capabilities" | "runtime.health"
            ),
            "unexpected RuntimeHost frame {} before {wanted}",
            typed.envelope_type
        );
    }
}

async fn next_binary(
    websocket: &mut WebSocketStream<TcpStream>,
    deadline: Instant,
    context: &str,
) -> Vec<u8> {
    loop {
        let message = tokio::time::timeout_at(deadline, websocket.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for RuntimeHost {context}"))
            .expect("RuntimeHost WebSocket closed before terminal")
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

struct AbortOnDrop(Option<JoinHandle<()>>);

impl AbortOnDrop {
    async fn stop(&mut self) {
        let Some(handle) = self.0.take() else {
            return;
        };
        handle.abort();
        match handle.await {
            Err(error) if error.is_cancelled() => {}
            result => panic!("RuntimeHost task did not stop by harness cancellation: {result:?}"),
        }
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

struct RuntimeHome(PathBuf);

impl RuntimeHome {
    fn new(prefix: &str) -> Self {
        let ordinal = NEXT_RUNTIME_HOME.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p6-r1-runtime-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create Phase 6 runtime home");
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
