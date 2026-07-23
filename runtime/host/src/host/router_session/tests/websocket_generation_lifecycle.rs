use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::RUNTIME_FRAME_SCHEMA_VERSION,
    runtime_assembly_request::{
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestIngressFrameHeader,
        RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestRoutingFrameHeader,
        RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
        RuntimeAssemblyWebSocketAdapterArgFrameHeader, RuntimeAssemblyWebSocketAdapterFrameHeader,
        RuntimeAssemblyWebSocketAdapterKindFrameHeader,
        RuntimeAssemblyWebSocketAdapterSourceFrameHeader,
        RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader,
        RuntimeAssemblyWebSocketConnectRequestFrameHeader,
        RuntimeAssemblyWebSocketMessageEncodingFrameHeader,
        RuntimeAssemblyWebSocketMessageFrameHeader, RuntimeAssemblyWebSocketMessageTagFrameHeader,
        RuntimeAssemblyWebSocketPayloadSegmentFrameHeader,
        RuntimeAssemblyWebSocketPayloadSegmentKindFrameHeader,
        RuntimeAssemblyWebSocketReceiveEventFrameHeader,
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
use crate::loader::assembly_admission::ActiveAssemblyRoute;

const ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:test-session";
const OTHER_ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:other-session";
const CONNECTION_ID: &str = "connection-a";
const WEBSOCKET_ENTRY_ID: &str =
    "skiff-websocket-entry-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GATEWAY_ENTRY_ID: &str =
    "skiff-gateway-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn websocket_generation_old_route_survives_reload_until_disconnect_without_artifact_io() {
    let (host, generation_a, generation_b) = fixture::reloaded_websocket_host().await;
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

    let receive_a = receive_header(&generation_a, WEBSOCKET_ENTRY_ID, CONNECTION_ID);
    let pinned = host
        .runtime_assembly_request_route_from_wire_for_test(ROUTER_SESSION, &receive_a)
        .expect("generation A receive should resolve only from the in-memory pin");
    assert_eq!(pinned.generation(), 1);

    let connect_b = connect_header(&generation_b, "connection-b");
    let current = host
        .runtime_assembly_request_route_from_wire_for_test(ROUTER_SESSION, &connect_b)
        .expect("new connect should resolve the current generation");
    assert_eq!(current.generation(), 2);

    let mut wrong_generation = receive_a.clone();
    wrong_generation.routing.assembly_generation = generation_b.generation();
    assert!(host
        .runtime_assembly_request_route_from_wire_for_test(ROUTER_SESSION, &wrong_generation,)
        .is_err());
    assert!(host
        .runtime_assembly_request_route_from_wire_for_test(OTHER_ROUTER_SESSION, &receive_a)
        .is_err());

    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .expect("session disconnect should release all pins");
    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .expect("duplicate session disconnect should be idempotent");
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    assert!(host
        .runtime_assembly_request_route_from_wire_for_test(ROUTER_SESSION, &receive_a)
        .is_err());
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
    let (host, generation_a, _) = fixture::reloaded_websocket_host().await;
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
    let (host, generation_a, _) = fixture::reloaded_websocket_host().await;
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

fn connect_header(
    route: &ActiveAssemblyRoute,
    connection_id: &str,
) -> RuntimeAssemblyRequestStartFrameHeader {
    websocket_header(
        route,
        RuntimeAssemblyWebSocketAdapterFrameHeader {
            kind: RuntimeAssemblyWebSocketAdapterKindFrameHeader::Connect,
            adapter_args: canonical_adapter_args(),
            context_expectation: None,
            connect_request: Some(RuntimeAssemblyWebSocketConnectRequestFrameHeader {
                connection_id: connection_id.to_string(),
                url: "ws://canonical.test/consume".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
                cookies: Vec::new(),
                version: None,
            }),
            receive_event: None,
        },
    )
}

fn receive_header(
    route: &ActiveAssemblyRoute,
    websocket_entry_id: &str,
    connection_id: &str,
) -> RuntimeAssemblyRequestStartFrameHeader {
    let mut header = websocket_header(
        route,
        RuntimeAssemblyWebSocketAdapterFrameHeader {
            kind: RuntimeAssemblyWebSocketAdapterKindFrameHeader::Receive,
            adapter_args: canonical_adapter_args(),
            context_expectation: None,
            connect_request: None,
            receive_event: Some(RuntimeAssemblyWebSocketReceiveEventFrameHeader {
                connection_id: connection_id.to_string(),
                business_identity: None,
                message: RuntimeAssemblyWebSocketMessageFrameHeader {
                    tag: RuntimeAssemblyWebSocketMessageTagFrameHeader::Text,
                    encoding: RuntimeAssemblyWebSocketMessageEncodingFrameHeader::Utf8,
                },
                payload_segments: vec![RuntimeAssemblyWebSocketPayloadSegmentFrameHeader {
                    kind: RuntimeAssemblyWebSocketPayloadSegmentKindFrameHeader::Message,
                    offset: 0,
                    length: 1,
                }],
                context_codec: None,
            }),
        },
    );
    header.websocket_entry_id = Some(websocket_entry_id.to_string());
    header
}

fn websocket_header(
    route: &ActiveAssemblyRoute,
    websocket_adapter: RuntimeAssemblyWebSocketAdapterFrameHeader,
) -> RuntimeAssemblyRequestStartFrameHeader {
    RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: format!("request-{}", uuid::Uuid::new_v4()),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
            target: "__skiff.runtime-assembly-ingress".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: route.assembly_identity().clone(),
            assembly_generation: route.generation(),
            contract_operation_id: route.binding().contract_operation_id.clone(),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::WebSocket,
                host: route.binding().selector.host.clone(),
                method: None,
                path: route.binding().selector.path.clone(),
            },
        },
        activation_identity: None,
        gateway_entry_identity: Some(GATEWAY_ENTRY_ID.to_string()),
        business_identity: None,
        websocket_entry_id: Some(WEBSOCKET_ENTRY_ID.to_string()),
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: "trace-websocket-generation".to_string(),
            span_id: "span-websocket-generation".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: None,
        http_adapter: None,
        websocket_adapter: Some(websocket_adapter),
        test_effects_enabled: false,
        test_effect_doubles: Default::default(),
    }
}

fn canonical_adapter_args() -> Vec<RuntimeAssemblyWebSocketAdapterArgFrameHeader> {
    vec![RuntimeAssemblyWebSocketAdapterArgFrameHeader {
        param: "event".to_string(),
        source: RuntimeAssemblyWebSocketAdapterSourceFrameHeader {
            kind: RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::IngressEvent,
        },
    }]
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
