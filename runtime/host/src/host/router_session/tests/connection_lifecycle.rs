use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use skiff_runtime_capability_context::{
    CancellationSource, ConnectionRequestSession, ConnectionRequestTerminal, ExecutionScope,
};
use skiff_runtime_request::OutboundResponse;
use tokio::{
    io::duplex,
    sync::{mpsc, oneshot, Notify},
};
use tokio_tungstenite::{
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame, Role},
        Message,
    },
    WebSocketStream,
};

use super::*;

const ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:close-test";

#[tokio::test]
async fn router_close_completes_handshake_and_session_without_waiting_for_transport_eof() {
    let host = test_host();
    let session = ConnectionRequestSession::new(ROUTER_SESSION).expect("session");
    let cancellation = CancellationSource::new();
    let scope = ExecutionScope::request(cancellation.token(), None);
    let mut connection_pending = host
        .connection_requests
        .install(session, scope, Arc::new(|_, _| Ok(())))
        .expect("pending connection request");
    let (outbound_sender, mut outbound_receiver) = mpsc::unbounded_channel();
    let _outbound_lease = host
        .outbound_requests
        .insert_with_lease(
            "pending-across-router-close".to_string(),
            outbound_sender,
            None,
            "caller_cancel",
        )
        .expect("pending outbound request");
    let mut actor_method_pending = host
        .actor_method_outbound
        .register(
            "actor-method-across-router-close".to_string(),
            "cancel-actor-method-across-router-close".to_string(),
            1,
            skiff_artifact_model::ActorImplementationIdentity::new(format!(
                "skiff-actor-implementation-v1:sha256:{}",
                "a".repeat(64)
            )),
        )
        .expect("pending actor method request");

    let (client_io, server_io) = duplex(4096);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let release_router = Arc::new(Notify::new());
    let router_released = Arc::clone(&release_router);
    let (close_reply_sender, close_reply_receiver) = oneshot::channel();
    let router_task = tokio::spawn(async move {
        router
            .send(Message::Ping(b"router-heartbeat".to_vec().into()))
            .await
            .expect("mock Router ping");
        assert_eq!(
            router
                .next()
                .await
                .expect("pong frame")
                .expect("valid pong"),
            Message::Pong(b"router-heartbeat".to_vec().into())
        );
        let close = CloseFrame {
            code: CloseCode::Away,
            reason: "router restart".into(),
        };
        router
            .send(Message::Close(Some(close.clone())))
            .await
            .expect("mock Router close");
        assert_eq!(
            router
                .next()
                .await
                .expect("close handshake reply")
                .expect("valid close reply"),
            Message::Close(Some(close))
        );
        close_reply_sender
            .send(())
            .expect("report close handshake reply");
        router_released.notified().await;
    });
    let mut session_task = tokio::spawn(run_connected_session(
        host.clone(),
        client,
        ROUTER_SESSION.to_string(),
    ));

    close_reply_receiver
        .await
        .expect("mock Router observed close handshake reply");
    let bounded_scheduler_turns = async {
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
    };
    tokio::pin!(bounded_scheduler_turns);
    let session_result = tokio::select! {
        biased;
        result = &mut session_task => result,
        () = &mut bounded_scheduler_turns => {
            panic!("Router Close must terminate the session before transport EOF")
        }
    };
    session_result
        .expect("session task")
        .expect("clean Router Close");
    assert_eq!(
        connection_pending.wait().await,
        ConnectionRequestTerminal::TransportUnavailable
    );
    let outbound = outbound_receiver
        .recv()
        .await
        .expect("outbound request connection-closed error");
    assert!(matches!(
        outbound,
        OutboundResponse::Error(error)
            if error.code == "ConnectionClosed"
                && error.message == "router connection closed"
    ));
    let actor_method_error = actor_method_pending
        .receive()
        .await
        .expect("actor method pending receiver")
        .expect_err("actor method must fail on Router close");
    assert_eq!(actor_method_error.code, "ConnectionClosed");
    assert_eq!(actor_method_error.message, "router connection closed");
    assert_eq!(host.connection_requests.pending_count(), 0);
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert_eq!(host.actor_method_outbound.pending_count(), 0);

    release_router.notify_one();
    router_task.await.expect("mock Router task");
}

#[tokio::test]
async fn transport_eof_terminates_session_and_fails_pending_control_request() {
    let host = test_host();
    let (outbound_sender, mut outbound_receiver) = mpsc::unbounded_channel();
    let _outbound_lease = host
        .outbound_requests
        .insert_with_lease(
            "pending-across-transport-eof".to_string(),
            outbound_sender,
            None,
            "caller_cancel",
        )
        .expect("pending outbound request");
    let (client_io, server_io) = duplex(4096);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;

    let session_task = tokio::spawn(run_connected_session(
        host.clone(),
        client,
        "skiff-router-session-v1:opaque:eof-test".to_string(),
    ));
    drop(router);

    session_task
        .await
        .expect("session task")
        .expect_err("transport EOF without Close must remain a transport error");
    let outbound = outbound_receiver
        .recv()
        .await
        .expect("outbound request connection-closed error");
    assert!(matches!(
        outbound,
        OutboundResponse::Error(error)
            if error.code == "ConnectionClosed"
                && error.message == "router connection closed"
    ));
    assert_eq!(host.outbound_requests.pending_count(), 0);
}
