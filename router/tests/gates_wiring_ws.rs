//! E-ws production assembly tests: WS gateway surface + method catalog,
//! connect admission correlation, inbound JSON-RPC dispatch and the
//! runtime-disconnect consumer wrapper (fake runtime seams; the real-socket
//! path is covered by `gates_wiring_http_seam` and the E-ws live gate).

mod ws_harness;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentRevision, GatewayEntryIdentity, ServiceDeploymentRef,
};
use skiff_router::dispatch::RuntimeAdmissionPool;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::SessionConsumer;
use skiff_router::supervisor::ws::{
    ConnectOutcome, WsBinding, WsConnectionRecord, WsDispatchStore, WsGatewaySurfaceView,
    WsInboundDispatch, WsLaneHandle, WsLaneSessionConsumer, WsMethodBinding, WsMethodCatalog,
    WsSessionWriter,
};
use skiff_router::ws::{
    AttachMeta, DispatchInbound, InboundDispatchAction, InboundExecutionToken, MethodCatalog,
    OverflowPolicy, PeerWriter, WebSocketLane, WebSocketLaneOptions, WebSocketRequestBrokerOptions,
};
use skiff_runtime_transport::connection_protocol::{OpaquePeerId, WebSocketRpcProfile};
use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_runtime_transport::protocol::{
    decode_bytecode_request_start_frame, BytecodeRequestNameValueFrameHeader,
    BytecodeWebSocketConnectIngressProtocol, BytecodeWebSocketJsonRpcResponseOutcome,
};
use tokio::sync::watch;

use ws_harness::{
    runtime_session, FakeDispatchInbound, FakeMethodCatalog, FakePeerWriter,
    FakeRuntimeViolationSink,
};

mod dispatch_harness;

fn websocket_entry() -> String {
    format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64))
}

fn entry_identity(tag: &str) -> GatewayEntryIdentity {
    GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", sha_digest(tag)))
        .expect("gateway entry identity")
}

fn sha_digest(seed: &str) -> String {
    // Deterministic 64-hex digest for test identities (no crypto required).
    let mut digest = String::new();
    for byte in seed.bytes().chain(std::iter::repeat(0)) {
        digest.push_str(&format!("{byte:02x}"));
        if digest.len() >= 64 {
            break;
        }
    }
    while digest.len() < 64 {
        digest.push('0');
    }
    digest
}

fn build_id() -> String {
    format!(
        "skiff-deployment-artifact-v4:sha256:{}",
        sha_digest("deployment")
    )
}

fn binding() -> WsBinding {
    WsBinding {
        service_id: "example.com/chat".to_string(),
        deployment: ServiceDeploymentRef {
            service_id: "example.com/chat".to_string(),
            contract_version: "example.com/chat@1".to_string(),
            deployment_revision: DeploymentRevision::new("1"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                "skiff-deployment-artifact-v4:sha256:{}",
                sha_digest("deployment")
            )),
        },
        gateway_entry_identity: entry_identity("connect"),
        websocket_entry_id: websocket_entry(),
        path: "/ws".to_string(),
        connect_handler: true,
        close_handler: false,
        methods: BTreeMap::from([(
            "chat.send".to_string(),
            WsMethodBinding {
                method: "chat.send".to_string(),
                gateway_entry_identity: entry_identity("chat.send"),
            },
        )]),
    }
}

fn surface() -> Arc<WsGatewaySurfaceView> {
    Arc::new(WsGatewaySurfaceView::from_bindings(BTreeMap::from([(
        ("example.com/chat".to_string(), "/ws".to_string()),
        binding(),
    )])))
}

#[derive(Debug, Default)]
struct FakeWsSessionWriter {
    frames: Arc<Mutex<Vec<WrittenFrame>>>,
}

type WrittenFrame = (RuntimeSessionEpoch, Vec<u8>);

impl WsSessionWriter for FakeWsSessionWriter {
    fn write(&self, runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
        self.frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((runtime.clone(), bytes));
        Ok(())
    }
}

/// Writer that always fails: models the runtime already disconnected (the
/// close notification must be swallowed, not propagated).
#[derive(Debug, Default)]
struct FailingWsSessionWriter;

impl WsSessionWriter for FailingWsSessionWriter {
    fn write(&self, _runtime: &RuntimeSessionEpoch, _bytes: Vec<u8>) -> Result<(), String> {
        Err("runtime disconnected".to_string())
    }
}

fn lane() -> Arc<WebSocketLane> {
    WebSocketLane::new(
        WebSocketLaneOptions {
            index: skiff_router::ws::ClientConnectionIndexOptions {
                connection_limit: 8,
                slow_client_budget_bytes: 1024 * 1024,
                high_water_capacity: 8,
            },
            broker: WebSocketRequestBrokerOptions {
                inbound_timeout_ms: 1000,
                ..Default::default()
            },
            ..Default::default()
        },
        Arc::new(FakeMethodCatalog::new()),
        Arc::new(skiff_router::ws::NoopNotificationObserver),
        Arc::new(FakeRuntimeViolationSink::new()),
        Arc::new(FakeDispatchInbound::new()),
    )
}

fn lane_with_dispatch(dispatch: Arc<dyn DispatchInbound>) -> Arc<WebSocketLane> {
    WebSocketLane::new(
        WebSocketLaneOptions {
            index: skiff_router::ws::ClientConnectionIndexOptions {
                connection_limit: 8,
                slow_client_budget_bytes: 1024 * 1024,
                high_water_capacity: 8,
            },
            broker: WebSocketRequestBrokerOptions {
                inbound_timeout_ms: 1000,
                ..Default::default()
            },
            ..Default::default()
        },
        Arc::new(FakeMethodCatalog::new()),
        Arc::new(skiff_router::ws::NoopNotificationObserver),
        Arc::new(FakeRuntimeViolationSink::new()),
        dispatch,
    )
}

fn attach_peer(
    lane: &Arc<WebSocketLane>,
    id: &str,
    runtime: &RuntimeSessionEpoch,
) -> Arc<FakePeerWriter> {
    let fake = Arc::new(FakePeerWriter::new());
    lane.reserve(id).expect("reserve");
    let _ = lane.admit(id, None, None, 1, OverflowPolicy::CloseOldest);
    let writer: Arc<dyn PeerWriter> = fake.clone();
    lane.attach(
        id,
        1,
        id.to_string(),
        runtime.clone(),
        writer,
        AttachMeta {
            service_id: "example.com/chat".to_string(),
            websocket_entry_id: websocket_entry(),
            profile: WebSocketRpcProfile::JsonRpc2_0Text,
        },
    )
    .expect("attach");
    fake
}

fn store_with<W: WsSessionWriter + 'static>(
    lane: &Arc<WebSocketLane>,
    writer: &Arc<W>,
) -> Arc<WsDispatchStore> {
    let handle = WsLaneHandle::new();
    let store = WsDispatchStore::new(
        handle.clone(),
        writer.clone(),
        RuntimeAdmissionPool::new(4),
        1000,
    );
    handle.set(Arc::clone(lane));
    store
}

fn record(connection_id: &str, runtime: &RuntimeSessionEpoch) -> WsConnectionRecord {
    WsConnectionRecord {
        connection_id: connection_id.to_string(),
        runtime: runtime.clone(),
        binding: binding(),
        business_identity: None,
        build_id: build_id(),
    }
}

fn inbound_action(connection_id: &str) -> InboundDispatchAction {
    let (_, cancel_rx) = watch::channel(false);
    InboundDispatchAction {
        profile: WebSocketRpcProfile::JsonRpc2_0Text,
        connection_id: connection_id.to_string(),
        socket_generation: 1,
        peer_id: OpaquePeerId::String("1".to_string()),
        method: "chat.send".to_string(),
        params: br#"{"message":"hi"}"#.to_vec(),
        execution_token: InboundExecutionToken {
            connection_id: connection_id.to_string(),
            socket_generation: 1,
            sequence: 1,
        },
        cancel: cancel_rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn surface_resolves_bindings_and_method_catalog_accepts_union() {
        let surface = surface();
        let resolved = surface
            .resolve("example.com/chat", "/ws")
            .expect("binding resolves");
        assert!(resolved.connect_handler);
        assert_eq!(resolved.methods.len(), 1);
        assert!(surface.resolve("example.com/chat", "/other").is_none());
        let catalog = WsMethodCatalog::new(Arc::clone(&surface));
        assert!(catalog.accepts("chat.send"));
        assert!(!catalog.accepts("chat.unknown"));
    }

    #[tokio::test]
    async fn connect_admission_writes_websocket_connect_frame_and_settles() {
        let runtime = runtime_session("runtime-a");
        let ws_lane = lane();
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&ws_lane, &writer);
        let (request_id, mut rx) = store
            .connect_begin(
                "wsconn-1",
                &binding(),
                &runtime,
                &build_id(),
                &skiff_router::supervisor::ws::WsConnectMetadata {
                    url: "ws://127.0.0.1/ws".to_string(),
                    query: vec![BytecodeRequestNameValueFrameHeader {
                        name: "user".to_string(),
                        value: "alice".to_string(),
                    }],
                    headers: Vec::new(),
                    cookies: Vec::new(),
                },
                1000,
            )
            .expect("connect begin");
        let frames = writer.frames.lock().unwrap().clone();
        assert_eq!(frames.len(), 1);
        let (header, payload) =
            decode_bytecode_request_start_frame(&frames[0].1).expect("connect frame decodes");
        let skiff_runtime_transport::protocol::BytecodeRequestStartFrameWireHeader::WebSocketConnect(connect) = header else {
            panic!("expected websocketConnect request.start");
        };
        assert_eq!(connect.request_id, request_id);
        assert_eq!(connect.websocket_connect.connection_id, "wsconn-1");
        assert_eq!(
            connect.websocket_connect.websocket_entry_id.as_str(),
            websocket_entry()
        );
        assert!(payload.is_empty());

        store.connect_response(
            &request_id,
            ConnectOutcome::Accepted {
                business_identity: Some("alice".to_string()),
                admission_rank: None,
                max_connections: 1,
                overflow: OverflowPolicy::CloseOldest,
                close_code: None,
                close_reason: None,
            },
        );
        rx.changed().await.expect("connect settles");
        assert!(matches!(
            rx.borrow_and_update().as_ref(),
            Some(ConnectOutcome::Accepted { .. })
        ));
        assert_eq!(store.pending_connect_count(), 0);
    }

    #[tokio::test]
    async fn connect_response_error_settles_unavailable_through_request_sink() {
        use skiff_router::dispatch::{RequestDispatcher, RuntimeDispatcherOptions};
        use skiff_router::supervisor::http::{PendingHttpRouter, RequestFrameSink};
        use skiff_runtime_transport::protocol::{
            encode_binary_frame, ResponseErrorFrameHeader, RuntimeErrorFramePayload,
        };

        let runtime = runtime_session("runtime-a");
        let ws_lane = lane();
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&ws_lane, &writer);
        let (request_id, mut rx) = store
            .connect_begin(
                "wsconn-error-1",
                &binding(),
                &runtime,
                &build_id(),
                &skiff_router::supervisor::ws::WsConnectMetadata {
                    url: "ws://127.0.0.1/ws".to_string(),
                    query: Vec::new(),
                    headers: Vec::new(),
                    cookies: Vec::new(),
                },
                1000,
            )
            .expect("connect begin");

        let candidate =
            dispatch_harness::FakeCandidateViewSource::new(vec![dispatch_harness::session_state(
                "s1",
                "runtime-a",
                1,
            )]);
        let options = RuntimeDispatcherOptions::new(
            4,
            Arc::new(candidate),
            Arc::new(dispatch_harness::FakeLeaseRevalidate::new()),
            Arc::new(dispatch_harness::FakeRuntimePeer::new()),
            Arc::new(dispatch_harness::FakeSessionAbort::new()),
        )
        .expect("options");
        let dispatcher = Arc::new(RequestDispatcher::new(options).expect("dispatcher"));
        let sink = RequestFrameSink::new_with_ws(
            Arc::clone(&dispatcher),
            Arc::new(PendingHttpRouter::new()),
            Some(store.clone()),
        );

        let header = ResponseErrorFrameHeader::control(
            request_id.clone(),
            RuntimeErrorFramePayload {
                code: "UnhandledServiceError".to_string(),
                message: "unhandled request-local user exception".to_string(),
                status: None,
                details: None,
            },
        );
        let bytes = encode_binary_frame(&header, &[]).expect("encode response.error");
        use skiff_router::session::InboundFrameSink;
        sink.handle(&runtime, &bytes)
            .expect("sink accepts response.error");

        rx.changed().await.expect("connect settles on error");
        assert!(matches!(
            rx.borrow_and_update().as_ref(),
            Some(ConnectOutcome::Unavailable { .. })
        ));
        assert_eq!(store.pending_connect_count(), 0);
    }

    #[tokio::test]
    async fn inbound_dispatch_writes_jsonrpc_frame_and_completes_to_peer_writer() {
        let runtime = runtime_session("runtime-a");
        let writer = Arc::new(FakeWsSessionWriter::default());
        let handle = WsLaneHandle::new();
        let store = WsDispatchStore::new(
            handle.clone(),
            writer.clone(),
            RuntimeAdmissionPool::new(4),
            1000,
        );
        let ws_lane = lane_with_dispatch(Arc::new(WsInboundDispatch::new(Arc::clone(&store))));
        handle.set(Arc::clone(&ws_lane));
        store.register_connection(record("wsconn-1", &runtime));
        let peer = attach_peer(&ws_lane, "wsconn-1", &runtime);
        // The broker owns the inbound entry/token; the store's DispatchInbound is
        // invoked through the real peer-text path.
        let request_frame =
            br#"{"jsonrpc":"2.0","id":"1","method":"chat.send","params":{"message":"hi"}}"#;
        let second_frame =
            br#"{"jsonrpc":"2.0","id":"2","method":"chat.send","params":{"message":"hi"}}"#;
        assert!(ws_lane.handle_peer_text("wsconn-1", second_frame).is_none());
        let frames = writer.frames.lock().unwrap().clone();
        assert_eq!(frames.len(), 1);
        let (header, payload) =
            decode_bytecode_request_start_frame(&frames[0].1).expect("jsonrpc frame decodes");
        let skiff_runtime_transport::protocol::BytecodeRequestStartFrameWireHeader::WebSocketJsonRpc(jsonrpc) = header else {
            panic!("expected websocketJsonRpc request.start");
        };
        assert_eq!(jsonrpc.websocket_json_rpc.connection_id, "wsconn-1");
        // Business params stay lexical-opaque: the broker hands the raw
        // `params` value slice (TS `losslessJsonSlice` parity; the runtime
        // codec requires the plain JSON value, not the member including the
        // key). Synced with the E-ws broker span fix.
        assert_eq!(payload, br#"{"message":"hi"}"#);
        assert_eq!(store.pending_inbound_count(), 1);

        store.on_inbound_response(
            &jsonrpc.request_id,
            BytecodeWebSocketJsonRpcResponseOutcome::Success,
            br#""ok""#.to_vec(),
        );
        assert_eq!(store.pending_inbound_count(), 0);
        let writes = peer.writes();
        assert_eq!(
            writes.len(),
            1,
            "peer must receive the result frame: {writes:?}"
        );
        assert!(writes[0].contains(r#""result":"ok""#), "{}", writes[0]);

        // Terminal mapping: deadline outcome aborts the inbound entry.
        assert!(ws_lane
            .handle_peer_text("wsconn-1", request_frame)
            .is_none());
        assert_eq!(store.pending_inbound_count(), 1);
        let frames = writer.frames.lock().unwrap().clone();
        let (header, _) =
            decode_bytecode_request_start_frame(&frames[1].1).expect("jsonrpc frame decodes");
        let skiff_runtime_transport::protocol::BytecodeRequestStartFrameWireHeader::WebSocketJsonRpc(jsonrpc) = header else {
            panic!("expected websocketJsonRpc request.start");
        };
        store.on_inbound_response(
            &jsonrpc.request_id,
            BytecodeWebSocketJsonRpcResponseOutcome::DeadlineExceeded,
            Vec::new(),
        );
        assert_eq!(store.pending_inbound_count(), 0);
        let writes = peer.writes();
        assert!(
            writes.last().unwrap().contains(r#""code":-32001"#),
            "{}",
            writes.last().unwrap()
        );
    }

    #[tokio::test]
    async fn runtime_disconnect_fails_pending_and_finishes_connections() {
        let runtime = runtime_session("runtime-a");
        let ws_lane = lane();
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&ws_lane, &writer);
        store.register_connection(record("wsconn-1", &runtime));
        let peer = attach_peer(&ws_lane, "wsconn-1", &runtime);
        let (_request_id, mut rx) = store
            .connect_begin(
                "wsconn-1",
                &binding(),
                &runtime,
                &build_id(),
                &Default::default(),
                1000,
            )
            .expect("connect begin");
        let dispatch = WsInboundDispatch::new(Arc::clone(&store));
        dispatch
            .dispatch(inbound_action("wsconn-1"))
            .expect("inbound dispatch accepted");
        assert_eq!(store.pending_connect_count(), 1);
        assert_eq!(store.pending_inbound_count(), 1);

        let consumer = WsLaneSessionConsumer::new(
            Arc::clone(&ws_lane),
            Arc::clone(&store),
            Arc::clone(&ws_lane.broker)
                as Arc<dyn skiff_router::session::consumer::SessionConsumer>,
        );
        consumer
            .on_session_closed(&runtime)
            .expect("consumer cleanup");
        rx.changed().await.expect("connect resolves");
        assert!(matches!(
            rx.borrow_and_update().as_ref(),
            Some(ConnectOutcome::Unavailable { .. })
        ));
        assert_eq!(store.pending_connect_count(), 0);
        assert_eq!(store.pending_inbound_count(), 0);
        assert_eq!(store.pinned_connection_count(), 0);

        // The lane finalizer must finish the attached client connection.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while ws_lane.snapshot().connection_count != 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "connection did not drain"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let _ = peer;
        assert!(ws_lane.snapshot().open_connections.is_empty());
    }

    #[tokio::test]
    async fn connect_reject_and_unavailable_fail_closed() {
        let runtime = runtime_session("runtime-a");
        let ws_lane = lane();
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&ws_lane, &writer);
        let (request_id, mut rx) = store
            .connect_begin(
                "wsconn-1",
                &binding(),
                &runtime,
                &build_id(),
                &Default::default(),
                1000,
            )
            .expect("connect begin");
        store.connect_response(
            &request_id,
            ConnectOutcome::Rejected {
                code: 1008,
                reason: "denied".to_string(),
            },
        );
        rx.changed().await.expect("connect settles");
        assert!(matches!(
            rx.borrow_and_update().as_ref(),
            Some(ConnectOutcome::Rejected { .. })
        ));

        let (request_id, mut rx) = store
            .connect_begin(
                "wsconn-2",
                &binding(),
                &runtime,
                &build_id(),
                &Default::default(),
                1000,
            )
            .expect("connect begin");
        store.connect_unavailable(&request_id, "timeout".to_string());
        rx.changed().await.expect("connect settles");
        assert!(matches!(
            rx.borrow_and_update().as_ref(),
            Some(ConnectOutcome::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn unregister_connection_writes_close_frame_when_close_handler_declared() {
        let runtime = runtime_session("runtime-a");
        let ws_lane = lane();
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&ws_lane, &writer);
        let mut record = record("wsconn-1", &runtime);
        record.binding.close_handler = true;
        record.business_identity = Some("alice".to_string());
        store.register_connection(record);

        store.unregister_connection("wsconn-1");

        let frames = writer.frames.lock().unwrap().clone();
        assert_eq!(frames.len(), 1, "exactly one close frame");
        assert_eq!(
            frames[0].0, runtime,
            "close frame targets the pinned runtime"
        );
        let (header, payload) =
            decode_bytecode_request_start_frame(&frames[0].1).expect("close frame decodes");
        let skiff_runtime_transport::protocol::BytecodeRequestStartFrameWireHeader::WebSocketConnectionClosed(close) = header else {
            panic!("expected connectionClosed request.start");
        };
        assert_eq!(close.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(close.frame_type, "request.start");
        assert_eq!(close.mode, "unary");
        assert_eq!(close.caller.kind, "gateway");
        assert_eq!(close.routing.kind, "runtimeAssembly");
        assert_eq!(close.routing.assembly_identity, None);
        assert_eq!(close.routing.assembly_generation, None);
        assert_eq!(close.routing.deployment, binding().deployment);
        assert_eq!(close.routing.build_id.as_deref(), Some(build_id().as_str()));
        assert_eq!(
            close.routing.gateway_entry_identity,
            entry_identity("connect")
        );
        assert_eq!(
            close.routing.ingress.protocol,
            BytecodeWebSocketConnectIngressProtocol::WebSocket
        );
        assert_eq!(close.routing.ingress.path, "/ws");
        assert_eq!(close.routing.ingress.entry_kind, "connectionClosed");
        assert_eq!(close.client_session, None);
        assert_eq!(close.deadline, None);
        assert!(close.trace.trace_id.starts_with("ws-trace-"));
        assert!(!close.test_effects_enabled);
        assert_eq!(close.websocket_connection_closed.connection_id, "wsconn-1");
        assert_eq!(
            close
                .websocket_connection_closed
                .websocket_entry_id
                .as_str(),
            websocket_entry()
        );
        assert_eq!(
            close.websocket_connection_closed.gateway_entry_identity,
            entry_identity("connect")
        );
        assert_eq!(
            close
                .websocket_connection_closed
                .business_identity
                .as_deref(),
            Some("alice")
        );
        assert_eq!(close.websocket_connection_closed.close_code, None);
        assert_eq!(close.websocket_connection_closed.close_reason, None);
        assert!(payload.is_empty(), "close notification carries no payload");

        // Exactly-once: a second unregister finds no record and writes nothing.
        store.unregister_connection("wsconn-1");
        assert_eq!(writer.frames.lock().unwrap().len(), 1);
        assert_eq!(store.pinned_connection_count(), 0);
    }

    #[tokio::test]
    async fn unregister_connection_without_close_handler_writes_no_frame() {
        let runtime = runtime_session("runtime-a");
        let ws_lane = lane();
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&ws_lane, &writer);
        store.register_connection(record("wsconn-1", &runtime));

        store.unregister_connection("wsconn-1");

        assert!(
            writer.frames.lock().unwrap().is_empty(),
            "no close frame without a declared close handler"
        );
        assert_eq!(store.pinned_connection_count(), 0);
    }

    #[tokio::test]
    async fn unregister_connection_swallows_write_failure_when_runtime_gone() {
        let runtime = runtime_session("runtime-a");
        let ws_lane = lane();
        let store = store_with(&ws_lane, &Arc::new(FailingWsSessionWriter::default()));
        let mut record = record("wsconn-1", &runtime);
        record.binding.close_handler = true;
        store.register_connection(record);

        // The runtime is already gone (writer fails): the close notification
        // is dropped and the teardown still completes without an error.
        store.unregister_connection("wsconn-1");

        assert_eq!(store.pinned_connection_count(), 0);
    }
}
