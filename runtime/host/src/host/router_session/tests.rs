use serde_json::json;

use super::*;
use skiff_runtime_loader::RuntimeAssemblyContentResolver;
use skiff_runtime_transport::{
    assembly_activation::{
        decode_assembly_activation_frame, encode_assembly_activation_frame,
        AssemblyActivationFrameDirection,
    },
    protocol::{
        encode_binary_frame, RequestCancelFrameHeader, ResponseChunkFrameHeader,
        ResponseEndFrameHeader, ResponseErrorFrameHeader, ResponseStartFrameHeader,
        RouterControlFrameHeader, RuntimeErrorFramePayload, RuntimeHealthCountersFrameHeader,
        RuntimeHealthFrameHeader, RuntimeHttpResponseFrameHeader, RuntimeRegisteredFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
};

mod runtime_assembly_request;

#[derive(Clone)]
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

#[derive(Clone)]
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

fn test_db_provider() -> skiff_runtime_capability_context::DbProviderSource {
    skiff_runtime_capability_context::DbProviderSource::new(TestDbProviderFactory)
}

fn test_host() -> super::super::RuntimeHost {
    super::super::RuntimeHost::new(super::super::RuntimeConfig {
        db_provider: test_db_provider(),
        services: Vec::new(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("runtime host should build")
}

#[tokio::test]
async fn text_json_router_control_is_rejected_on_runtime_websocket() {
    let error = reject_router_text_message(
        &json!({
            "type": "router.control",
            "artifactRoots": ["/tmp/skiff-runtime-router-control"],
        })
        .to_string(),
    )
    .expect_err("text JSON router.control should fail closed");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("text protocol messages are not supported on runtime WebSocket"));
}

#[test]
fn writer_encodes_outbound_control_command_as_binary_frame() {
    let message = super::super::RouterWriterMessage::Control(
        skiff_runtime_request::OutboundControlMessage::RequestCancel {
            request: skiff_runtime_request::RequestCancelControl {
                request_id: "request-cancel-from-control".to_string(),
                reason: "caller_cancel".to_string(),
            },
        },
    );

    let bytes = match encode_writer_message(message).expect("control command should encode") {
        tokio_tungstenite::tungstenite::Message::Binary(bytes) => bytes,
        other => panic!("expected binary websocket message, got {other:?}"),
    };
    let (header, payload): (RequestCancelFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&bytes).expect("request.cancel should decode");

    assert_eq!(header.request_id, "request-cancel-from-control");
    assert_eq!(header.reason, "caller_cancel");
    assert!(payload.is_empty());
}

#[tokio::test]
async fn runtime_health_frame_reports_loop_risk_counters() {
    let host = test_host();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let stream_baseline = crate::capability_context::stream_runtime_streams_active();
    let flag_waiter_baseline =
        skiff_runtime_capability_context::flag_backed_cancel_waiters_active();

    let counters = host.runtime_health_counters().await;
    host.queue_runtime_health_with_counters(&sender, "runtime-health-zero", counters)
        .await
        .expect("runtime.health should encode");

    let frame = match receiver
        .recv()
        .await
        .expect("runtime.health frame should be queued")
    {
        super::super::RouterWriterMessage::Binary(frame) => frame,
        other => panic!("expected binary runtime.health frame, got {other:?}"),
    };
    let (header, payload): (RuntimeHealthFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("runtime.health should decode");

    assert!(payload.is_empty());
    assert_eq!(header.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(header.envelope_type, "runtime.health");
    assert_eq!(header.runtime_id, "runtime-health-zero");
    assert_eq!(header.counters.outbound_requests_pending, 0);
    assert_eq!(header.counters.outbound_stream_leases_active, 0);
    assert_eq!(
        header.counters.stream_runtime_streams_active,
        stream_baseline
    );
    assert_eq!(
        header.counters.flag_backed_cancel_waiters_active,
        flag_waiter_baseline
    );
    assert_eq!(header.counters.spawned_tasks_active, 0);
}

#[tokio::test]
async fn runtime_health_reporter_sends_immediate_zero_transition() {
    let host = test_host();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut reporter = RuntimeHealthReporter::default();
    reporter
        .registered_runtime_ids
        .insert("runtime-health-zero-transition".to_string());

    reporter
        .send_counters(
            &host,
            &sender,
            runtime_health_counters_for_test(1, 1, 0, 0, 0),
        )
        .await
        .expect("nonzero runtime.health should send");
    let nonzero = recv_runtime_health(&mut receiver).await;
    assert_eq!(nonzero.runtime_id, "runtime-health-zero-transition");
    assert_eq!(nonzero.counters.outbound_requests_pending, 1);
    assert!(reporter.should_probe_zero_transition());

    let sent = reporter
        .send_zero_transition_for_counters(
            &host,
            &sender,
            runtime_health_counters_for_test(0, 0, 0, 0, 0),
        )
        .await
        .expect("zero transition runtime.health should send");
    assert!(sent);
    let zero = recv_runtime_health(&mut receiver).await;
    assert_eq!(zero.runtime_id, "runtime-health-zero-transition");
    assert_eq!(zero.counters.outbound_requests_pending, 0);
    assert_eq!(zero.counters.outbound_stream_leases_active, 0);
    assert!(!reporter.should_probe_zero_transition());
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn runtime_health_reporter_sends_final_frame_before_session_close() {
    let host = test_host();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut reporter = RuntimeHealthReporter::default();
    reporter
        .registered_runtime_ids
        .insert("runtime-health-final".to_string());

    reporter
        .send_final(&host, &sender)
        .await
        .expect("final runtime.health should send before session close");
    let final_health = recv_runtime_health(&mut receiver).await;
    assert_eq!(final_health.runtime_id, "runtime-health-final");
    assert_eq!(final_health.envelope_type, "runtime.health");
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn binary_runtime_registered_with_empty_payload_is_accepted() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &RuntimeRegisteredFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.registered".to_string(),
            runtime_id: "runtime-registered-binary".to_string(),
        },
        &[],
    )
    .expect("runtime.registered frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("binary runtime.registered should be accepted");

    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

async fn recv_runtime_health(
    receiver: &mut mpsc::UnboundedReceiver<super::super::RouterWriterMessage>,
) -> RuntimeHealthFrameHeader {
    match receiver
        .recv()
        .await
        .expect("runtime.health frame should be queued")
    {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (RuntimeHealthFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("runtime.health should decode");
            assert!(payload.is_empty());
            header
        }
        other => panic!("expected binary runtime.health frame, got {other:?}"),
    }
}

fn runtime_health_counters_for_test(
    outbound_requests_pending: usize,
    outbound_stream_leases_active: usize,
    stream_runtime_streams_active: usize,
    flag_backed_cancel_waiters_active: usize,
    spawned_tasks_active: usize,
) -> RuntimeHealthCountersFrameHeader {
    RuntimeHealthCountersFrameHeader {
        outbound_requests_pending,
        outbound_stream_leases_active,
        stream_runtime_streams_active,
        flag_backed_cancel_waiters_active,
        spawned_tasks_active,
    }
}

#[tokio::test]
async fn binary_runtime_registered_rejects_non_empty_payload() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &RuntimeRegisteredFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.registered".to_string(),
            runtime_id: "runtime-registered-binary".to_string(),
        },
        b"unexpected",
    )
    .expect("runtime.registered frame should encode");

    let error = dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect_err("non-empty runtime.registered payload should fail");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("runtime.registered binary frame payload must be empty"));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_router_control_rejects_non_empty_payload() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &RouterControlFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "router.control".to_string(),
            artifact_roots: vec!["/tmp/skiff-runtime-router-control".into()],
            dev_reload: None,
            mode: None,
            generation: None,
            fingerprint: None,
            service_config: Vec::new(),
            telemetry: None,
            file_backend: None,
        },
        b"unexpected",
    )
    .expect("router.control frame should encode");

    let error = dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect_err("non-empty router.control payload should fail");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("router.control binary frame payload must be empty"));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_router_control_decode_error_propagates() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &json!({
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "router.control",
            "artifactRoots": 123,
        }),
        &[],
    )
    .expect("invalid router.control frame should encode");

    let error = dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect_err("invalid binary router.control should fail");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_assembly_activation_command_uses_router_to_runtime_codec() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let activation = assembly_activation_control("prepare");
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &activation,
    )
    .expect("router activation command should encode");

    let error = dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect_err("activation without a canonical artifact root should fail after frame decode");

    assert!(error.to_string().contains(
        "whole-assembly activation requires exactly one configured canonical artifact root"
    ));
}

#[test]
fn assembly_activation_reply_uses_runtime_to_router_codec() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let activation = assembly_activation_control("prepared");

    super::super::RuntimeHost::queue_assembly_activation(sender, &activation)
        .expect("runtime activation reply should queue");
    let super::super::RouterWriterMessage::Binary(frame) = receiver
        .try_recv()
        .expect("runtime activation reply should be present")
    else {
        panic!("expected binary assembly activation reply");
    };
    let decoded =
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, &frame)
            .expect("runtime activation reply should decode in runtime-to-router direction");

    assert_eq!(decoded, activation);
}

fn assembly_activation_control(
    control_type: &str,
) -> skiff_artifact_model::AssemblyActivationControl {
    serde_json::from_value(json!({
        "type": control_type,
        "environment": "test",
        "activationId": "activation-42",
        "expectedGeneration": 41,
        "candidateGeneration": 42,
        "assembly": {
            "assemblyIdentity": "skiff-runtime-assembly-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "replicaId": "runtime-a"
    }))
    .expect("assembly activation control fixture should decode")
}

#[tokio::test]
async fn binary_response_end_completes_pending_outbound_request() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let (response_sender, mut response_receiver) = mpsc::unbounded_channel();
    let lease = host
        .outbound_requests
        .insert_with_lease(
            "request-outbound-1".to_string(),
            response_sender,
            None,
            "caller_cancel",
        )
        .expect("pending outbound response should register");
    let frame = encode_binary_frame(
        &ResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.end".to_string(),
            request_id: "request-outbound-1".to_string(),
            payload_present: true,
            http_response: None,
            websocket_connect: None,
        },
        b"encoded-result",
    )
    .expect("response.end frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("response.end should route to pending outbound request");

    let response = response_receiver
        .recv()
        .await
        .expect("pending outbound receiver should complete");
    assert!(matches!(
        response,
        skiff_runtime_request::OutboundResponse::End { payload }
            if payload == b"encoded-result"
    ));
    assert!(host.outbound_requests.contains("request-outbound-1"));
    lease.complete();
    assert!(!host.outbound_requests.contains("request-outbound-1"));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_response_error_completes_pending_outbound_request() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let (response_sender, mut response_receiver) = mpsc::unbounded_channel();
    let lease = host
        .outbound_requests
        .insert_with_lease(
            "request-outbound-error".to_string(),
            response_sender,
            None,
            "caller_cancel",
        )
        .expect("pending outbound response should register");
    let frame = encode_binary_frame(
        &ResponseErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.error".to_string(),
            request_id: "request-outbound-error".to_string(),
            error: RuntimeErrorFramePayload {
                code: "RemoteError".to_string(),
                message: "callee failed".to_string(),
                status: Some(503),
                details: None,
            },
        },
        &[],
    )
    .expect("response.error frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("response.error should route to pending outbound request");

    let response = response_receiver
        .recv()
        .await
        .expect("pending outbound receiver should complete");
    assert!(matches!(
        response,
        skiff_runtime_request::OutboundResponse::Error(error)
            if error.message == "callee failed" && error.status == Some(503)
    ));
    assert!(host.outbound_requests.contains("request-outbound-error"));
    lease.complete();
    assert!(!host.outbound_requests.contains("request-outbound-error"));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_response_start_for_pending_outbound_sends_stream_event_without_completing() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let (response_sender, mut response_receiver) = mpsc::unbounded_channel();
    let _lease = host
        .outbound_requests
        .insert_with_lease(
            "request-outbound-stream".to_string(),
            response_sender,
            None,
            "caller_cancel",
        )
        .expect("pending outbound response should register");
    let frame = encode_binary_frame(
        &ResponseStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.start".to_string(),
            request_id: "request-outbound-stream".to_string(),
            http_response: RuntimeHttpResponseFrameHeader {
                status: 200,
                headers: Vec::new(),
            },
        },
        &[],
    )
    .expect("response.start frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("response.start should route to pending outbound request");

    assert!(host.outbound_requests.contains("request-outbound-stream"));
    let response = response_receiver
        .try_recv()
        .expect("response.start event should be available");
    assert!(matches!(
        response,
        skiff_runtime_request::OutboundResponse::Start { http_response }
            if http_response.status == 200
    ));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_response_chunk_for_pending_outbound_sends_stream_event_without_completing() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let (response_sender, mut response_receiver) = mpsc::unbounded_channel();
    let _lease = host
        .outbound_requests
        .insert_with_lease(
            "request-outbound-stream".to_string(),
            response_sender,
            None,
            "caller_cancel",
        )
        .expect("pending outbound response should register");
    let frame = encode_binary_frame(
        &ResponseChunkFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.chunk".to_string(),
            request_id: "request-outbound-stream".to_string(),
            seq: 0,
        },
        b"chunk",
    )
    .expect("response.chunk frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("response.chunk should route to pending outbound request");

    assert!(host.outbound_requests.contains("request-outbound-stream"));
    let response = response_receiver
        .try_recv()
        .expect("response.chunk event should be available");
    assert!(matches!(
        response,
        skiff_runtime_request::OutboundResponse::Chunk { seq: 0, payload }
            if payload == b"chunk".to_vec()
    ));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn text_json_request_start_is_rejected_on_runtime_websocket() {
    let error = reject_router_text_message(
        &json!({
            "type": "request.start",
            "requestId": "request-legacy-text",
            "mode": "unary",
            "target": "service.test.Api.hello",
            "buildId": "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "serviceProtocolIdentity": "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "trace": {
                "traceId": "trace-legacy-text",
                "spanId": "span-legacy-text"
            },
            "args": {
                "name": "Ada"
            }
        })
        .to_string(),
    )
    .expect_err("text protocol request.start should fail closed");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("text protocol messages are not supported on runtime WebSocket"));
}
