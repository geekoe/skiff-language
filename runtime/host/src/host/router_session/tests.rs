use serde_json::json;

use super::*;
use skiff_runtime_transport::{
    assembly_activation::{
        decode_assembly_activation_frame, encode_assembly_activation_frame,
        AssemblyActivationFrameDirection,
    },
    connection_protocol::{
        encode_connection_response_frame, ConnectionResponseFrameHeader, ConnectionResponseOutcome,
    },
    protocol::{
        encode_binary_frame, RequestCancelFrameHeader, RouterControlFrameHeader,
        RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader, RuntimeRegisteredFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
};

#[tokio::test]
async fn connection_request_response_demux_uses_exact_router_session() {
    let host = test_host();
    let session = skiff_runtime_capability_context::ConnectionRequestSession::new(
        "skiff-router-session-v1:opaque:test-session",
    )
    .expect("test session");
    let cancellation = skiff_runtime_capability_context::CancellationSource::new();
    let scope =
        skiff_runtime_capability_context::ExecutionScope::request(cancellation.token(), None);
    let mut pending = host
        .connection_requests
        .install(session, scope, std::sync::Arc::new(|_, _| Ok(())))
        .expect("pending request");
    let request_id = pending.request_id().to_string();
    let frame = encode_connection_response_frame(
        &ConnectionResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "connection.response".to_string(),
            request_id,
            outcome: ConnectionResponseOutcome::Success,
            remote: None,
        },
        b"null",
    )
    .expect("strict response frame");
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("response should dispatch");

    assert_eq!(
        pending.wait().await,
        skiff_runtime_capability_context::ConnectionRequestTerminal::Success(b"null".to_vec())
    );
    assert_eq!(host.connection_requests.pending_count(), 0);
    assert_eq!(host.connection_requests.active_lease_count(), 0);
    assert_eq!(host.connection_requests.active_timer_count(), 0);
}

mod connection_lifecycle;
mod foreign_db_exact_identity;
mod runtime_assembly_request;
mod websocket_generation_lifecycle;
mod websocket_jsonrpc_dispatch;

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
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        environment: "test".to_string(),
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
async fn writer_sends_no_websocket_frame_for_invalid_spawn_service_id() {
    use std::{
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        task::{Context, Poll},
    };

    struct CountingSocket(Arc<AtomicUsize>);

    impl futures_util::Sink<tokio_tungstenite::tungstenite::Message> for CountingSocket {
        type Error = tokio_tungstenite::tungstenite::Error;

        fn poll_ready(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(
            self: Pin<&mut Self>,
            _message: tokio_tungstenite::tungstenite::Message,
        ) -> std::result::Result<(), Self::Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    let encoded_frames = Arc::new(AtomicUsize::new(0));
    let message = super::super::RouterWriterMessage::Control(
        skiff_runtime_request::OutboundControlMessage::SpawnSubmit {
            request: skiff_runtime_request::SpawnSubmitControlRequest {
                rpc_id: "rpc-spawn".to_string(),
                runtime_id: "runtime-1".to_string(),
                target_kind: "operation".to_string(),
                service_id: "test.skiff/agine.ai/api-tests/case-23".to_string(),
                service_version: "1.0.0".to_string(),
                service_protocol_identity: "service-protocol-1".to_string(),
                target: "Worker.run".to_string(),
                spawn_id: Some("spawn-1".to_string()),
                build_id: Some("build-1".to_string()),
                activation_identity: skiff_runtime_request::ActivationIdentityControl {
                    assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                        "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    ),
                    generation: 7,
                    runtime_replica_id: "runtime-replica-7".to_string(),
                    deployment_revision: skiff_artifact_model::DeploymentRevision::new(
                        "deployment-revision-7",
                    ),
                },
                caller_request_id: Some("request-1".to_string()),
                trace_id: Some("trace-1".to_string()),
                caller_target: Some("Caller.start".to_string()),
                max_queue_wait_ms: Some(250.0),
            },
            payload: b"opaque spawn args".to_vec(),
        },
    );
    send_writer_message(&mut CountingSocket(Arc::clone(&encoded_frames)), message)
        .await
        .expect_err("invalid service ID must fail before writing a frame");

    assert_eq!(encoded_frames.load(Ordering::SeqCst), 0);
}

#[test]
fn connection_bootstrap_fixes_exact_artifact_path_and_db_transport() {
    let artifact_path = std::env::temp_dir().join(format!(
        "skiff-runtime-bootstrap-positive-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&artifact_path).expect("test artifact root should exist");
    let typed = TypedEnvelope {
        envelope_type: "router.bootstrap".to_string(),
        rest: serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(json!({
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "artifactsPath": artifact_path,
            "serviceDb": { "mongoUrl": "mongodb://router-owned" },
            "http": { "maxResponseBytes": 67108864 }
        }))
        .expect("bootstrap fields should decode"),
    };

    let bootstrap =
        super::decode_connection_bootstrap(typed, &[]).expect("bootstrap should install");

    assert_eq!(
        bootstrap.resolver.store().root(),
        artifact_path
            .canonicalize()
            .expect("test artifact root should canonicalize")
    );
    assert_eq!(
        bootstrap.service_db.mongo_url,
        "mongodb://router-owned".to_string()
    );
    assert_eq!(bootstrap.max_response_bytes, 67_108_864);
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
async fn binary_router_control_is_rejected_before_legacy_payload_decode() {
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
    .expect_err("legacy router.control should fail");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("router.control artifactRoots/serviceConfig reload is not supported"));
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
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let activation = assembly_activation_control("prepare");
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &activation,
    )
    .expect("router activation command should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("missing exact assembly record should produce a typed rejection");

    let super::super::RouterWriterMessage::Binary(reply) =
        receiver.try_recv().expect("rejection should be queued")
    else {
        panic!("expected binary rejection");
    };
    assert!(matches!(
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, &reply)
            .expect("rejection should decode"),
        skiff_artifact_model::AssemblyActivationControl::Reject { .. }
    ));
}

#[tokio::test]
async fn assembly_activation_fails_closed_before_connection_bootstrap() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &assembly_activation_control("prepare"),
    )
    .expect("router activation command should encode");
    let mut bootstrap = None;

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
    )
    .await
    .expect_err("activation before bootstrap must fail");

    assert!(error
        .to_string()
        .contains("assembly activation requires router.bootstrap first"));
}

#[tokio::test]
async fn duplicate_connection_bootstrap_fails_closed() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let artifact_path = std::env::temp_dir().join("skiff-runtime-bootstrap-duplicate");
    std::fs::create_dir_all(&artifact_path).expect("test artifact root should exist");
    let mut bootstrap = Some(super::ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .expect("test resolver should open"),
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        max_response_bytes: 67_108_864,
    });
    let frame = encode_binary_frame(
        &json!({
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "router.bootstrap",
            "artifactsPath": artifact_path,
            "serviceDb": { "mongoUrl": "mongodb://127.0.0.1:27017" },
            "http": { "maxResponseBytes": 67108864 }
        }),
        &[],
    )
    .expect("bootstrap frame should encode");

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
    )
    .await
    .expect_err("duplicate bootstrap must fail");

    assert!(error
        .to_string()
        .contains("router.bootstrap must appear exactly once per connection"));
}

#[tokio::test]
async fn activation_rejects_superseded_transient_service_db_wire() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let artifact_path = std::env::temp_dir().join("skiff-runtime-bootstrap-service-db");
    std::fs::create_dir_all(&artifact_path).expect("test artifact root should exist");
    let mut bootstrap = Some(super::ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .expect("test resolver should open"),
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://bootstrap-owner".to_string(),
        },
        max_response_bytes: 67_108_864,
    });
    let mut activation = serde_json::to_value(assembly_activation_control("prepare"))
        .expect("activation should encode as JSON");
    activation
        .as_object_mut()
        .expect("activation should be an object")
        .insert(
            "serviceDb".to_string(),
            json!({ "mongoUrl": "mongodb://transient-owner" }),
        );
    let activation: skiff_artifact_model::AssemblyActivationControl =
        serde_json::from_value(activation).expect("legacy activation wire should decode");
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &activation,
    )
    .expect("activation frame should encode");

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
    )
    .await
    .expect_err("transient serviceDb must fail");

    assert!(error
        .to_string()
        .contains("assembly activation serviceDb is not supported"));
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
            "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "replicaId": "runtime-base"
    }))
    .expect("assembly activation control fixture should decode")
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
