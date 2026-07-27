use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    runtime_assembly_request::{
        decode_runtime_assembly_websocket_connect_response_end_frame,
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressProtocol,
        RuntimeAssemblyWebSocketConnectRequestFrameHeader,
        RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        RuntimeAssemblyWebSocketConnectResponseFrameHeader,
        RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
    },
    websocket_generation_lifecycle::{
        decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
        WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
        WebSocketGenerationLifecycleOperation, WebSocketGenerationLifecycleRejectionCode,
        WebSocketGenerationLifecycleSender, WebSocketGenerationLifecycleTuple,
        WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
    },
};
use std::sync::Arc;

use tokio::sync::mpsc;

use super::runtime_assembly_request::fixture;

const ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:test-session";
const OTHER_ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:other-session";
const CONNECTION_ID: &str = "connection-a";
const WEBSOCKET_ENTRY_ID: &str =
    "skiff-websocket-entry-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn websocket_generation_old_route_survives_reload_until_disconnect_without_artifact_io() {
    let (host, generation_a, generation_b) = fixture::reloaded_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    assert_eq!(generation_a.generation(), 1);
    assert_eq!(generation_b.generation(), 2);

    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.queue_websocket_generation_acquire_for_test(
        &generation_a,
        ROUTER_SESSION,
        WEBSOCKET_ENTRY_ID,
        CONNECTION_ID,
        &sender,
    )
    .expect("generation A connect should queue an acquire");
    let acquire = receive_lifecycle(&mut receiver).await;
    assert!(matches!(
        &acquire,
        WebSocketGenerationLifecycleControl::Acquire { .. }
    ));
    let ack_frame = encode_websocket_generation_lifecycle_frame(
        WebSocketGenerationLifecycleDirection::RouterToRuntime,
        &acquire_ack(&acquire),
    )
    .expect("exact acquire ack should encode");
    let mut control = None;
    let mut fingerprint = None;
    super::super::dispatch_router_binary_frame(
        &host,
        &ack_frame,
        &sender,
        &mut control,
        &mut fingerprint,
    )
    .await
    .expect("binary acquire ack should correlate");

    let duplicate_acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_a.clone(),
            WEBSOCKET_ENTRY_ID.to_string(),
            CONNECTION_ID.to_string(),
        )
        .expect("the same connection tuple should acquire idempotently");
    host.websocket_generations
        .handle_acquire_response(&acquire_ack(&duplicate_acquire))
        .expect("duplicate exact acquire ack should correlate");
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 1);

    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .expect("session disconnect should release all pins");
    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .expect("duplicate session disconnect should be idempotent");
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    assert!(host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_b,
            WEBSOCKET_ENTRY_ID.to_string(),
            "late-connection".to_string(),
        )
        .is_err());
}

#[tokio::test]
async fn websocket_generation_release_is_exact_idempotent_and_fail_closed() {
    let (host, generation_a, _) = fixture::reloaded_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let retired_context = Arc::downgrade(generation_a.context_set());
    let acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_a,
            WEBSOCKET_ENTRY_ID.to_string(),
            CONNECTION_ID.to_string(),
        )
        .expect("connect should pin");
    host.websocket_generations
        .handle_acquire_response(&acquire_ack(&acquire))
        .expect("exact acquire ack should correlate");
    let tuple = lifecycle_tuple(&acquire).clone();

    let mut mismatched_tuple = tuple.clone();
    mismatched_tuple.websocket_entry_id =
        format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64));
    assert!(matches!(
        host.websocket_generations
            .handle_release(
                ROUTER_SESSION,
                release_control("release-wrong-tuple", mismatched_tuple),
            )
            .expect("wrong tuple should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::TupleMismatch,
            ..
        }
    ));
    assert!(matches!(
        host.websocket_generations
            .handle_release(
                OTHER_ROUTER_SESSION,
                release_control("release-wrong-session", tuple.clone()),
            )
            .expect("wrong session should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
            ..
        }
    ));
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 1);

    let release = release_control("release-1", tuple.clone());
    let first = host
        .websocket_generations
        .handle_release(ROUTER_SESSION, release.clone())
        .expect("exact release should apply");
    assert!(matches!(
        first,
        WebSocketGenerationLifecycleControl::Ack {
            operation: WebSocketGenerationLifecycleOperation::Release,
            ..
        }
    ));
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    assert!(
        retired_context.upgrade().is_none(),
        "release should reclaim the retired generation after its last connection pin"
    );

    assert_eq!(
        host.websocket_generations
            .handle_release(ROUTER_SESSION, release)
            .expect("duplicate exact release should be idempotent"),
        first
    );
    assert!(matches!(
        host.websocket_generations
            .handle_release(
                OTHER_ROUTER_SESSION,
                release_control("release-1", lifecycle_tuple(&acquire).clone()),
            )
            .expect("cross-session replay should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
            ..
        }
    ));

    let mut conflicting_tuple = tuple;
    conflicting_tuple.websocket_entry_id =
        format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64));
    let conflict = release_control("release-1", conflicting_tuple);
    assert!(matches!(
        host.websocket_generations
            .handle_release(ROUTER_SESSION, conflict)
            .expect("request-id conflict should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::RequestConflict,
            ..
        }
    ));

    let mut valid_unacquired_tuple = lifecycle_tuple(&acquire).clone();
    valid_unacquired_tuple.service_id = "example.com/service".to_string();
    valid_unacquired_tuple.connection_id = "unacquired-connection".to_string();
    let unacquired = release_control("release-unacquired", valid_unacquired_tuple);
    let unacquired_frame = encode_websocket_generation_lifecycle_frame(
        WebSocketGenerationLifecycleDirection::RouterToRuntime,
        &unacquired,
    )
    .expect("valid unacquired release should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut fingerprint = None;
    super::super::dispatch_router_binary_frame(
        &host,
        &unacquired_frame,
        &sender,
        &mut control,
        &mut fingerprint,
    )
    .await
    .expect("unacquired release should receive a typed rejection");
    assert!(matches!(
        receive_lifecycle(&mut receiver).await,
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::NotAcquired,
            ..
        }
    ));
}

#[tokio::test]
async fn websocket_generation_acquire_rejection_rolls_back_the_route_pin() {
    let (host, generation_a, _) = fixture::reloaded_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_a,
            WEBSOCKET_ENTRY_ID.to_string(),
            CONNECTION_ID.to_string(),
        )
        .expect("connect should tentatively pin");
    let reject = acquire_rejection(&acquire);
    host.websocket_generations
        .handle_acquire_response(&reject)
        .expect("typed acquire rejection should be isolated to the connection");
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

mod websocket_jsonrpc_target {
    use super::*;

    #[tokio::test]
    async fn handlerless_method_websocket_eager_pins_before_accept_without_user_connect() {
        let (host, physical, _) = fixture::admitted_websocket_gateway_host().await;
        host.websocket_generations.connect(ROUTER_SESSION).unwrap();
        assert!(physical.entry().optional_handler().is_none());
        assert!(physical.has_websocket_jsonrpc_methods().unwrap());

        let header = handlerless_connect_header(&physical, "handlerless-method-connect");
        let frame = encode_binary_frame(&header, &[]).unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut control = None;
        let mut fingerprint = None;
        super::super::super::dispatch_router_binary_frame(
            &host,
            &frame,
            &sender,
            &mut control,
            &mut fingerprint,
        )
        .await
        .expect("handlerless method-bearing connect should enter Host admission");

        let acquire = receive_lifecycle(&mut receiver).await;
        assert!(
            receiver.try_recv().is_err(),
            "synthetic accept must wait for the exact acquire receipt"
        );
        host.websocket_generations
            .handle_acquire_response(&acquire_ack(&acquire))
            .expect("exact acquire receipt");

        let RouterWriterMessage::Binary(response) = receiver.recv().await.unwrap() else {
            panic!("synthetic accept must use the binary response wire")
        };
        let response = decode_runtime_assembly_websocket_connect_response_end_frame(&response)
            .expect("synthetic connect response");
        assert_eq!(response.request_id, header.request_id);
        assert!(matches!(
            response.websocket_connect,
            RuntimeAssemblyWebSocketConnectResponseFrameHeader::Accept {
                business_identity: None,
                connection_policy: None,
            }
        ));
        assert_eq!(host.websocket_generations.pin_count().unwrap(), 1);

        let release = release_control("handlerless-close", lifecycle_tuple(&acquire).clone());
        assert!(matches!(
            host.websocket_generations
                .handle_release(ROUTER_SESSION, release)
                .unwrap(),
            WebSocketGenerationLifecycleControl::Ack {
                operation: WebSocketGenerationLifecycleOperation::Release,
                ..
            }
        ));
        assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    }
}

#[tokio::test]
async fn websocket_jsonrpc_target_resolves_from_old_and_current_generation_pins() {
    let (host, physical_a, method_a, physical_b, method_b) =
        fixture::reloaded_websocket_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let websocket_entry_id = skiff_artifact_identity::websocket_entry_id(
        &physical_a.entry().owner().service_id,
        physical_a.gateway_entry_key(),
    )
    .unwrap();

    for (route, connection_id) in [
        (&physical_a, "old-generation-connection"),
        (&physical_b, "current-generation-connection"),
    ] {
        let acquire = host
            .websocket_generations
            .begin_acquire(
                ROUTER_SESSION,
                route.clone(),
                websocket_entry_id.as_str().to_string(),
                connection_id.to_string(),
            )
            .unwrap();
        host.websocket_generations
            .handle_acquire_response(&acquire_ack(&acquire))
            .unwrap();
    }

    let target_a = host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "old-generation-connection",
            physical_a.assembly_identity(),
            physical_a.generation(),
            &websocket_entry_id,
            &method_a.selector().host,
            &method_a.selector().path,
            method_a.selector().method.as_deref().unwrap(),
            method_a.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .expect("old connection resolves only from its pinned generation A");
    let target_b = host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "current-generation-connection",
            physical_b.assembly_identity(),
            physical_b.generation(),
            &websocket_entry_id,
            &method_b.selector().host,
            &method_b.selector().path,
            method_b.selector().method.as_deref().unwrap(),
            method_b.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .expect("new connection resolves from generation B");

    assert_eq!(target_a.assembly_generation(), 1);
    assert_eq!(target_b.assembly_generation(), 2);
    assert!(!Arc::ptr_eq(
        target_a.eval().activation_context(),
        target_b.eval().activation_context()
    ));
    assert_eq!(target_a.method(), "status.get");
    assert_eq!(target_b.method(), "status.get");
    assert!(host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "old-generation-connection",
            physical_a.assembly_identity(),
            physical_a.generation(),
            &websocket_entry_id,
            "wrong.test",
            &method_a.selector().path,
            "status.get",
            method_a.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .is_err());
    assert!(host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "old-generation-connection",
            physical_a.assembly_identity(),
            physical_a.generation(),
            &websocket_entry_id,
            &method_a.selector().host,
            "/wrong",
            "status.get",
            method_a.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .is_err());
    let wrong_websocket_entry_id = skiff_artifact_model::WebSocketEntryId::parse(format!(
        "skiff-websocket-entry-v1:sha256:{}",
        "e".repeat(64)
    ))
    .unwrap();
    assert!(host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "old-generation-connection",
            physical_a.assembly_identity(),
            physical_a.generation(),
            &wrong_websocket_entry_id,
            &method_a.selector().host,
            &method_a.selector().path,
            "status.get",
            method_a.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .is_err());
    assert!(host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "old-generation-connection",
            physical_a.assembly_identity(),
            physical_a.generation() + 1,
            &websocket_entry_id,
            &method_a.selector().host,
            &method_a.selector().path,
            "status.get",
            method_a.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .is_err());
    assert!(host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "old-generation-connection",
            physical_a.assembly_identity(),
            physical_a.generation(),
            &websocket_entry_id,
            &method_a.selector().host,
            &method_a.selector().path,
            "status.missing",
            method_a.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .is_err());
    assert!(host
        .websocket_generations
        .websocket_jsonrpc_target(
            ROUTER_SESSION,
            "old-generation-connection",
            physical_a.assembly_identity(),
            physical_a.generation(),
            &websocket_entry_id,
            &method_a.selector().host,
            &method_a.selector().path,
            "status.get",
            physical_a.gateway_entry_identity(),
            skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
        )
        .is_err());

    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .unwrap();
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

#[tokio::test]
async fn websocket_jsonrpc_target_path_only_no_method_keeps_zero_acquire() {
    let (host, physical) = fixture::admitted_path_only_websocket_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    assert!(physical.entry().optional_handler().is_none());
    assert!(!physical.has_websocket_jsonrpc_methods().unwrap());

    let header = handlerless_connect_header(&physical, "path-only-connect");
    let frame = encode_binary_frame(&header, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut fingerprint = None;
    super::super::dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut fingerprint,
    )
    .await
    .expect("path-only runtime request should fail closed at Host admission");
    let RouterWriterMessage::Binary(_) = receiver.recv().await.unwrap() else {
        panic!("path-only admission rejection must use binary response.error")
    };
    assert!(
        receiver.try_recv().is_err(),
        "path-only admission must not queue a generation acquire"
    );
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

#[tokio::test]
async fn websocket_generation_acquire_receipt_mismatch_fails_and_cleans_before_attach() {
    let (host, physical, _) = fixture::admitted_websocket_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let websocket_entry_id = skiff_artifact_identity::websocket_entry_id(
        &physical.entry().owner().service_id,
        physical.gateway_entry_key(),
    )
    .unwrap();
    let (acquire, receipt) = host
        .websocket_generations
        .begin_acquire_with_receipt(
            ROUTER_SESSION,
            physical,
            websocket_entry_id.as_str().to_string(),
            "receipt-mismatch".to_string(),
        )
        .unwrap();
    let mut mismatch = acquire_ack(&acquire);
    let WebSocketGenerationLifecycleControl::Ack { tuple, .. } = &mut mismatch else {
        unreachable!()
    };
    tuple.websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "f".repeat(64));
    assert!(host
        .websocket_generations
        .handle_acquire_response(&mismatch)
        .is_err());
    assert!(receipt.wait().await.is_err());
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

fn handlerless_connect_header(
    route: &crate::loader::assembly_admission::ActiveAssemblyRoute,
    request_id: &str,
) -> RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
    let selector = route.selector();
    let websocket_entry_id = skiff_artifact_identity::websocket_entry_id(
        &route.entry().owner().service_id,
        route.gateway_entry_key(),
    )
    .unwrap();
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: route.assembly_identity().clone(),
            assembly_generation: route.generation(),
            gateway_entry_identity: route.gateway_entry_identity().clone(),
            ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader {
                protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                host: selector.host.clone(),
                method: (),
                path: selector.path.clone(),
            },
        },
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-handlerless-websocket".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader {
            connection_id: "handlerless-connection".to_string(),
            url: format!("ws://{}{}", selector.host, selector.path),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            version: None,
            websocket_entry_id,
            gateway_entry_identity: route.gateway_entry_identity().clone(),
        },
        test_effects_enabled: false,
    }
}

fn acquire_ack(
    acquire: &WebSocketGenerationLifecycleControl,
) -> WebSocketGenerationLifecycleControl {
    let WebSocketGenerationLifecycleControl::Acquire {
        request_id, tuple, ..
    } = acquire
    else {
        panic!("expected acquire")
    };
    WebSocketGenerationLifecycleControl::Ack {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        operation: WebSocketGenerationLifecycleOperation::Acquire,
        request_id: request_id.clone(),
        sender: WebSocketGenerationLifecycleSender::Router,
        tuple: tuple.clone(),
    }
}

fn acquire_rejection(
    acquire: &WebSocketGenerationLifecycleControl,
) -> WebSocketGenerationLifecycleControl {
    let WebSocketGenerationLifecycleControl::Acquire {
        request_id, tuple, ..
    } = acquire
    else {
        panic!("expected acquire")
    };
    WebSocketGenerationLifecycleControl::Reject {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        operation: WebSocketGenerationLifecycleOperation::Acquire,
        request_id: request_id.clone(),
        sender: WebSocketGenerationLifecycleSender::Router,
        tuple: tuple.clone(),
        code: WebSocketGenerationLifecycleRejectionCode::GenerationUnavailable,
        reason: "generation unavailable".to_string(),
    }
}

fn lifecycle_tuple(
    control: &WebSocketGenerationLifecycleControl,
) -> &WebSocketGenerationLifecycleTuple {
    match control {
        WebSocketGenerationLifecycleControl::Acquire { tuple, .. } => tuple,
        _ => panic!("expected acquire"),
    }
}

fn release_control(
    suffix: &str,
    tuple: WebSocketGenerationLifecycleTuple,
) -> WebSocketGenerationLifecycleControl {
    WebSocketGenerationLifecycleControl::Release {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        request_id: format!("skiff-websocket-lifecycle-request-v1:opaque:{suffix}"),
        sender: WebSocketGenerationLifecycleSender::Router,
        tuple,
    }
}

async fn receive_lifecycle(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
) -> WebSocketGenerationLifecycleControl {
    let RouterWriterMessage::Binary(frame) = receiver
        .recv()
        .await
        .expect("lifecycle response should queue")
    else {
        panic!("lifecycle response must be binary")
    };
    decode_websocket_generation_lifecycle_frame(
        WebSocketGenerationLifecycleDirection::RuntimeToRouter,
        &frame,
    )
    .expect("lifecycle response should decode")
}
