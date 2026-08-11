//! H-registration-cut full-loop handshake tests.
//!
//! Drives the production Runtime session loop (`run_connected_session`) over
//! an in-memory duplex WebSocket pair, exercising the frozen §3.5 handshake
//! from the Runtime side: bootstrap -> capabilities -> registered ACK ->
//! health. Wrong order, identity change, duplicate bootstrap, legacy inbound
//! frames, business frames before the ACK and deadlines are strict terminals
//! (C-model-registration §2.3).

use std::path::PathBuf;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use skiff_runtime_transport::{
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, RuntimeCapabilitiesFrameHeader,
        RuntimeRegisteredFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    },
};
use tokio::{
    io::duplex,
    time::{timeout, Duration},
};
use tokio_tungstenite::{
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame, Role},
        Message,
    },
    WebSocketStream,
};

const RUNTIME_ID: &str = "runtime-a";
const SESSION: &str = "skiff-router-session-v1:opaque:h-registration-cut";

struct HandshakeSession {
    host: crate::host::RuntimeHost,
    router: WebSocketStream<tokio::io::DuplexStream>,
    session_task: tokio::task::JoinHandle<crate::error::Result<()>>,
    artifact_root: PathBuf,
}

impl HandshakeSession {
    async fn start(label: &str) -> Self {
        let artifact_root = super::runtime_assembly_request::fixture::registration_fixture();

        let host = crate::host::RuntimeHost::new(crate::host::RuntimeConfig {
            db_provider: super::test_db_provider(),
            router_url: "ws://127.0.0.1:4001/runtime".to_string(),
            base_runtime_id: RUNTIME_ID.to_string(),
            runtime_home: std::env::temp_dir().join(format!(
                "skiff-runtime-h-registration-cut-{label}-{}",
                uuid::Uuid::new_v4()
            )),
            profile: "test".to_string(),
            bytecode_only: false,
            http_response_max_bytes: 1024,
            http_egress_proxy: None,
        })
        .expect("h-registration-cut test host");
        let (client_io, server_io) = duplex(1 << 20);
        let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        let session_task = tokio::spawn(super::super::run_connected_session_with_deadlines(
            host.clone(),
            client,
            SESSION.to_string(),
            None,
            super::super::handshake::HandshakeDeadlines {
                bootstrap: Duration::from_millis(100),
                registered: Duration::from_millis(200),
            },
        ));
        Self {
            host,
            router,
            session_task,
            artifact_root,
        }
    }

    fn bootstrap_frame(&self) -> Vec<u8> {
        encode_binary_frame(
            &json!({
                "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
                "type": "router.bootstrap",
                "artifactsPath": self.artifact_root,
                "serviceDb": { "mongoUrl": "mongodb://h-registration-cut" },
                "activation": {
                    "profile": "test"
                },
                "http": { "maxResponseBytes": 1024 }
            }),
            &[],
        )
        .expect("bootstrap frame")
    }

    async fn send_bootstrap(&mut self) {
        let frame = self.bootstrap_frame();
        self.router
            .send(Message::Binary(frame.into()))
            .await
            .expect("send bootstrap");
    }

    async fn recv_binary(&mut self, context: &str) -> Vec<u8> {
        timeout(Duration::from_secs(5), async {
            loop {
                let message = self
                    .router
                    .next()
                    .await
                    .expect("runtime websocket remains open")
                    .expect("valid runtime frame");
                if let Message::Binary(frame) = message {
                    return frame.to_vec();
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{context}: timed out waiting for a binary frame"))
    }

    async fn recv_capabilities(&mut self) -> RuntimeCapabilitiesFrameHeader {
        let frame = self.recv_binary("capabilities").await;
        let (header, _): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("capabilities frame");
        assert_eq!(header.envelope_type, "runtime.capabilities");
        header
    }

    async fn send_registered_ack(&mut self, runtime_id: &str) {
        let ack = encode_binary_frame(
            &RuntimeRegisteredFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "runtime.registered".to_string(),
                runtime_id: runtime_id.to_string(),
            },
            &[],
        )
        .expect("registered ACK frame");
        self.router
            .send(Message::Binary(ack.into()))
            .await
            .expect("send registered ACK");
    }

    async fn wait_for_session(mut self) -> crate::error::Result<()> {
        self.session_task
            .await
            .expect("session task must not panic")
    }

    async fn close(mut self) -> crate::error::Result<()> {
        let close = CloseFrame {
            code: CloseCode::Normal,
            reason: "test done".into(),
        };
        self.router
            .send(Message::Close(Some(close)))
            .await
            .expect("send Router close");
        self.wait_for_session().await
    }
}

#[tokio::test(flavor = "current_thread")]
async fn accept_sequence_registers_then_ack_then_health() {
    let mut session = HandshakeSession::start("accept").await;
    session.send_bootstrap().await;

    let capabilities = session.recv_capabilities().await;
    assert_eq!(capabilities.runtime_id, RUNTIME_ID);

    // Health must NOT be emitted before the registered ACK: nothing may be on
    // the wire between the capabilities and the ACK.
    assert!(
        timeout(
            Duration::from_millis(40),
            session.recv_binary("health must not precede the ACK")
        )
        .await
        .is_err(),
        "runtime must not send frames before the registered ACK"
    );

    session.send_registered_ack(RUNTIME_ID).await;
    let health = session.recv_binary("health after ACK").await;
    let (header, _): (
        skiff_runtime_transport::protocol::RuntimeHealthFrameHeader,
        Vec<u8>,
    ) = decode_typed_binary_frame(&health).expect("health frame");
    assert_eq!(header.envelope_type, "runtime.health");
    assert_eq!(header.runtime_id, RUNTIME_ID);

    session.close().await.expect("clean Router close");
}

#[tokio::test(flavor = "current_thread")]
async fn registered_ack_before_bootstrap_is_wrong_order_terminal() {
    let mut session = HandshakeSession::start("ack-before-bootstrap").await;
    session.send_registered_ack(RUNTIME_ID).await;
    let error = session
        .wait_for_session()
        .await
        .expect_err("ACK before bootstrap must terminate the session");
    assert!(
        error.to_string().contains("WrongOrder"),
        "unexpected error: {error:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn registered_ack_identity_change_is_terminal() {
    let mut session = HandshakeSession::start("ack-identity").await;
    session.send_bootstrap().await;
    let _ = session.recv_capabilities().await;
    session.send_registered_ack("runtime-other").await;
    let error = session
        .wait_for_session()
        .await
        .expect_err("mismatched ACK identity must terminate the session");
    assert!(
        error.to_string().contains("IdentityChange"),
        "unexpected error: {error:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn duplicate_bootstrap_is_wrong_order_terminal() {
    let mut session = HandshakeSession::start("duplicate-bootstrap").await;
    session.send_bootstrap().await;
    let _ = session.recv_capabilities().await;
    session.send_bootstrap().await;
    let error = session
        .wait_for_session()
        .await
        .expect_err("duplicate bootstrap must terminate the session");
    assert!(
        error.to_string().contains("WrongOrder"),
        "unexpected error: {error:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn business_frame_before_ack_is_wrong_order_terminal() {
    let mut session = HandshakeSession::start("business-before-ack").await;
    session.send_bootstrap().await;
    let _ = session.recv_capabilities().await;
    let request = encode_binary_frame(
        &json!({
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "request.start",
            "requestId": "pre-ack-business"
        }),
        &[],
    )
    .expect("business frame should encode");
    session
        .router
        .send(Message::Binary(request.into()))
        .await
        .expect("send business frame before ACK");
    let error = session
        .wait_for_session()
        .await
        .expect_err("business frame before ACK must terminate the session");
    assert!(
        error.to_string().contains("WrongOrder"),
        "unexpected error: {error:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn bootstrap_timeout_is_terminal() {
    let mut session = HandshakeSession::start("bootstrap-timeout").await;
    let error = session
        .wait_for_session()
        .await
        .expect_err("bootstrap timeout must terminate the session");
    assert!(
        error.to_string().contains("BootstrapTimeout"),
        "unexpected error: {error:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn disconnect_mid_handshake_is_terminal_without_health() {
    let mut session = HandshakeSession::start("disconnect").await;
    session.send_bootstrap().await;
    let _ = session.recv_capabilities().await;
    drop(session.router);
    let _ = session
        .session_task
        .await
        .expect("session task must not panic");
}
