use std::{sync::Arc, time::Duration};

use serde_json::json;
use skiff_artifact_model::{
    AssemblyIdentity, GatewayDispatchMode, GatewayProtocolSurface, IngressProtocol,
};
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, RequestCancelFrameHeader,
        ResponseChunkFrameHeader, ResponseEndFrameHeader, ResponseEndFrameMetadata,
        ResponseErrorFrameHeader, ResponseStartFrameHeader, TypedEnvelope,
        BINARY_FRAME_HEADER_ENCODING_JSON, BINARY_FRAME_MAGIC, BINARY_FRAME_VERSION,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
    runtime_assembly_request::{
        RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
        RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestIngressFrameHeader,
        RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestRoutingFrameHeader,
        RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
    },
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::{sync::mpsc, time::timeout};

use crate::{host::RuntimeHost, loader::assembly_admission::ActiveAssemblyRoute};

pub(super) mod fixture;

#[tokio::test]
async fn host_current_scope_compiled_artifact_admits_exact_source_routes() {
    let (_host, routes) = fixture::admitted_current_scope_gateway_host().await;
    assert_eq!(routes.len(), 3);

    let unary = &routes["/current-scope/unary"];
    assert_eq!(
        unary.assembly_identity().as_str(),
        "skiff-runtime-assembly-v3:sha256:c85e32a68052e107eab2f93934ea77ca96f20c868df2a7278bc60c5306525e83"
    );
    assert_eq!(
        unary.gateway_entry_identity().as_str(),
        "skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2"
    );
    let GatewayProtocolSurface::Http(unary_surface) = &unary.protocol_surface().protocol else {
        panic!("current-scope unary route must remain HTTP")
    };
    assert_eq!(unary_surface.dispatch_mode, GatewayDispatchMode::Unary);

    let stream = &routes["/current-scope/stream"];
    assert_eq!(
        stream.gateway_entry_identity().as_str(),
        "skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0"
    );
    let GatewayProtocolSurface::Http(stream_surface) = &stream.protocol_surface().protocol else {
        panic!("current-scope stream route must remain HTTP")
    };
    assert_eq!(
        stream_surface.dispatch_mode,
        GatewayDispatchMode::ServerStream
    );

    let websocket = &routes["/current-scope/socket"];
    assert_eq!(
        websocket.gateway_entry_identity().as_str(),
        "skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d"
    );
    assert!(matches!(
        websocket.protocol_surface().protocol,
        GatewayProtocolSurface::WebSocketConnect(_)
    ));
}

#[tokio::test]
async fn host_http_gateway_typed_raw_and_stream_execute_private_handlers() {
    let (host, routes) = fixture::admitted_gateway_host().await;

    let typed = canonical_header(&routes["/typed"], "host-http-typed");
    let (typed_end, typed_body) = dispatch_unary(&host, typed, br#""typed-body""#, 1024).await;
    assert_eq!(http_status(&typed_end), 200);
    assert_eq!(typed_body, br#""typed-body""#);

    let raw = canonical_header(&routes["/raw"], "host-http-raw");
    let raw_body = vec![0, 1, 2, 0xff, 0xfe];
    let (raw_end, returned_raw_body) = dispatch_unary(&host, raw, &raw_body, 1024).await;
    assert_eq!(http_status(&raw_end), 201);
    assert_eq!(returned_raw_body, raw_body);

    let stream = canonical_header(&routes["/stream"], "host-http-stream");
    let stream_body = vec![0, 0xff, 3];
    let frame = encode_binary_frame(&stream, &stream_body).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender).await.unwrap();
    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 202);
    assert!(payload.is_empty());
    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, stream_body);
    let end = recv_binary(&mut receiver).await;
    let (end, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&end).unwrap();
    assert_eq!(end.request_id, stream.request_id);
    assert_eq!(end.metadata, ResponseEndFrameMetadata::None);
    assert!(payload.is_empty());
    assert_no_second_frame(&mut receiver).await;
}

#[tokio::test]
async fn package_direct_http_stream_registry_return_stream_reaches_real_gateway() {
    let (host, routes) = fixture::admitted_package_direct_stream_gateway_host().await;
    let stream = canonical_header(
        &routes["/package-direct/stream"],
        "package-direct-http-stream-return",
    );
    let body = b"package-direct-body";
    let frame = encode_binary_frame(&stream, body).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender).await.unwrap();

    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 202);
    assert!(payload.is_empty());

    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, body);

    let end = recv_binary(&mut receiver).await;
    let (end, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&end).unwrap();
    assert_eq!(end.request_id, stream.request_id);
    assert_eq!(end.metadata, ResponseEndFrameMetadata::None);
    assert!(payload.is_empty());
    assert_no_second_frame(&mut receiver).await;
}

#[test]
fn package_direct_stream_producer_argument_real_gateway() {
    std::thread::Builder::new()
        .name("package-direct-stream-producer-argument".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("stream-producing argument test runtime")
                .block_on(async {
                    package_direct_stream_producer_argument_normal().await;
                    package_direct_stream_producer_argument_producer_error().await;
                    package_direct_stream_producer_argument_consumer_cancel().await;
                });
        })
        .expect("stream-producing argument test thread")
        .join()
        .expect("stream-producing argument test thread should not panic");
}

#[test]
fn deferred_package_direct_stream_keeps_raw_http_response_sink() {
    std::thread::Builder::new()
        .name("deferred-package-direct-response-sink".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("deferred response-sink test runtime")
                .block_on(async {
                    deferred_package_direct_response_sink_normal().await;
                    deferred_package_direct_response_sink_producer_error().await;
                    deferred_package_direct_response_sink_consumer_cancel().await;
                });
        })
        .expect("deferred response-sink test thread")
        .join()
        .expect("deferred response-sink test thread should not panic");
}

async fn deferred_package_direct_response_sink_normal() {
    let (host, routes) = fixture::admitted_stream_argument_gateway_host().await;
    let stream = canonical_header(
        &routes["response-sink-normal"],
        "deferred-package-direct-response-sink-normal",
    );
    let frame = encode_binary_frame(&stream, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender).await.unwrap();

    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 200);
    assert!(payload.is_empty());

    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, b"body");

    let end = recv_binary(&mut receiver).await;
    let (end, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&end).unwrap();
    assert_eq!(end.request_id, stream.request_id);
    assert_eq!(end.metadata, ResponseEndFrameMetadata::None);
    assert!(payload.is_empty());
    assert_no_second_frame(&mut receiver).await;
    assert_eq!(host.request_supervisor.active_count().await, 0);
}

async fn deferred_package_direct_response_sink_producer_error() {
    let (host, routes) = fixture::admitted_stream_argument_gateway_host().await;
    let stream = canonical_header(
        &routes["response-sink-producer-error"],
        "deferred-package-direct-response-sink-producer-error",
    );
    let frame = encode_binary_frame(&stream, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender).await.unwrap();

    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 200);
    assert!(payload.is_empty());

    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, b"before-error");

    let Terminal::Error(response, payload) = recv_terminal(&mut receiver).await else {
        panic!("producer error must preserve the native response chunk and terminate once")
    };
    assert_eq!(response.request_id(), stream.request_id);
    assert!(!control_error(&response, &payload).code.is_empty());
    assert_no_second_frame(&mut receiver).await;
    assert_eq!(host.request_supervisor.active_count().await, 0);
}

async fn deferred_package_direct_response_sink_consumer_cancel() {
    let (host, routes) = fixture::admitted_stream_argument_gateway_host().await;
    let stream = canonical_header(
        &routes["response-sink-consumer-cancel"],
        "deferred-package-direct-response-sink-consumer-cancel",
    );
    let frame = encode_binary_frame(&stream, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender).await.unwrap();

    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 200);
    assert!(payload.is_empty());

    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, b"first");

    dispatch_cancel(&host, &stream, &sender, "consumer_break").await;
    for _ in 0..100 {
        if host.request_supervisor.active_count().await == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_frame(&mut receiver).await;
}

async fn package_direct_stream_producer_argument_normal() {
    let (host, routes) = fixture::admitted_stream_argument_gateway_host().await;
    let stream = canonical_header(&routes["normal"], "package-direct-stream-argument-normal");
    let frame = encode_binary_frame(&stream, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender).await.unwrap();

    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 200);
    assert!(payload.is_empty());

    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, b"body");

    let end = recv_binary(&mut receiver).await;
    let (end, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&end).unwrap();
    assert_eq!(end.request_id, stream.request_id);
    assert_eq!(end.metadata, ResponseEndFrameMetadata::None);
    assert!(payload.is_empty());
    assert_no_second_frame(&mut receiver).await;
    assert_eq!(host.request_supervisor.active_count().await, 0);
}

async fn package_direct_stream_producer_argument_producer_error() {
    let (host, routes) = fixture::admitted_stream_argument_gateway_host().await;
    let stream = canonical_header(
        &routes["producer-error"],
        "package-direct-stream-argument-producer-error",
    );
    let frame = encode_binary_frame(&stream, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender).await.unwrap();

    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 200);
    assert!(payload.is_empty());

    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, b"before-error");

    let Terminal::Error(response, payload) = recv_terminal(&mut receiver).await else {
        panic!("producer error must preserve prior response items and end once with an error")
    };
    assert_eq!(response.request_id(), stream.request_id);
    assert!(!control_error(&response, &payload).code.is_empty());
    assert_no_second_frame(&mut receiver).await;
    assert_eq!(host.request_supervisor.active_count().await, 0);
}

async fn package_direct_stream_producer_argument_consumer_cancel() {
    let (host, routes) = fixture::admitted_stream_argument_gateway_host().await;
    let stream = canonical_header(
        &routes["consumer-cancel"],
        "package-direct-stream-argument-consumer-cancel",
    );
    let frame = encode_binary_frame(&stream, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender).await.unwrap();

    let start = recv_binary(&mut receiver).await;
    let (start, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&start).unwrap();
    assert_eq!(start.request_id, stream.request_id);
    assert_eq!(start.http_response.status, 200);
    assert!(payload.is_empty());

    let chunk = recv_binary(&mut receiver).await;
    let (chunk, payload): (ResponseChunkFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&chunk).unwrap();
    assert_eq!(chunk.request_id, stream.request_id);
    assert_eq!(chunk.seq, 0);
    assert_eq!(payload, b"first");

    dispatch_cancel(&host, &stream, &sender, "consumer_break").await;
    for _ in 0..100 {
        if host.request_supervisor.active_count().await == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_frame(&mut receiver).await;
}

#[tokio::test]
async fn host_http_gateway_exact_route_identity_generation_mode_and_http_metadata_fail_closed() {
    let (host, routes) = fixture::admitted_gateway_host().await;
    let exact = canonical_header(&routes["/typed"], "host-http-negative");
    let mut cases = Vec::new();

    let mut different_url_host = exact.clone();
    different_url_host.request_id = "host-http-url-host-metadata".to_string();
    different_url_host.http_request.url = "http://other.test/typed".to_string();
    let (response, body) = dispatch_unary(&host, different_url_host, br#""url-host""#, 1024).await;
    assert_eq!(http_status(&response), 200);
    assert_eq!(body, br#""url-host""#);

    let mut wrong_assembly = exact.clone();
    wrong_assembly.routing.assembly_identity = AssemblyIdentity::new(format!(
        "skiff-runtime-assembly-v3:sha256:{}",
        "f".repeat(64)
    ));
    cases.push(("assembly", wrong_assembly));

    let mut wrong_generation = exact.clone();
    wrong_generation.routing.assembly_generation += 1;
    cases.push(("generation", wrong_generation));

    let mut wrong_gateway_identity = exact.clone();
    wrong_gateway_identity.routing.gateway_entry_identity =
        routes["/raw"].gateway_entry_identity().clone();
    cases.push(("gateway-identity", wrong_gateway_identity));

    let mut wrong_mode = exact.clone();
    wrong_mode.mode = "serverStream".to_string();
    cases.push(("mode", wrong_mode));

    let mut wrong_method = exact.clone();
    wrong_method.http_request.method = "GET".to_string();
    cases.push(("method", wrong_method));

    let mut wrong_path = exact.clone();
    wrong_path.http_request.path = "/wrong".to_string();
    cases.push(("path", wrong_path));

    let mut wrong_url_path = exact.clone();
    wrong_url_path.http_request.url = "http://api.example.test/wrong".to_string();
    cases.push(("url-path", wrong_url_path));

    let mut wrong_deployment = exact;
    wrong_deployment.routing.deployment.service_id =
        "example.com/other-host-http-gateway-service".to_string();
    cases.push(("deployment", wrong_deployment));

    for (name, mut header) in cases {
        header.request_id = format!("host-http-negative-{name}");
        let frame = encode_binary_frame(&header, br#""body""#).unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        dispatch(&host, &frame, &sender).await.unwrap();
        let Terminal::Error(response, payload) = recv_terminal(&mut receiver).await else {
            panic!("{name} must return response.error")
        };
        assert_eq!(response.request_id(), header.request_id);
        let _ = control_error(&response, &payload);
        assert_no_second_frame(&mut receiver).await;
    }

    let typed = &routes["/typed"];
    assert_eq!(typed.selector().protocol, IngressProtocol::Http);
    assert_eq!(typed.gateway_entry_key().as_str(), "typed");
    assert_eq!(
        typed.gateway_entry_identity(),
        typed.entry().gateway_entry_identity()
    );
    assert!(typed.selector_and_owner_key_share_entry());
}

#[tokio::test]
async fn host_http_gateway_deadline_clamps_wire_expiry_and_deployment_policy() {
    let (host, routes) = fixture::admitted_gateway_host().await;
    let route = &routes["/typed"];
    let mut header = canonical_header(route, "host-http-deadline");

    let policy_only = host
        .runtime_assembly_request_deadline_from_wire_for_test(&header)
        .unwrap()
        .unwrap();
    assert_eq!(policy_only.timeout_ms, 1_000);

    header.deadline = Some(deadline(75, 5_000));
    let wire_shorter = host
        .runtime_assembly_request_deadline_from_wire_for_test(&header)
        .unwrap()
        .unwrap();
    assert_eq!(wire_shorter.timeout_ms, 75);

    header.deadline = Some(deadline(5_000, 40));
    let expiry_shorter = host
        .runtime_assembly_request_deadline_from_wire_for_test(&header)
        .unwrap()
        .unwrap();
    assert!(expiry_shorter.timeout_ms <= 40);

    header.deadline = Some(RuntimeAssemblyRequestDeadlineFrameHeader {
        timeout_ms: 100,
        expires_at: "not-rfc3339".to_string(),
    });
    assert!(host
        .runtime_assembly_request_deadline_from_wire_for_test(&header)
        .unwrap_err()
        .to_string()
        .contains("RFC3339"));

    let expired = canonical_header(&routes["/slow"], "host-http-expired");
    let mut expired = expired;
    expired.deadline = Some(deadline(100, 0));
    let frame = encode_binary_frame(&expired, br#""slow""#).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender).await.unwrap();
    let Terminal::Error(response, payload) = recv_terminal(&mut receiver).await else {
        panic!("expired request must fail immediately")
    };
    assert_eq!(control_error(&response, &payload).code, "TimeoutError");
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_frame(&mut receiver).await;

    let mut running = canonical_header(&routes["/slow"], "host-http-running-deadline");
    running.deadline = Some(deadline(5, 5_000));
    let frame = encode_binary_frame(&running, br#""slow""#).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender).await.unwrap();
    let Terminal::Error(response, payload) = recv_terminal(&mut receiver).await else {
        panic!("running request deadline must remain an ordinary timeout")
    };
    assert_eq!(control_error(&response, &payload).code, "TimeoutError");
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_frame(&mut receiver).await;
}

#[tokio::test]
async fn host_http_gateway_response_ceiling_cancel_and_stream_terminal_are_single_owner() {
    let (host, routes) = fixture::admitted_gateway_host().await;

    let raw = canonical_header(&routes["/raw"], "host-http-over-limit");
    let frame = encode_binary_frame(&raw, b"too-large").unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    super::super::dispatch_router_binary_frame_with_http_response_max(&host, &frame, &sender, 1)
        .await
        .unwrap();
    let Terminal::Error(response, payload) = recv_terminal(&mut receiver).await else {
        panic!("oversize unary response must fail")
    };
    assert_eq!(
        control_error(&response, &payload).code,
        "ResourceLimitExceeded"
    );
    assert_no_second_frame(&mut receiver).await;

    let slow = canonical_header(&routes["/slow"], "host-http-cancel");
    let start = encode_binary_frame(&slow, br#""slow""#).unwrap();
    let cancel = encode_binary_frame(
        &RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.cancel".to_string(),
            request_id: slow.request_id.clone(),
            reason: "caller_cancel".to_string(),
        },
        &[],
    )
    .unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &start, &sender).await.unwrap();
    for _ in 0..50 {
        if host.request_supervisor.active_count().await == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    dispatch(&host, &cancel, &sender).await.unwrap();
    for _ in 0..100 {
        if host.request_supervisor.active_count().await == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert_eq!(host.outbound_requests.active_lease_count(), 0);
    assert_no_second_frame(&mut receiver).await;
    let cancel_events = host
        .telemetry
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .filter(|event| {
            event.request_id.as_deref() == Some("host-http-cancel")
                && event.name.as_deref() == Some("request.cancel")
        })
        .count();
    assert_eq!(cancel_events, 1);

    let stream = canonical_header(&routes["/stream"], "host-http-stream-over-limit");
    let frame = encode_binary_frame(&stream, b"stream-too-large").unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    super::super::dispatch_router_binary_frame_with_http_response_max(&host, &frame, &sender, 1)
        .await
        .unwrap();
    let first = recv_binary(&mut receiver).await;
    let (typed, _): (TypedEnvelope, Vec<u8>) = decode_typed_binary_frame(&first).unwrap();
    assert_eq!(typed.envelope_type, "response.start");
    let Terminal::Error(response, payload) = recv_terminal(&mut receiver).await else {
        panic!("oversize stream must have one error terminal")
    };
    assert_eq!(
        control_error(&response, &payload).code,
        "ResourceLimitExceeded"
    );
    assert_no_second_frame(&mut receiver).await;
}

#[tokio::test]
async fn host_cancel_races_success_error_and_deadline_with_one_terminal_owner() {
    let (host, routes) = fixture::admitted_gateway_host().await;

    let success = canonical_header(&routes["/typed"], "host-http-race-success");
    let success_frame = encode_binary_frame(&success, br#""success""#).unwrap();
    let (success_sender, mut success_receiver) = mpsc::unbounded_channel();
    dispatch(&host, &success_frame, &success_sender)
        .await
        .unwrap();
    dispatch_cancel(&host, &success, &success_sender, "race_success").await;
    assert_race_settled(
        &host,
        &success.request_id,
        &mut success_receiver,
        Some(("response.end", None)),
    )
    .await;

    let ordinary_error = canonical_header(&routes["/raw"], "host-http-race-error");
    let ordinary_error_frame = encode_binary_frame(&ordinary_error, b"too-large").unwrap();
    let (error_sender, mut error_receiver) = mpsc::unbounded_channel();
    super::super::dispatch_router_binary_frame_with_http_response_max(
        &host,
        &ordinary_error_frame,
        &error_sender,
        1,
    )
    .await
    .unwrap();
    dispatch_cancel(&host, &ordinary_error, &error_sender, "race_ordinary_error").await;
    assert_race_settled(
        &host,
        &ordinary_error.request_id,
        &mut error_receiver,
        Some(("response.error", Some("ResourceLimitExceeded"))),
    )
    .await;

    let mut deadline = canonical_header(&routes["/slow"], "host-http-race-deadline");
    deadline.deadline = Some(self::deadline(1, 5_000));
    let deadline_frame = encode_binary_frame(&deadline, br#""slow""#).unwrap();
    let (deadline_sender, mut deadline_receiver) = mpsc::unbounded_channel();
    dispatch(&host, &deadline_frame, &deadline_sender)
        .await
        .unwrap();
    tokio::task::yield_now().await;
    dispatch_cancel(&host, &deadline, &deadline_sender, "race_deadline").await;
    assert_race_settled(
        &host,
        &deadline.request_id,
        &mut deadline_receiver,
        Some(("response.error", Some("TimeoutError"))),
    )
    .await;
}

#[tokio::test]
async fn host_http_gateway_reload_pins_old_route_and_rejects_stale_wire() {
    let (host, pinned, current) = fixture::reloaded_gateway_host().await;
    assert_eq!(pinned.generation(), 1);
    assert_eq!(current.generation(), 2);
    assert!(!Arc::ptr_eq(pinned.context_set(), current.context_set()));
    assert!(pinned.request_target().is_ok());

    let stale = canonical_header(&pinned, "host-http-stale");
    let frame = encode_binary_frame(&stale, br#""stale""#).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender).await.unwrap();
    assert!(matches!(
        recv_terminal(&mut receiver).await,
        Terminal::Error(_, _)
    ));
    assert_no_second_frame(&mut receiver).await;
}

#[tokio::test]
async fn host_http_gateway_websocket_and_legacy_request_bridges_fail_before_host_admission() {
    let (host, routes) = fixture::admitted_gateway_host().await;
    let header = canonical_header(&routes["/typed"], "host-http-strict-wire");
    let canonical = serde_json::to_value(&header).unwrap();
    let mut websocket = canonical.clone();
    websocket["routing"]["ingress"]["protocol"] = json!("webSocket");
    websocket
        .as_object_mut()
        .unwrap()
        .insert("websocketEntryId".to_string(), json!("legacy"));
    let mut operation = canonical;
    operation["routing"]
        .as_object_mut()
        .unwrap()
        .insert("contractOperationId".to_string(), json!("legacy-operation"));

    for (name, value) in [("websocket", websocket), ("operation", operation)] {
        let frame = encode_binary_frame(&value, br#""body""#).unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let error = dispatch(&host, &frame, &sender)
            .await
            .expect_err("strict canonical decoder must reject legacy bridge metadata");
        assert!(!error.to_string().is_empty(), "{name}");
        assert!(receiver.try_recv().is_err(), "{name}");
    }
}

fn canonical_header(
    route: &ActiveAssemblyRoute,
    request_id: &str,
) -> RuntimeAssemblyRequestStartFrameHeader {
    let selector = route.selector();
    assert_eq!(selector.protocol, IngressProtocol::Http);
    let method = selector.method.clone().expect("HTTP method");
    let GatewayProtocolSurface::Http(http) = &route.protocol_surface().protocol else {
        panic!("HTTP request fixture must use an HTTP surface");
    };
    RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: match http.dispatch_mode {
            GatewayDispatchMode::Unary => "unary",
            GatewayDispatchMode::ServerStream => "serverStream",
        }
        .to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: route.assembly_identity().clone(),
            assembly_generation: route.generation(),
            deployment: route.deployment().clone(),
            gateway_entry_identity: route.gateway_entry_identity().clone(),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                method: method.clone(),
                path: selector.path.clone(),
            },
        },
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-host-http".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: RuntimeAssemblyHttpRequestFrameHeader {
            method,
            url: format!("http://api.example.test{}", selector.path),
            path: selector.path.clone(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: false,
    }
}

fn deadline(timeout_ms: u64, expires_in_ms: i64) -> RuntimeAssemblyRequestDeadlineFrameHeader {
    RuntimeAssemblyRequestDeadlineFrameHeader {
        timeout_ms,
        expires_at: (OffsetDateTime::now_utc() + time::Duration::milliseconds(expires_in_ms))
            .format(&Rfc3339)
            .unwrap(),
    }
}

async fn dispatch_unary(
    host: &RuntimeHost,
    header: RuntimeAssemblyRequestStartFrameHeader,
    body: &[u8],
    max_response_bytes: usize,
) -> (ResponseEndFrameHeader, Vec<u8>) {
    let frame = encode_binary_frame(&header, body).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    super::super::dispatch_router_binary_frame_with_http_response_max(
        host,
        &frame,
        &sender,
        max_response_bytes,
    )
    .await
    .unwrap();
    let (end, payload) = match recv_terminal(&mut receiver).await {
        Terminal::End(end, payload) => (end, payload),
        Terminal::Error(error, payload) => {
            panic!(
                "unary HTTP gateway request should succeed: {:?}",
                control_error(&error, &payload)
            )
        }
    };
    assert_no_second_frame(&mut receiver).await;
    (end, payload)
}

fn http_status(end: &ResponseEndFrameHeader) -> u16 {
    let ResponseEndFrameMetadata::Http(http) = &end.metadata else {
        panic!("HTTP unary response must carry HTTP metadata")
    };
    http.status
}

async fn dispatch(
    host: &RuntimeHost,
    frame: &[u8],
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> crate::error::Result<()> {
    let mut control = None;
    let mut artifact_fingerprint = None;
    super::super::dispatch_router_binary_frame(
        host,
        frame,
        sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
}

async fn dispatch_cancel(
    host: &RuntimeHost,
    request: &RuntimeAssemblyRequestStartFrameHeader,
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    reason: &str,
) {
    let cancel = encode_binary_frame(
        &RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.cancel".to_string(),
            request_id: request.request_id.clone(),
            reason: reason.to_string(),
        },
        &[],
    )
    .unwrap();
    dispatch(host, &cancel, sender).await.unwrap();
}

async fn assert_race_settled(
    host: &RuntimeHost,
    request_id: &str,
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    ordinary_terminal: Option<(&str, Option<&str>)>,
) {
    for _ in 0..100 {
        if host.request_supervisor.active_count().await == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(host.request_supervisor.active_count().await, 0);

    if let Ok(Some(RouterWriterMessage::Binary(frame))) =
        timeout(Duration::from_millis(50), receiver.recv()).await
    {
        let (typed, _): (TypedEnvelope, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("race terminal envelope");
        let (expected_type, expected_code) =
            ordinary_terminal.expect("only an ordinary winner emits a frame");
        assert_eq!(typed.envelope_type, expected_type);
        if let Some(expected_code) = expected_code {
            let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("ordinary race error");
            let error = control_error(&header, &payload);
            assert_eq!(
                error.code, expected_code,
                "unexpected race error: {error:?}"
            );
        }
    }
    assert_no_second_frame(receiver).await;

    let terminal_events = host
        .telemetry
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .filter(|event| {
            event.request_id.as_deref() == Some(request_id)
                && matches!(
                    event.name.as_deref(),
                    Some("request.end" | "request.error" | "request.cancel")
                )
        })
        .count();
    assert_eq!(
        terminal_events, 1,
        "success, ordinary error, deadline and cancellation share one terminal owner"
    );
}

#[derive(Debug)]
enum Terminal {
    End(ResponseEndFrameHeader, Vec<u8>),
    Error(ResponseErrorFrameHeader, Vec<u8>),
}

async fn recv_binary(receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>) -> Vec<u8> {
    let message = timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("response frame timeout")
        .expect("response channel closed");
    let RouterWriterMessage::Binary(frame) = message else {
        panic!("response must be a binary frame")
    };
    frame
}

async fn recv_terminal(receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>) -> Terminal {
    let frame = recv_binary(receiver).await;
    let (typed, _): (TypedEnvelope, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("terminal envelope");
    match typed.envelope_type.as_str() {
        "response.end" => {
            let (header, payload) = decode_typed_binary_frame(&frame).unwrap();
            Terminal::End(header, payload)
        }
        "response.error" => {
            let (header, payload) = decode_typed_binary_frame(&frame).unwrap();
            Terminal::Error(header, payload)
        }
        other => panic!("unexpected terminal {other}"),
    }
}

fn control_error<'a>(
    header: &'a ResponseErrorFrameHeader,
    payload: &[u8],
) -> &'a skiff_runtime_transport::protocol::RuntimeErrorFramePayload {
    assert!(payload.is_empty());
    match header {
        ResponseErrorFrameHeader::Control { error, .. } => error,
        ResponseErrorFrameHeader::FixedService { .. } => panic!("expected control error"),
    }
}

async fn assert_no_second_frame(receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>) {
    assert!(
        timeout(Duration::from_millis(50), receiver.recv())
            .await
            .is_err(),
        "request emitted a second terminal/frame"
    );
}

#[allow(dead_code)]
fn raw_binary_frame(header: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + header.len() + payload.len());
    frame.extend_from_slice(&BINARY_FRAME_MAGIC);
    frame.push(BINARY_FRAME_VERSION);
    frame.push(BINARY_FRAME_HEADER_ENCODING_JSON);
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(header);
    frame.extend_from_slice(payload);
    frame
}
