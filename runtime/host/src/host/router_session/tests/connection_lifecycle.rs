use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use skiff_runtime_capability_context::{
    CancellationSource, ConnectionRequestSession, ConnectionRequestTerminal, ExecutionScope,
};
use skiff_runtime_request::OutboundResponse;
use skiff_runtime_transport::protocol::TaskRef;
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

#[tokio::test(flavor = "current_thread")]
async fn dropping_owned_session_children_synchronously_releases_actor_test_owners() {
    const SESSION: &str = "skiff-router-session-v1:opaque:owned-child-drop";
    let host = test_host();
    host.open_actor_instance_session(SESSION).unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:owned-child-drop",
            SESSION,
            "root:owned-child-drop".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let mut invoke = actor_owner_test_invoke(
        "actor:owned-child-drop",
        "case:owned-child-drop",
        "root:owned-child-drop",
    );
    invoke.invoke.trace_id = Some("skiff-test:pending-after-actor-owner-admission".to_string());
    let (sender, _receiver) = mpsc::unbounded_channel();
    let task = host
        .begin_actor_owner_invoke(SESSION.to_string(), invoke, Vec::new(), sender)
        .unwrap();
    let mut children = RouterSessionChildTasks::default();
    children.push(task);

    assert!(host
        .actor_owner_invocations
        .contains("actor:owned-child-drop"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(SESSION, "actor:owned-child-drop")
        .is_some());
    drop(children);

    assert!(!host
        .actor_owner_invocations
        .contains("actor:owned-child-drop"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(SESSION, "actor:owned-child-drop")
        .is_none());
    root.finalize().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn owned_session_child_panic_unwinds_actor_test_owners_without_detaching() {
    const SESSION: &str = "skiff-router-session-v1:opaque:owned-child-panic";
    let host = test_host();
    host.open_actor_instance_session(SESSION).unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:owned-child-panic",
            SESSION,
            "root:owned-child-panic".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let mut invoke = actor_owner_test_invoke(
        "actor:owned-child-panic",
        "case:owned-child-panic",
        "root:owned-child-panic",
    );
    invoke.invoke.trace_id = Some("skiff-test:panic-after-actor-owner-admission".to_string());
    let (sender, _receiver) = mpsc::unbounded_channel();
    let task = host
        .begin_actor_owner_invoke(SESSION.to_string(), invoke, Vec::new(), sender)
        .unwrap();
    let mut children = RouterSessionChildTasks::default();
    children.push(task);
    children.next().await;

    assert!(children.is_empty());
    assert!(!host
        .actor_owner_invocations
        .contains("actor:owned-child-panic"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(SESSION, "actor:owned-child-panic")
        .is_none());
    root.finalize().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn real_connected_session_close_waits_for_owned_actor_child_drop() {
    const SESSION: &str = "skiff-router-session-v1:opaque:owned-child-close";
    let host = test_host();
    let (client_io, server_io) = duplex(4096);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let session_task = tokio::spawn(run_connected_session_with_bootstrap(
        host.clone(),
        client,
        SESSION.to_string(),
        Some(test_connection_bootstrap("owned-child-close").unwrap()),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !host.actor_instances.is_session_open(SESSION) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("connected session opens its Actor generation");
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:owned-child-close",
            SESSION,
            "root:owned-child-close".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let mut invoke = actor_owner_test_invoke(
        "actor:owned-child-close",
        "case:owned-child-close",
        "root:owned-child-close",
    );
    invoke.invoke.trace_id = Some("skiff-test:pending-after-actor-owner-admission".to_string());
    let frame = encode_actor_owner_invoke_frame(&invoke, &[]).unwrap();
    router.send(Message::Binary(frame.into())).await.unwrap();
    crate::host::actor_owner_execution::pending_actor_owner_after_admission_barrier()
        .wait()
        .await;

    router.send(Message::Close(None)).await.unwrap();
    assert!(matches!(
        router.next().await.unwrap().unwrap(),
        Message::Close(_)
    ));
    session_task.await.unwrap().unwrap();

    assert!(!host
        .actor_owner_invocations
        .contains("actor:owned-child-close"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(SESSION, "actor:owned-child-close")
        .is_none());
    assert!(host.actor_instances.store().is_empty());
    assert!(!host.actor_instances.is_session_open(SESSION));
    root.finalize().await.unwrap();
}

#[tokio::test]
async fn aborting_real_connected_session_runs_raii_teardown() {
    const SESSION: &str = "skiff-router-session-v1:opaque:abort-raii";
    let host = test_host();
    let (client_io, server_io) = duplex(4096);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let _router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let session_task = tokio::spawn(run_connected_session(
        host.clone(),
        client,
        SESSION.to_string(),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !host.actor_instances.is_session_open(SESSION) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("real connected session opens its Actor generation");

    let root = host
        .test_http_entries
        .begin_root_case(
            "case:abort-raii",
            SESSION,
            "root:abort-raii".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let (outbound_sender, mut outbound_receiver) = mpsc::unbounded_channel();
    let _outbound_lease = host
        .outbound_requests
        .insert_with_lease(
            "pending-across-session-abort".to_string(),
            outbound_sender,
            None,
            "caller_cancel",
        )
        .unwrap();

    session_task.abort();
    assert!(session_task.await.unwrap_err().is_cancelled());
    assert!(!host.actor_instances.is_session_open(SESSION));
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert!(matches!(
        outbound_receiver.recv().await,
        Some(OutboundResponse::Error(error)) if error.code == "ConnectionClosed"
    ));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(SESSION, "root:abort-raii")
        .is_none());
    drop(root);
}

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
    assert!(!host.actor_instances.is_session_open(ROUTER_SESSION));

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
    assert!(!host
        .actor_instances
        .is_session_open("skiff-router-session-v1:opaque:eof-test"));
}

#[tokio::test]
async fn authority_error_ends_real_session_before_later_success_receipt_is_read() {
    const SESSION: &str = "skiff-router-session-v1:opaque:authority-stop";
    let host = test_host();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:authority-stop",
            SESSION,
            "root:authority-stop".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let (outbound_sender, mut outbound_receiver) = mpsc::unbounded_channel();
    let _outbound_lease = host
        .outbound_requests
        .insert_with_lease(
            "rpc-after-authority-error".to_string(),
            outbound_sender,
            None,
            "caller_cancel",
        )
        .expect("pending outbound request");
    let invalid_actor = encode_actor_owner_invoke_frame(
        &actor_owner_test_invoke(
            "actor:stale-session-parent",
            "case:authority-stop",
            "missing:parent",
        ),
        &[],
    )
    .unwrap();
    let later_success = encode_binary_frame(
        &TaskSubmitResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "task.submit.response".to_string(),
            rpc_id: "rpc-after-authority-error".to_string(),
            task_ref: TaskRef::new("task-must-not-complete", "example.com/service")
                .expect("task ref"),
            task_id: "task-must-not-complete".to_string(),
            request_id: "request-must-not-complete".to_string(),
            status: "submitted".to_string(),
        },
        &[],
    )
    .unwrap();

    let (client_io, server_io) = duplex(16 * 1024);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    router
        .feed(Message::Binary(invalid_actor.into()))
        .await
        .expect("queue invalid Actor frame");
    router
        .feed(Message::Binary(later_success.into()))
        .await
        .expect("queue later success receipt");
    router.flush().await.expect("flush both Router frames");

    let error = super::run_connected_session_with_bootstrap(
        host.clone(),
        client,
        SESSION.to_string(),
        Some(super::test_connection_bootstrap("authority-stop").unwrap()),
    )
    .await
    .expect_err("authority failure must end the Router session");
    assert!(error.to_string().contains("parent request is unknown"));
    let outbound = outbound_receiver
        .recv()
        .await
        .expect("session cleanup must fail the untouched pending request");
    assert!(matches!(
        outbound,
        OutboundResponse::Error(error)
            if error.code == "ConnectionClosed"
                && error.message == "router connection closed"
    ));
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert!(!host
        .actor_owner_invocations
        .contains("actor:stale-session-parent"));
    assert!(!host.actor_instances.is_session_open(SESSION));
    root.finalize().await.unwrap();
}

fn pending_initial_activation_control(
    route: &crate::loader::assembly_admission::ActiveAssemblyRoute,
    request_id: &str,
    capability: &str,
    parent_request_id: &str,
    actor_id: &str,
) -> (
    skiff_runtime_transport::actor_owner::ActorOwnerControlFrameHeader,
    String,
) {
    use base64::Engine as _;
    use sha2::{Digest as _, Sha256};
    use skiff_runtime_linked_program::{FileAddr, UnitAddr};
    use skiff_runtime_transport::{
        actor_method::{
            ActorDeclarationOwnerFrameHeader, ActorMethodDeadlineFrameHeader,
            ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
        },
        actor_owner::{
            ActorActivationBootstrapFrameHeader, ActorOwnerControlFenceFrameHeader,
            ActorOwnerControlFrameHeader, ActorOwnerControlOperation, ACTOR_BOOTSTRAP_ENCODING_V1,
            ACTOR_OWNER_CONTROL_FRAME_TYPE,
        },
    };

    let declaration = route
        .execution_image()
        .execution_packages()
        .iter()
        .flat_map(|package| package.files())
        .flat_map(|file| &file.actor_declarations)
        .find(|declaration| declaration.actor_type.symbol == "Counter")
        .expect("current-scope fixture must contain Counter");
    let implementation_owner = declaration
        .implementation_owner
        .as_ref()
        .expect("Counter must have an exact linked implementation owner");
    let owner = ActorDeclarationOwnerFrameHeader {
        unit: match implementation_owner.unit {
            UnitAddr::Service => ActorOwnerUnitFrameHeader::Service,
            UnitAddr::Package(slot) => ActorOwnerUnitFrameHeader::Package(
                u64::try_from(slot).expect("package slot must fit the wire representation"),
            ),
        },
        file: match &implementation_owner.file {
            FileAddr::LoadedFileIndex(index) => ActorOwnerFileFrameHeader::LoadedFileIndex(
                u64::try_from(*index).expect("file index must fit the wire representation"),
            ),
            FileAddr::FileIrIdentity(identity) => {
                ActorOwnerFileFrameHeader::FileIrIdentity(identity.clone())
            }
        },
        actor_symbol: implementation_owner.actor_symbol.clone(),
    };
    let actor_id_key = serde_json::to_vec(actor_id).unwrap();
    let actor_id_hash = format!("sha256:{}", hex::encode(Sha256::digest(&actor_id_key)));
    let control = ActorOwnerControlFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.to_string(),
        target_runtime_id: "runtime-base".to_string(),
        request_id: request_id.to_string(),
        operation: ActorOwnerControlOperation::ActivateInitial,
        route_authority: ActorOwnerRouteAuthorityFrameHeader {
            build_id: route
                .deployment()
                .deployment_artifact_identity
                .as_str()
                .to_string(),
        },
        fence: ActorOwnerControlFenceFrameHeader {
            service_id: route.deployment().service_id.clone(),
            actor_type_identity: declaration.actor_type.symbol_path(),
            actor_id_type_identity: "builtin:string".to_string(),
            actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
            canonical_actor_id_key_bytes_base64: base64::engine::general_purpose::STANDARD
                .encode(&actor_id_key),
            actor_id_hash: actor_id_hash.clone(),
            epoch: 1,
            actor_abi_identity: declaration.actor_abi_identity.clone(),
            actor_implementation_identity: declaration.actor_implementation_identity.clone(),
            declaration_owner: owner,
            owner_lease_id: "lease:pending-create".to_string(),
            eviction_request_id: None,
        },
        transition: None,
        bootstrap: Some(ActorActivationBootstrapFrameHeader {
            encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
            payload_base64: base64::engine::general_purpose::STANDARD.encode(b"[]"),
        }),
        deadline: Some(ActorMethodDeadlineFrameHeader {
            timeout_ms: 30_000,
            expires_at: "2099-01-01T00:00:00Z".to_string(),
        }),
        test_case_capability: Some(capability.to_string()),
        test_case_parent_request_id: Some(parent_request_id.to_string()),
    };
    (control, actor_id_hash)
}

#[tokio::test(flavor = "current_thread")]
async fn pending_test_aware_activate_initial_is_dropped_with_exact_session_and_reconnects_cleanly()
{
    use skiff_runtime_transport::actor_owner::{
        encode_actor_owner_control_frame, ActorOwnerControlAckFrameHeader,
    };

    const SESSION: &str = "skiff-router-session-v1:opaque:pending-test-create";
    const FIRST_CAPABILITY: &str = "case:pending-test-create";
    const FIRST_ROOT: &str = "root:pending-test-create";
    const FIRST_CREATE: &str = "actor:create:pending-test-create";
    const SECOND_CAPABILITY: &str = "case:reconnected-test-create";
    const SECOND_ROOT: &str = "root:reconnected-test-create";
    const SECOND_CREATE: &str = "actor:create:reconnected-test-create";

    let (host, routes) =
        super::runtime_assembly_request::fixture::admitted_current_scope_gateway_host().await;
    let route = routes
        .values()
        .find(|route| route.deployment().service_id == "example.com/current-scope-consumer")
        .expect("current-scope consumer route")
        .clone();
    let (first_control, actor_id_hash) = pending_initial_activation_control(
        &route,
        FIRST_CREATE,
        FIRST_CAPABILITY,
        FIRST_ROOT,
        "pending-create",
    );
    let gate =
        skiff_runtime_eval::actor_executor::install_actor_create_test_gate(actor_id_hash, false);

    let (client_io, server_io) = duplex(16 * 1024);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let session_task = tokio::spawn(run_connected_session_with_bootstrap(
        host.clone(),
        client,
        SESSION.to_string(),
        Some(test_connection_bootstrap("pending-test-create").unwrap()),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !host.actor_instances.is_session_open(SESSION) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first Router session must open");
    let first_root = host
        .test_http_entries
        .begin_root_case(
            FIRST_CAPABILITY,
            SESSION,
            FIRST_ROOT.to_string(),
            route.activation().activation_id().as_str().to_string(),
            "http://127.0.0.1:44100/test-case",
            route.deployment().clone(),
        )
        .unwrap();
    router
        .send(Message::Binary(
            encode_actor_owner_control_frame(&first_control)
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), gate.wait_entered())
        .await
        .expect("ActivateInitial must reach the real Actor create body");
    assert_eq!(host.actor_instances.store().len(), 1);
    assert_eq!(host.actor_instances.tracked_owner_count_for_test(), 1);
    assert!(host
        .test_http_entries
        .self_ingress_for_request(SESSION, FIRST_CREATE)
        .is_some());

    router.send(Message::Close(None)).await.unwrap();
    let mut accepted_first_ack = false;
    while let Some(message) = tokio::time::timeout(std::time::Duration::from_secs(1), router.next())
        .await
        .expect("first session close handshake must finish")
    {
        match message.unwrap() {
            Message::Binary(bytes) => {
                let decoded: std::result::Result<(ActorOwnerControlAckFrameHeader, Vec<u8>), _> =
                    skiff_runtime_transport::protocol::decode_typed_binary_frame(&bytes);
                if let Ok((ack, payload)) = decoded {
                    assert!(payload.is_empty());
                    if ack.request_id == FIRST_CREATE && ack.accepted {
                        accepted_first_ack = true;
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    session_task.await.unwrap().unwrap();
    assert!(
        !accepted_first_ack,
        "a cancelled create must never be accepted"
    );
    assert!(!host.actor_instances.is_session_open(SESSION));
    assert!(host.actor_instances.store().is_empty());
    assert_eq!(host.actor_instances.tracked_owner_count_for_test(), 0);
    assert!(host
        .test_http_entries
        .self_ingress_for_request(SESSION, FIRST_CREATE)
        .is_none());
    tokio::time::timeout(std::time::Duration::from_secs(1), first_root.finalize())
        .await
        .expect("cancelled create root must finalize after exact lease cleanup")
        .unwrap();

    let (second_control, second_hash) = pending_initial_activation_control(
        &route,
        SECOND_CREATE,
        SECOND_CAPABILITY,
        SECOND_ROOT,
        "pending-create",
    );
    assert_eq!(
        second_hash, first_control.fence.actor_id_hash,
        "reconnect must retry the exact Actor incarnation"
    );
    let (client_io, server_io) = duplex(16 * 1024);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let second_session_task = tokio::spawn(run_connected_session_with_bootstrap(
        host.clone(),
        client,
        SESSION.to_string(),
        Some(test_connection_bootstrap("reconnected-test-create").unwrap()),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !host.actor_instances.is_session_open(SESSION) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("same-id Router session must reopen with a fresh generation");
    let second_root = host
        .test_http_entries
        .begin_root_case(
            SECOND_CAPABILITY,
            SESSION,
            SECOND_ROOT.to_string(),
            route.activation().activation_id().as_str().to_string(),
            "http://127.0.0.1:44100/test-case",
            route.deployment().clone(),
        )
        .unwrap();
    router
        .send(Message::Binary(
            encode_actor_owner_control_frame(&second_control)
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    let second_ack = loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), router.next())
            .await
            .expect("reconnected ActivateInitial must answer")
            .expect("reconnected Router session must stay open")
            .unwrap();
        if let Message::Binary(bytes) = message {
            let decoded: std::result::Result<(ActorOwnerControlAckFrameHeader, Vec<u8>), _> =
                skiff_runtime_transport::protocol::decode_typed_binary_frame(&bytes);
            if let Ok((ack, payload)) = decoded {
                assert!(payload.is_empty());
                if ack.request_id == SECOND_CREATE {
                    break ack;
                }
            }
        }
    };
    assert!(
        second_ack.accepted,
        "fresh same-id session must not inherit stale pending ownership: {:?}",
        second_ack.reason
    );
    assert_eq!(host.actor_instances.store().len(), 1);
    assert_eq!(host.actor_instances.tracked_owner_count_for_test(), 1);
    tokio::time::timeout(std::time::Duration::from_secs(1), second_root.finalize())
        .await
        .expect("reconnected create root must finalize after ACK")
        .unwrap();

    router.send(Message::Close(None)).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while let Some(message) = router.next().await {
            if matches!(message.unwrap(), Message::Close(_)) {
                break;
            }
        }
    })
    .await
    .expect("reconnected session close handshake must finish");
    second_session_task.await.unwrap().unwrap();
    assert!(host.actor_instances.store().is_empty());
    assert_eq!(host.actor_instances.tracked_owner_count_for_test(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn activate_initial_pins_exact_retained_generation_after_reload_and_fails_closed_when_missing(
) {
    use skiff_runtime_transport::actor_owner::{
        encode_actor_owner_control_frame, ActorOwnerControlAckFrameHeader,
    };

    const SESSION: &str = "skiff-router-session-v1:opaque:generation-pin";
    const HANG_CAPABILITY: &str = "case:generation-hang";
    const HANG_ROOT: &str = "root:generation-hang";
    const HANG_CREATE: &str = "actor:create:generation-hang";
    const CHILD_CAPABILITY: &str = "case:generation-child";
    const CHILD_ROOT: &str = "root:generation-child";
    const CHILD_CREATE: &str = "actor:create:generation-child";
    const MISSING_GENERATION: &str = "actor:create:missing-generation";
    const MISMATCHED_IDENTITY: &str = "actor:create:mismatched-identity";

    let (host, pinned, assembly, resolver) =
        super::runtime_assembly_request::fixture::current_scope_gateway_host_for_reload().await;
    let pinned_build_id = pinned.deployment().deployment_artifact_identity.as_str();

    // G1 work hangs while the same buildId is admitted again. The Host must
    // keep resolving further buildId-authority work on the exact image held
    // by that work.
    let (hang_control, hang_hash) = pending_initial_activation_control(
        &pinned,
        HANG_CREATE,
        HANG_CAPABILITY,
        HANG_ROOT,
        "generation-hang",
    );
    let hang_gate =
        skiff_runtime_eval::actor_executor::install_actor_create_test_gate(hang_hash, false);
    let (child_control, child_hash) = pending_initial_activation_control(
        &pinned,
        CHILD_CREATE,
        CHILD_CAPABILITY,
        CHILD_ROOT,
        "generation-child",
    );
    let child_gate =
        skiff_runtime_eval::actor_executor::install_actor_create_test_gate(child_hash, false);
    assert_eq!(
        hang_control.route_authority.build_id, pinned_build_id,
        "authority must carry the deployment buildId"
    );
    assert_eq!(child_control.route_authority.build_id, pinned_build_id);

    let (client_io, server_io) = duplex(16 * 1024);
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let mut router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let session_task = tokio::spawn(run_connected_session_with_bootstrap(
        host.clone(),
        client,
        SESSION.to_string(),
        Some(test_connection_bootstrap("generation-pin").unwrap()),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !host.actor_instances.is_session_open(SESSION) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pinned generation Router session must open");
    let hang_root = host
        .test_http_entries
        .begin_root_case(
            HANG_CAPABILITY,
            SESSION,
            HANG_ROOT.to_string(),
            pinned.activation().activation_id().as_str().to_string(),
            "http://127.0.0.1:44100/test-case",
            pinned.deployment().clone(),
        )
        .unwrap();
    let child_root = host
        .test_http_entries
        .begin_root_case(
            CHILD_CAPABILITY,
            SESSION,
            CHILD_ROOT.to_string(),
            pinned.activation().activation_id().as_str().to_string(),
            "http://127.0.0.1:44100/test-case",
            pinned.deployment().clone(),
        )
        .unwrap();

    async fn next_ack(
        router: &mut WebSocketStream<tokio::io::DuplexStream>,
        request_id: &str,
    ) -> ActorOwnerControlAckFrameHeader {
        loop {
            let message = tokio::time::timeout(std::time::Duration::from_secs(5), router.next())
                .await
                .expect("ActivateInitial must answer")
                .expect("Router session must stay open")
                .unwrap();
            if let Message::Binary(bytes) = message {
                let decoded: std::result::Result<(ActorOwnerControlAckFrameHeader, Vec<u8>), _> =
                    skiff_runtime_transport::protocol::decode_typed_binary_frame(&bytes);
                if let Ok((ack, payload)) = decoded {
                    assert!(payload.is_empty());
                    if ack.request_id == request_id {
                        return ack;
                    }
                }
            }
        }
    }

    router
        .send(Message::Binary(
            encode_actor_owner_control_frame(&hang_control)
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), hang_gate.wait_entered())
        .await
        .expect("G1 create must reach the deterministic gate");
    assert!(host
        .actor_route_holds
        .find(pinned_build_id)
        .is_some());

    // Re-admit the same buildId while the create is still hung. The exact
    // image is retained only by the live work, so a new child control with
    // the same buildId must still execute on the retained image rather than
    // silently moving to the re-admitted one.
    host.assembly_admission
        .admit(Arc::clone(&assembly), &resolver)
        .await
        .expect("current-scope re-admission should admit");
    router
        .send(Message::Binary(
            encode_actor_owner_control_frame(&child_control)
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), child_gate.wait_entered())
        .await
        .expect("G1 child create must reach the deterministic gate");
    hang_gate.release();
    child_gate.release();
    let hang_ack = next_ack(&mut router, HANG_CREATE).await;
    assert!(
        hang_ack.accepted,
        "hung G1 create must complete on the exact retained generation: {:?}",
        hang_ack.reason
    );
    let child_ack = next_ack(&mut router, CHILD_CREATE).await;
    assert!(
        child_ack.accepted,
        "G1 child must execute on the exact retained generation after G2 reload: {:?}",
        child_ack.reason
    );

    let mut missing = hang_control.clone();
    missing.request_id = MISSING_GENERATION.to_string();
    missing.route_authority.build_id =
        format!("skiff-deployment-artifact-v4:sha256:{}", "9".repeat(64));
    router
        .send(Message::Binary(
            encode_actor_owner_control_frame(&missing).unwrap().into(),
        ))
        .await
        .unwrap();
    let rejected = next_ack(&mut router, MISSING_GENERATION).await;
    assert!(
        !rejected.accepted,
        "missing retained buildId must fail closed: {:?}",
        rejected.reason
    );

    let mut mismatched = child_control.clone();
    mismatched.request_id = MISMATCHED_IDENTITY.to_string();
    mismatched.route_authority.build_id =
        format!("skiff-deployment-artifact-v4:sha256:{}", "0".repeat(64));
    router
        .send(Message::Binary(
            encode_actor_owner_control_frame(&mismatched)
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    let rejected_identity = next_ack(&mut router, MISMATCHED_IDENTITY).await;
    assert!(
        !rejected_identity.accepted,
        "mismatched buildId must fail closed: {:?}",
        rejected_identity.reason
    );

    hang_root.finalize().await.unwrap();
    child_root.finalize().await.unwrap();
    router.send(Message::Close(None)).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while let Some(message) = router.next().await {
            if matches!(message.unwrap(), Message::Close(_)) {
                break;
            }
        }
    })
    .await
    .expect("pinned generation session close handshake must finish");
    session_task.await.unwrap().unwrap();
    assert!(host.actor_instances.store().is_empty());
    assert_eq!(host.actor_instances.tracked_owner_count_for_test(), 0);
}
