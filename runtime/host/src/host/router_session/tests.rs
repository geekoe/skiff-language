use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use skiff_artifact_identity::{
    derive_package_test_entrypoint_id, file_ir_identity, package_abi_identity,
    package_build_identity, package_test_build_identity, package_test_entrypoint_local_id,
    publication_abi_identity,
};
use skiff_artifact_model::{
    ConfigAndEffectMetadata, FileIrUnit, MetadataValue, PackageProductionLinkScope,
    PackageTestAssembly, PackageTestAssemblyKind, PackageTestEntrypoint, PackageTestEntrypointKind,
    PackageTestExecutableRef, PackageTestFileIrRef, PackageTestFileLinkScope,
    PackageTestLinkPolicy, PackageTestPackageUnitRef, PackageUnit, FILE_IR_FORMAT_VERSION,
    FILE_IR_OPCODE_TABLE_VERSION, FILE_IR_SCHEMA_VERSION, PACKAGE_TEST_ASSEMBLY_SCHEMA_VERSION,
};

use super::*;
use crate::artifact_cache::PackageTestRuntimeTemplateCache;
use crate::loader::value_sha256;
use skiff_runtime_package_test::PackageTestDispatchSelection;
use skiff_runtime_transport::protocol::{
    encode_binary_frame, PackageTestStartFrameHeader, RequestCancelFrameHeader,
    ResponseChunkFrameHeader, ResponseEndFrameHeader, ResponseErrorFrameHeader,
    ResponseStartFrameHeader, RouterControlFrameHeader, RuntimeCallerFrameHeader,
    RuntimeErrorFramePayload, RuntimeHttpResponseFrameHeader, RuntimeRegisteredFrameHeader,
    RuntimeTraceContextFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};

const PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX: &str =
    "skiff-package-implementation-links-v1:sha256";

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
    test_host_with_artifact_roots(Vec::new())
}

fn test_host_with_artifact_roots(artifact_roots: Vec<PathBuf>) -> super::super::RuntimeHost {
    super::super::RuntimeHost::new(super::super::RuntimeConfig {
        db_provider: test_db_provider(),
        services: Vec::new(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots,
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("runtime host should build")
}

async fn apply_router_control_artifact_roots(
    host: &super::super::RuntimeHost,
    artifact_roots: Vec<PathBuf>,
    generation: &str,
) {
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &RouterControlFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "router.control".to_string(),
            artifact_roots,
            dev_reload: None,
            mode: None,
            generation: Some(generation.to_string()),
            fingerprint: Some(format!("fingerprint:{generation}")),
            service_config: Vec::new(),
            telemetry: None,
            file_backend: None,
        },
        &[],
    )
    .expect("router.control frame should encode");

    dispatch_router_binary_frame(
        host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("router.control should apply artifact roots");
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
async fn binary_package_test_start_fails_closed_without_artifact_roots() {
    let host = test_host();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &PackageTestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "package-test.start".to_string(),
            request_id: "package-test-no-root".to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "gateway".to_string(),
                target: "__skiff.test-dispatch".to_string(),
            },
            package_id: "example.com/pkg".to_string(),
            package_version: "1.0.0".to_string(),
            test_build_identity:
                "skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            entrypoint_id:
                "skiff-package-test-entrypoint-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            activation_id: "skiff-package-test-run-v1:example~com~~pkg:run:1".to_string(),
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: "trace-package-test-no-root".to_string(),
                span_id: "span-package-test-no-root".to_string(),
                parent_span_id: None,
                sampled: Some(true),
            },
            test_effects_enabled: false,
            test_effect_doubles: Default::default(),
        },
        b"payload",
    )
    .expect("package-test.start frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("package-test.start validation error should be returned as response.error");

    let error = match receiver
        .recv()
        .await
        .expect("response.error should be sent")
    {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("response.error should decode");
            assert!(payload.is_empty());
            header.error
        }
        other => panic!("expected binary response.error frame, got {other:?}"),
    };
    assert_eq!(error.code, "InvalidArtifact");
    assert!(error
        .message
        .contains("no artifact roots are configured for package-test dispatch"));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_package_test_start_executes_loaded_test_entrypoint() {
    let fixture = write_package_test_runtime_fixture();
    let host = test_host_with_artifact_roots(vec![fixture.artifact_root_path()]);
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(&fixture.start_header("package-test-executes"), &[])
        .expect("package-test.start frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("package-test.start should spawn runtime execution");

    let message = tokio::time::timeout(tokio::time::Duration::from_secs(1), receiver.recv())
        .await
        .expect("package-test runtime response should not block")
        .expect("package-test runtime response should be sent");
    let (header, payload) = match message {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("response.end should decode");
            (header, payload)
        }
        other => panic!("expected binary response.end frame, got {other:?}"),
    };

    assert_eq!(header.envelope_type, "response.end");
    assert_eq!(header.request_id, "package-test-executes");
    assert!(header.payload_present);
    assert!(!payload.is_empty());
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_package_test_start_returns_before_template_preprocessing_completes() {
    let fixture = write_package_test_runtime_fixture();
    let artifact_roots = vec![fixture.artifact_root_path()];
    let host = test_host_with_artifact_roots(artifact_roots.clone());
    let header = fixture.start_header("package-test-start-nonblocking");
    let cache_key = package_test_template_cache_key(&artifact_roots, &header);
    let template_build_guard = host
        .acquire_package_test_template_build_lock_for_test(cache_key)
        .await;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(&header, &[]).expect("package-test.start frame should encode");

    tokio::time::timeout(
        tokio::time::Duration::from_millis(100),
        dispatch_router_binary_frame(
            &host,
            &frame,
            &sender,
            &mut control,
            &mut artifact_fingerprint,
        ),
    )
    .await
    .expect("package-test.start dispatch should not wait for template preprocessing")
    .expect("package-test.start should submit to executor");
    assert!(
        receiver.try_recv().is_err(),
        "template preprocessing is still blocked, so no response should be available"
    );

    drop(template_build_guard);
    let message = tokio::time::timeout(tokio::time::Duration::from_secs(1), receiver.recv())
        .await
        .expect("package-test runtime response should arrive after unblocking preprocessing")
        .expect("package-test runtime response should be sent");
    let (header, payload) = match message {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("response.end should decode");
            (header, payload)
        }
        other => panic!("expected binary response.end frame, got {other:?}"),
    };

    assert_eq!(header.request_id, "package-test-start-nonblocking");
    assert!(header.payload_present);
    assert!(!payload.is_empty());
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_package_test_start_queues_when_execution_slots_are_full() {
    let fixture = write_package_test_runtime_fixture();
    let host = test_host_with_artifact_roots(vec![fixture.artifact_root_path()]);
    let held_start_permits = host.acquire_all_package_test_start_execution_permits_for_test();
    let header = fixture.start_header("package-test-start-queued");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(&header, &[]).expect("package-test.start frame should encode");

    tokio::time::timeout(
        tokio::time::Duration::from_millis(100),
        dispatch_router_binary_frame(
            &host,
            &frame,
            &sender,
            &mut control,
            &mut artifact_fingerprint,
        ),
    )
    .await
    .expect("package-test.start dispatch should enqueue without waiting for an execution slot")
    .expect("package-test.start should submit to executor");
    assert!(
        receiver.try_recv().is_err(),
        "queued package-test start should not fail while execution slots are full"
    );

    drop(held_start_permits);
    let message = tokio::time::timeout(tokio::time::Duration::from_secs(1), receiver.recv())
        .await
        .expect(
            "queued package-test runtime response should arrive after releasing execution slots",
        )
        .expect("queued package-test runtime response should be sent");
    let (header, payload) = match message {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("response.end should decode");
            (header, payload)
        }
        other => panic!("expected binary response.end frame, got {other:?}"),
    };

    assert_eq!(header.request_id, "package-test-start-queued");
    assert!(header.payload_present);
    assert!(!payload.is_empty());
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_package_test_start_cancelled_while_queued_returns_cancel_error() {
    let fixture = write_package_test_runtime_fixture();
    let host = test_host_with_artifact_roots(vec![fixture.artifact_root_path()]);
    let held_start_permits = host.acquire_all_package_test_start_execution_permits_for_test();
    let header = fixture.start_header("package-test-start-queued-cancel");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let start_frame =
        encode_binary_frame(&header, &[]).expect("package-test.start frame should encode");

    dispatch_router_binary_frame(
        &host,
        &start_frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("package-test.start should submit to executor");
    assert!(
        receiver.try_recv().is_err(),
        "queued package-test start should not respond before cancellation can be observed"
    );

    let cancel_frame = encode_binary_frame(
        &RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.cancel".to_string(),
            request_id: "package-test-start-queued-cancel".to_string(),
            reason: "test_cancel_before_start_execution".to_string(),
        },
        &[],
    )
    .expect("request.cancel frame should encode");
    dispatch_router_binary_frame(
        &host,
        &cancel_frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("queued package-test cancel should be accepted");
    assert!(
        receiver.try_recv().is_err(),
        "queued cancellation is emitted when the start job reaches execution"
    );

    drop(held_start_permits);
    let error = match tokio::time::timeout(tokio::time::Duration::from_secs(1), receiver.recv())
        .await
        .expect("queued package-test cancellation response should arrive")
        .expect("queued package-test cancellation response should be sent")
    {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("response.error should decode");
            assert_eq!(header.request_id, "package-test-start-queued-cancel");
            assert!(payload.is_empty());
            header.error
        }
        other => panic!("expected binary response.error frame, got {other:?}"),
    };

    assert_eq!(error.code, "CancelError");
    assert_eq!(error.message, "request was cancelled");
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_package_test_start_saturation_returns_request_error_without_ws_failure() {
    let host = test_host();
    let _held_permits = host.acquire_all_package_test_start_admission_permits_for_test();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &PackageTestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "package-test.start".to_string(),
            request_id: "package-test-start-saturated".to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "gateway".to_string(),
                target: "__skiff.test-dispatch".to_string(),
            },
            package_id: "example.com/pkg".to_string(),
            package_version: "1.0.0".to_string(),
            test_build_identity:
                "skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            entrypoint_id:
                "skiff-package-test-entrypoint-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            activation_id: "skiff-package-test-run-v1:example~com~~pkg:run:saturated".to_string(),
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: "trace-package-test-saturated".to_string(),
                span_id: "span-package-test-saturated".to_string(),
                parent_span_id: None,
                sampled: Some(true),
            },
            test_effects_enabled: false,
            test_effect_doubles: Default::default(),
        },
        b"payload",
    )
    .expect("package-test.start frame should encode");

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("saturation should be returned as request-level response.error");

    let error = match receiver
        .recv()
        .await
        .expect("saturation response.error should be sent")
    {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("response.error should decode");
            assert_eq!(header.request_id, "package-test-start-saturated");
            assert!(payload.is_empty());
            header.error
        }
        other => panic!("expected binary response.error frame, got {other:?}"),
    };
    assert_eq!(error.code, "ResourceLimitExceeded");
    assert!(error
        .message
        .contains("package-test start executor admission saturated"));
    assert_eq!(
        error
            .details
            .as_ref()
            .and_then(|details| details.get("resource"))
            .and_then(serde_json::Value::as_str),
        Some("packageTestStartExecutor")
    );
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn package_test_runtime_template_first_miss_is_singleflight_per_cache_key() {
    let fixture = write_package_test_runtime_fixture();
    let artifact_roots = vec![fixture.artifact_root_path()];
    let host = test_host_with_artifact_roots(artifact_roots.clone());
    let first_header = fixture.start_header_for(
        "package-test-singleflight-first",
        0,
        "skiff-package-test-run-v1:example~com~~pkg:run:1",
    );
    let second_header = fixture.start_header_for(
        "package-test-singleflight-second",
        1,
        "skiff-package-test-run-v1:example~com~~pkg:run:2",
    );
    let cache_key = package_test_template_cache_key(&artifact_roots, &first_header);
    let template_build_guard = host
        .acquire_package_test_template_build_lock_for_test(cache_key)
        .await;

    let first_host = host.clone();
    let first = tokio::spawn(async move {
        first_host
            .load_package_test_runtime_program(&first_header)
            .await
    });
    let second_host = host.clone();
    let second = tokio::spawn(async move {
        second_host
            .load_package_test_runtime_program(&second_header)
            .await
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    assert_eq!(
        host.package_test_template_build_count_for_test(),
        0,
        "held singleflight guard should block actual template builds"
    );
    assert!(!first.is_finished());
    assert!(!second.is_finished());

    drop(template_build_guard);
    let first_loaded = first
        .await
        .expect("first package-test load task should join")
        .expect("first package-test runtime program should load");
    let second_loaded = second
        .await
        .expect("second package-test load task should join")
        .expect("second package-test runtime program should load");

    assert_eq!(host.package_test_template_build_count_for_test(), 1);
    assert_eq!(host.artifact_caches.package_test_templates.len(), 1);
    assert!(std::sync::Arc::ptr_eq(
        &first_loaded.image,
        &second_loaded.image
    ));
    assert!(std::sync::Arc::ptr_eq(
        &first_loaded.activation,
        &second_loaded.activation
    ));
}

#[tokio::test]
async fn package_test_service_context_loads_local_config() {
    let fixture = write_package_test_runtime_fixture();
    let host = test_host_with_artifact_roots(vec![fixture.artifact_root_path()]);
    let header = fixture.start_header("package-test-local-config");

    let loaded = host
        .load_package_test_runtime_program(&header)
        .await
        .expect("package-test runtime program should load");
    let context = host
        .package_test_service_context(&loaded, &header)
        .expect("package-test service context should load local config");

    assert_eq!(context.service_id, "example.com/pkg");
    assert_eq!(
        context.activation_identity.as_deref(),
        Some(header.activation_id.as_str())
    );
    assert_eq!(context.build_id, header.test_build_identity);
    assert!(context
        .contract_identity
        .starts_with("skiff-protocol-v1:sha256:"));
    assert_eq!(
        context
            .config
            .resolved_config_value()
            .pointer("/app/secret"),
        Some(&json!("router-secret"))
    );
    assert_eq!(
        context
            .config
            .resolved_config_value()
            .pointer("/serviceDb/mongoUrl"),
        Some(&json!("business-config"))
    );
    assert!(
        context.service_db.is_some(),
        "top-level package-test serviceDb should configure DB runtime"
    );
}

#[tokio::test]
async fn package_test_runtime_program_cache_reuses_template_across_entrypoints_and_activations() {
    let fixture = write_package_test_runtime_fixture();
    let host = test_host_with_artifact_roots(vec![fixture.artifact_root_path()]);
    let first_header = fixture.start_header_for(
        "package-test-cache-first",
        0,
        "skiff-package-test-run-v1:example~com~~pkg:run:1",
    );
    let second_header = fixture.start_header_for(
        "package-test-cache-second",
        1,
        "skiff-package-test-run-v1:example~com~~pkg:run:2",
    );

    let first = host
        .load_package_test_runtime_program(&first_header)
        .await
        .expect("first package-test runtime program should load");
    assert_eq!(host.artifact_caches.package_test_templates.len(), 1);
    let second = host
        .load_package_test_runtime_program(&second_header)
        .await
        .expect("second package-test runtime program should load from template cache");

    assert_eq!(host.artifact_caches.package_test_templates.len(), 1);
    assert!(std::sync::Arc::ptr_eq(&first.image, &second.image));
    assert!(std::sync::Arc::ptr_eq(
        &first.activation,
        &second.activation
    ));
    assert_ne!(
        first.dispatch.entrypoint.entrypoint_id,
        second.dispatch.entrypoint.entrypoint_id
    );
    assert_eq!(
        first.dispatch.entrypoint.entrypoint_id,
        fixture.entrypoint_ids[0]
    );
    assert_eq!(
        second.dispatch.entrypoint.entrypoint_id,
        fixture.entrypoint_ids[1]
    );

    let first_context = host
        .package_test_service_context(&first, &first_header)
        .expect("first package-test service context should load");
    let second_context = host
        .package_test_service_context(&second, &second_header)
        .expect("second package-test service context should load");
    assert_eq!(
        first_context.activation_identity.as_deref(),
        Some("skiff-package-test-run-v1:example~com~~pkg:run:1")
    );
    assert_eq!(
        second_context.activation_identity.as_deref(),
        Some("skiff-package-test-run-v1:example~com~~pkg:run:2")
    );
    assert_eq!(
        first_context
            .config
            .resolved_config_value()
            .pointer("/app/secret"),
        Some(&json!("router-secret"))
    );
    assert_eq!(
        second_context
            .config
            .resolved_config_value()
            .pointer("/app/secret"),
        Some(&json!("router-secret-2"))
    );
}

#[tokio::test]
async fn package_test_runtime_program_cache_clears_on_reload_and_root_change() {
    let first_fixture = write_package_test_runtime_fixture();
    let second_fixture = write_package_test_runtime_fixture();
    let host = test_host();

    apply_router_control_artifact_roots(
        &host,
        vec![first_fixture.artifact_root_path()],
        "package-test-cache-root-1",
    )
    .await;
    let first = host
        .load_package_test_runtime_program(&first_fixture.start_header("package-test-root-1"))
        .await
        .expect("first root package-test runtime program should load");
    assert_eq!(host.artifact_caches.package_test_templates.len(), 1);

    apply_router_control_artifact_roots(
        &host,
        vec![second_fixture.artifact_root_path()],
        "package-test-cache-root-2",
    )
    .await;
    assert!(
        host.artifact_caches.package_test_templates.is_empty(),
        "successful reload must invalidate package-test runtime template cache"
    );

    let second = host
        .load_package_test_runtime_program(&second_fixture.start_header("package-test-root-2"))
        .await
        .expect("second root package-test runtime program should load after cache invalidation");
    assert_eq!(host.artifact_caches.package_test_templates.len(), 1);
    assert!(
        !std::sync::Arc::ptr_eq(&first.image, &second.image),
        "root-changing reload must not serve the stale cached package-test image"
    );
    assert!(
        !std::sync::Arc::ptr_eq(&first.activation, &second.activation),
        "root-changing reload must not serve the stale cached package-test activation"
    );
}

#[tokio::test]
async fn package_test_runtime_program_cache_does_not_store_failed_entrypoint_load() {
    let fixture = write_package_test_runtime_fixture();
    let host = test_host_with_artifact_roots(vec![fixture.artifact_root_path()]);
    let mut bad_header = fixture.start_header("package-test-cache-bad-entrypoint");
    bad_header.entrypoint_id =
        "skiff-package-test-entrypoint-v1:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            .to_string();

    let error = host
        .load_package_test_runtime_program(&bad_header)
        .await
        .expect_err("invalid entrypoint should not load");
    assert!(error.to_string().contains("is not listed in assembly"));
    assert!(
        host.artifact_caches.package_test_templates.is_empty(),
        "failed entrypoint dispatch must not populate the package-test template cache"
    );

    host.load_package_test_runtime_program(&fixture.start_header("package-test-cache-valid"))
        .await
        .expect("valid package-test runtime program should load after failed dispatch");
    assert_eq!(host.artifact_caches.package_test_templates.len(), 1);
}

#[tokio::test]
async fn package_test_template_stats_count_shared_runtime_objects() {
    let fixture = write_package_test_runtime_fixture();
    let artifact_roots = vec![fixture.artifact_root_path()];
    let host = test_host_with_artifact_roots(artifact_roots.clone());
    let header = fixture.start_header("package-test-cache-stats");

    let loaded = host
        .load_package_test_runtime_program(&header)
        .await
        .expect("package-test runtime program should load");
    let selection = PackageTestDispatchSelection {
        package_id: header.package_id.clone(),
        package_version: header.package_version.clone(),
        test_build_identity: header.test_build_identity.clone(),
        entrypoint_id: header.entrypoint_id.clone(),
        activation_id: header.activation_id.clone(),
    };
    let cache_key =
        PackageTestRuntimeTemplateCache::cache_key(&artifact_roots, &selection.build_selection());
    let template = host
        .artifact_caches
        .package_test_templates
        .get(&cache_key)
        .expect("package-test runtime template should be cached");
    let shared_runtime_bytes = template.shared_runtime_estimated_size_bytes();
    let stats = host.artifact_caches.stats();

    assert_eq!(stats.package_test_templates.entries, 1);
    assert_eq!(
        stats.package_test_templates.estimated_size_bytes,
        template.estimated_size_bytes()
    );
    assert!(
        shared_runtime_bytes
            >= std::mem::size_of_val(loaded.image.as_ref())
                .saturating_add(std::mem::size_of_val(loaded.activation.as_ref())),
        "shared runtime estimate must include at least the package-test image and activation roots"
    );
    assert!(
        stats.package_test_templates.estimated_size_bytes >= shared_runtime_bytes,
        "package-test template cache stats must include shared runtime object estimates"
    );
    assert!(stats.total_estimated_size_bytes >= stats.package_test_templates.estimated_size_bytes);
}

#[tokio::test]
async fn binary_response_end_completes_pending_outbound_request() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let (response_sender, mut response_receiver) = mpsc::unbounded_channel();
    host.outbound_requests
        .insert("request-outbound-1".to_string(), response_sender)
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
    assert!(host
        .outbound_requests
        .complete("request-outbound-1")
        .is_none());
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
    host.outbound_requests
        .insert("request-outbound-error".to_string(), response_sender)
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
    assert!(host
        .outbound_requests
        .complete("request-outbound-error")
        .is_none());
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
    host.outbound_requests
        .insert("request-outbound-stream".to_string(), response_sender)
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
    host.outbound_requests
        .insert("request-outbound-stream".to_string(), response_sender)
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

fn package_test_template_cache_key(
    artifact_roots: &[PathBuf],
    header: &PackageTestStartFrameHeader,
) -> String {
    let selection = PackageTestDispatchSelection {
        package_id: header.package_id.clone(),
        package_version: header.package_version.clone(),
        test_build_identity: header.test_build_identity.clone(),
        entrypoint_id: header.entrypoint_id.clone(),
        activation_id: header.activation_id.clone(),
    };
    PackageTestRuntimeTemplateCache::cache_key(artifact_roots, &selection.build_selection())
}

struct PackageTestRuntimeFixture {
    artifact_root: TempArtifactRoot,
    package_id: String,
    package_version: String,
    test_build_identity: String,
    entrypoint_ids: Vec<String>,
}

impl PackageTestRuntimeFixture {
    fn artifact_root_path(&self) -> PathBuf {
        self.artifact_root.path().to_path_buf()
    }

    fn start_header(&self, request_id: &str) -> PackageTestStartFrameHeader {
        self.start_header_for(
            request_id,
            0,
            "skiff-package-test-run-v1:example~com~~pkg:run:1",
        )
    }

    fn start_header_for(
        &self,
        request_id: &str,
        entrypoint_index: usize,
        activation_id: &str,
    ) -> PackageTestStartFrameHeader {
        PackageTestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "package-test.start".to_string(),
            request_id: request_id.to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "gateway".to_string(),
                target: "__skiff.test-dispatch".to_string(),
            },
            package_id: self.package_id.clone(),
            package_version: self.package_version.clone(),
            test_build_identity: self.test_build_identity.clone(),
            entrypoint_id: self.entrypoint_ids[entrypoint_index].clone(),
            activation_id: activation_id.to_string(),
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: format!("trace-{request_id}"),
                span_id: format!("span-{request_id}"),
                parent_span_id: None,
                sampled: Some(true),
            },
            test_effects_enabled: false,
            test_effect_doubles: Default::default(),
        }
    }
}

fn write_package_test_runtime_fixture() -> PackageTestRuntimeFixture {
    let artifact_root = unique_temp_dir();
    let package_id = "example.com/pkg".to_string();
    let package_version = "1.0.0".to_string();
    let package_storage = "example~com~~pkg";

    let mut production_package = PackageUnit::empty(&package_id, &package_version, "", "");
    production_package.publication_abi.abi_identity =
        publication_abi_identity(&production_package.publication_abi)
            .expect("publication ABI identity");
    production_package.abi_identity =
        package_abi_identity(&production_package).expect("package ABI identity");
    production_package.build_identity =
        package_build_identity(&production_package).expect("package build identity");
    let package_build_hash = identity_hash(&production_package.build_identity);
    let package_unit_path = PathBuf::from("units")
        .join("packages")
        .join(package_storage)
        .join(format!("{package_build_hash}.json"));
    write_json_artifact(
        artifact_root.path(),
        &package_unit_path,
        &production_package,
    );

    let mut test_file = package_test_file_ir_fixture();
    test_file.file_ir_identity = file_ir_identity(&test_file).expect("test file identity");
    let test_file_hash = identity_hash(&test_file.file_ir_identity);
    let test_file_path = PathBuf::from("units")
        .join("files")
        .join(format!("{test_file_hash}.json"));
    write_json_artifact(artifact_root.path(), &test_file_path, &test_file);

    let owner_test_file = PackageTestFileIrRef {
        file_ir_identity: test_file.file_ir_identity.clone(),
        file_ir_path: relative_path_string(&test_file_path),
        source_path: "pkg.test.skiff".to_string(),
        module_path: test_file.module_path.clone(),
    };
    let first_entrypoint_local_id = package_test_entrypoint_local_id(
        &package_id,
        &package_version,
        &owner_test_file.source_path,
        0,
        "returns package test ok",
    )
    .expect("first package test entrypoint local id");
    let second_entrypoint_local_id = package_test_entrypoint_local_id(
        &package_id,
        &package_version,
        &owner_test_file.source_path,
        1,
        "returns second package test ok",
    )
    .expect("second package test entrypoint local id");
    let entrypoint_local_ids = vec![
        first_entrypoint_local_id.clone(),
        second_entrypoint_local_id.clone(),
    ];
    let mut config_and_effect_metadata = ConfigAndEffectMetadata::default();
    config_and_effect_metadata.config.insert(
        "shape".to_string(),
        MetadataValue::from_json(json!({
            "schemaVersion": "skiff-config-shape-v1",
            "entries": [
                { "path": "app.secret", "type": "string", "required": true },
                { "path": "serviceDb.mongoUrl", "type": "string", "required": false }
            ]
        })),
    );
    let mut assembly = PackageTestAssembly {
        schema_version: PACKAGE_TEST_ASSEMBLY_SCHEMA_VERSION.to_string(),
        kind: PackageTestAssemblyKind::PackageTest,
        package_id: package_id.clone(),
        package_version: package_version.clone(),
        test_build_identity:
            "skiff-package-test-build-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
        production_package_unit: PackageTestPackageUnitRef {
            package_id: package_id.clone(),
            version: package_version.clone(),
            build_identity: production_package.build_identity.clone(),
            unit_path: relative_path_string(&package_unit_path),
            public_abi_identity: production_package.abi_identity.clone(),
            implementation_links_identity: package_implementation_links_identity(
                &production_package,
            ),
        },
        test_files: vec![owner_test_file.clone()],
        dependency_package_units: Vec::new(),
        test_entrypoints: vec![
            PackageTestEntrypoint {
                kind: PackageTestEntrypointKind::TestOnly,
                entrypoint_local_id: first_entrypoint_local_id.clone(),
                entrypoint_id:
                    "skiff-package-test-entrypoint-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                display_name: "returns package test ok".to_string(),
                source_path: owner_test_file.source_path.clone(),
                module_path: owner_test_file.module_path.clone(),
                owner_test_file: owner_test_file.clone(),
                executable_ref: PackageTestExecutableRef {
                    file_ir_identity: owner_test_file.file_ir_identity.clone(),
                    executable_index: 0,
                    executable_local_id: "entrypoint-0".to_string(),
                    symbol: Some("__skiff_package_test_0".to_string()),
                },
                default_run: true,
                config_and_effect_metadata: config_and_effect_metadata.clone(),
                runtime_expected_error: None,
            },
            PackageTestEntrypoint {
                kind: PackageTestEntrypointKind::TestOnly,
                entrypoint_local_id: second_entrypoint_local_id.clone(),
                entrypoint_id:
                    "skiff-package-test-entrypoint-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                display_name: "returns second package test ok".to_string(),
                source_path: owner_test_file.source_path.clone(),
                module_path: owner_test_file.module_path.clone(),
                owner_test_file: owner_test_file.clone(),
                executable_ref: PackageTestExecutableRef {
                    file_ir_identity: owner_test_file.file_ir_identity.clone(),
                    executable_index: 1,
                    executable_local_id: "entrypoint-1".to_string(),
                    symbol: Some("__skiff_package_test_1".to_string()),
                },
                default_run: false,
                config_and_effect_metadata: config_and_effect_metadata.clone(),
                runtime_expected_error: None,
            },
        ],
        link_policy: PackageTestLinkPolicy {
            current_package_production: PackageProductionLinkScope {
                package_id: package_id.clone(),
                version: package_version.clone(),
                build_identity: production_package.build_identity.clone(),
                files_digest: canonical_digest(&production_package.files),
                implementation_links_digest: canonical_digest(
                    &production_package.implementation_links,
                ),
                allow_private: true,
            },
            test_file_scopes: vec![PackageTestFileLinkScope {
                owner_test_file_identity: owner_test_file.file_ir_identity.clone(),
                source_path: owner_test_file.source_path.clone(),
                module_path: owner_test_file.module_path.clone(),
                allowed_local_link_digest: package_test_allowed_local_link_digest(
                    &owner_test_file,
                    &test_file,
                    &entrypoint_local_ids,
                ),
                entrypoint_local_ids,
            }],
            dependency_public_scopes: Vec::new(),
        },
        config_and_effect_metadata,
        source_map: json!({}),
    };
    assembly.test_build_identity =
        package_test_build_identity(&assembly).expect("package test build identity");
    let entrypoint_ids = vec![
        derive_package_test_entrypoint_id(
            &assembly.test_build_identity,
            &first_entrypoint_local_id,
        )
        .expect("first package test entrypoint identity"),
        derive_package_test_entrypoint_id(
            &assembly.test_build_identity,
            &second_entrypoint_local_id,
        )
        .expect("second package test entrypoint identity"),
    ];
    for (entrypoint, entrypoint_id) in assembly.test_entrypoints.iter_mut().zip(&entrypoint_ids) {
        entrypoint.entrypoint_id = entrypoint_id.clone();
    }

    let test_build_hash = identity_hash(&assembly.test_build_identity);
    let assembly_path = PathBuf::from("assemblies")
        .join("package-tests")
        .join(package_storage)
        .join(format!("{test_build_hash}.json"));
    let pointer_path = PathBuf::from("dev")
        .join("package-tests")
        .join(package_storage)
        .join(format!("{test_build_hash}.json"));
    write_json_artifact(artifact_root.path(), &assembly_path, &assembly);
    write_json_artifact(
        artifact_root.path(),
        &pointer_path,
        &json!({
            "schemaVersion": "skiff-package-test-dev-pointer-v1",
            "packageId": package_id,
            "packageVersion": package_version,
            "testBuildIdentity": assembly.test_build_identity,
            "packageTestAssembly": {
                "assemblyPath": relative_path_string(&assembly_path),
                "assemblyIdentity": assembly.test_build_identity
            }
        }),
    );
    write_yaml_artifact(
        artifact_root.path(),
        &PathBuf::from("configs")
            .join("package-tests")
            .join("skiff-package-test-run-v1:example~com~~pkg:run:1")
            .join("config.yml"),
        &json!({
            "serviceDb": {
                "mongoUrl": "mongodb://127.0.0.1:27017/router-session-package-test"
            },
            "service": {
                "app": {
                    "secret": "router-secret"
                },
                "serviceDb": {
                    "mongoUrl": "business-config"
                }
            }
        }),
    );
    write_yaml_artifact(
        artifact_root.path(),
        &PathBuf::from("configs")
            .join("package-tests")
            .join("skiff-package-test-run-v1:example~com~~pkg:run:2")
            .join("config.yml"),
        &json!({
            "serviceDb": {
                "mongoUrl": "mongodb://127.0.0.1:27017/router-session-package-test-2"
            },
            "service": {
                "app": {
                    "secret": "router-secret-2"
                },
                "serviceDb": {
                    "mongoUrl": "business-config-2"
                }
            }
        }),
    );

    PackageTestRuntimeFixture {
        artifact_root,
        package_id: "example.com/pkg".to_string(),
        package_version: "1.0.0".to_string(),
        test_build_identity: assembly.test_build_identity,
        entrypoint_ids,
    }
}

fn package_test_file_ir_fixture() -> FileIrUnit {
    serde_json::from_value(json!({
        "schemaVersion": FILE_IR_SCHEMA_VERSION,
        "fileIrIdentity": "",
        "sourceAstHash": "source:package-test",
        "modulePath": "pkg.test",
        "irFormatVersion": FILE_IR_FORMAT_VERSION,
        "opcodeTableVersion": FILE_IR_OPCODE_TABLE_VERSION,
        "sourceMap": {
            "format": "skiff-file-ir-source-map-v1",
            "sources": [],
            "spans": []
        },
        "declarations": {
            "interfaces": {},
            "executables": {
                "__skiff_package_test_0": {
                    "executableIndex": 0,
                    "symbol": "__skiff_package_test_0"
                },
                "__skiff_package_test_1": {
                    "executableIndex": 1,
                    "symbol": "__skiff_package_test_1"
                }
            }
        },
        "linkTargets": {},
        "typeTable": [],
        "constants": [],
        "executables": [
            {
                "kind": "function",
                "symbol": "__skiff_package_test_0",
                "typeParams": [],
                "params": [],
                "returnType": { "kind": "builtin", "name": "string", "args": [] },
                "slots": { "slots": [], "frameSize": 0 },
                "maySuspend": false,
                "body": {
                    "blocks": [
                        {
                            "label": "entry",
                            "statements": [{ "statement": 0 }]
                        }
                    ],
                    "statements": [
                        {
                            "kind": "return",
                            "value": { "expression": 0 }
                        }
                    ],
                    "expressions": [
                        {
                            "kind": "literal",
                            "value": { "kind": "string", "value": "package-test-ok" }
                        }
                    ]
                }
            },
            {
                "kind": "function",
                "symbol": "__skiff_package_test_1",
                "typeParams": [],
                "params": [],
                "returnType": { "kind": "builtin", "name": "string", "args": [] },
                "slots": { "slots": [], "frameSize": 0 },
                "maySuspend": false,
                "body": {
                    "blocks": [
                        {
                            "label": "entry",
                            "statements": [{ "statement": 0 }]
                        }
                    ],
                    "statements": [
                        {
                            "kind": "return",
                            "value": { "expression": 0 }
                        }
                    ],
                    "expressions": [
                        {
                            "kind": "literal",
                            "value": { "kind": "string", "value": "package-test-ok-2" }
                        }
                    ]
                }
            }
        ],
        "externalRefs": {}
    }))
    .expect("package-test File IR fixture should deserialize")
}

fn write_json_artifact<T: serde::Serialize>(root: &Path, relative_path: &Path, value: &T) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().expect("artifact path should have parent"))
        .expect("artifact parent should be created");
    fs::write(
        &path,
        serde_json::to_vec_pretty(value).expect("artifact should serialize"),
    )
    .expect("artifact should be written");
}

fn write_yaml_artifact<T: serde::Serialize>(root: &Path, relative_path: &Path, value: &T) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().expect("artifact path should have parent"))
        .expect("artifact parent should be created");
    fs::write(
        &path,
        serde_yaml::to_string(value).expect("artifact YAML should serialize"),
    )
    .expect("artifact should be written");
}

fn relative_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn package_test_allowed_local_link_digest(
    reference: &PackageTestFileIrRef,
    file: &FileIrUnit,
    entrypoint_local_ids: &[String],
) -> String {
    let mut entrypoint_local_ids = entrypoint_local_ids.to_vec();
    entrypoint_local_ids.sort();
    entrypoint_local_ids.dedup();
    value_sha256(&json!({
        "fileIrIdentity": reference.file_ir_identity,
        "sourcePath": reference.source_path,
        "modulePath": reference.module_path,
        "entrypointLocalIds": entrypoint_local_ids,
        "localTargets": {
            "declarations": &file.declarations,
            "linkTargets": &file.link_targets,
            "typeCount": file.type_table.len(),
            "constCount": file.constants.len(),
            "executableCount": file.executables.len(),
        },
    }))
    .expect("test file link scope digest")
}

fn package_implementation_links_identity(unit: &PackageUnit) -> String {
    format!(
        "{PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX}:{}",
        canonical_digest(&unit.implementation_links)
    )
}

fn canonical_digest<T: serde::Serialize>(value: &T) -> String {
    let value = serde_json::to_value(value).expect("artifact projection should serialize");
    value_sha256(&value).expect("artifact projection should hash")
}

fn identity_hash(identity: &str) -> &str {
    identity
        .rsplit_once(':')
        .map(|(_, hash)| hash)
        .expect("identity should contain hash suffix")
}

struct TempArtifactRoot {
    path: PathBuf,
}

impl TempArtifactRoot {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempArtifactRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_temp_dir() -> TempArtifactRoot {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic enough")
        .as_nanos();
    for attempt in 0..1000 {
        let path = std::env::temp_dir().join(format!(
            "skiff-runtime-router-package-test-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return TempArtifactRoot { path },
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("temp dir should be created: {error}"),
        }
    }
    panic!("failed to allocate unique package-test temp dir after 1000 attempts");
}
