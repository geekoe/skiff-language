use std::sync::{Arc, Mutex};

use skiff_artifact_model::{GatewayEntryIdentity, IngressProtocol};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionEventSink, BytecodeExecutionObservation,
};
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::protocol::{
    decode_binary_frame, BytecodeRequestStartFrameWireHeader, ValidatedResponseErrorFrame,
};
use tokio::{sync::mpsc, time::timeout};

use super::phase_0_proof_support::{
    runtime_host, CanonicalSkbfRequest, CorrelatedResponse, Correlation, PublishedFixture,
};

#[derive(Default)]
struct RecordingSink(Mutex<Vec<BytecodeExecutionObservation>>);

impl BytecodeExecutionEventSink for RecordingSink {
    fn observe(&self, observation: BytecodeExecutionObservation) {
        self.0
            .lock()
            .expect("Phase 0 negative recording sink lock")
            .push(observation);
    }
}

impl RecordingSink {
    fn assert_empty(&self, scenario: &str) {
        let observations = self.0.lock().expect("Phase 0 negative recording sink lock");
        assert!(
            observations.is_empty(),
            "{scenario} must fail before production emits an execution observation: {observations:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn phase_0_negative_production_boundaries() {
    corrupt_published_bytecode_identity_fails_before_observation().await;
    wrong_gateway_identity_fails_before_observation().await;
    server_stream_mode_fails_before_observation().await;
}

async fn corrupt_published_bytecode_identity_fails_before_observation() {
    let scenario = "corrupt-bytecode-identity";
    let correlation = Correlation::new(scenario);
    let fixture = PublishedFixture::build(scenario);
    let corruption = fixture.corrupt_bytecode_identity();
    assert_ne!(corruption.before_sha256, corruption.after_sha256);
    assert!(!corruption.record_path.is_empty());

    let bytecode = fixture
        .package_artifact
        .bytecode
        .as_ref()
        .expect("published fixture has bytecode");
    let expected_message = format!(
        "bytecode deployment load failed: deployment load attempt 1 failed: deployment bytecode hydration failed: failed to resolve PackageBytecode {{ package: {:?}, bytecode: {:?} }}: invalid canonical ecosystem record at {}: raw JSON bytecodeIdentity does not match exact reference value {}",
        fixture.package_ref,
        bytecode,
        std::fs::canonicalize(fixture.artifact_root.path().join(&corruption.record_path))
            .expect("canonicalize corrupted bytecode record path")
            .display(),
        bytecode.bytecode_identity,
    );
    run_negative_request(
        scenario,
        correlation,
        fixture,
        |request| request,
        "InternalError",
        expected_message,
    )
    .await;
}

async fn wrong_gateway_identity_fails_before_observation() {
    let scenario = "wrong-gateway-identity";
    let correlation = Correlation::new(scenario);
    let fixture = PublishedFixture::build(scenario);
    let gateway_key = fixture
        .deployment_artifact
        .ingress
        .iter()
        .find(|binding| {
            binding.selector.protocol == IngressProtocol::Http
                && binding.selector.method.as_deref() == Some("POST")
                && binding.selector.path == "/phase-0/vcp"
        })
        .expect("fixture publishes exact HTTP ingress")
        .gateway_entry_key
        .clone();
    let wrong_identity = different_gateway_identity(&fixture.gateway_identity);
    let expected_message = format!(
        "bytecode deployment {} ingress entry {} does not match routed gateway identity {}",
        fixture.deployment.deployment_artifact_identity, gateway_key, wrong_identity,
    );

    run_negative_request(
        scenario,
        correlation,
        fixture,
        move |mut request| {
            let BytecodeRequestStartFrameWireHeader::Http(header) = &mut request.header else {
                panic!("canonical Phase 0 request remains HTTP")
            };
            header.routing.gateway_entry_identity = wrong_identity;
            request
        },
        "InternalError",
        expected_message,
    )
    .await;
}

async fn server_stream_mode_fails_before_observation() {
    let scenario = "server-stream-mode";
    let correlation = Correlation::new(scenario);
    let fixture = PublishedFixture::build(scenario);
    run_negative_request(
        scenario,
        correlation,
        fixture,
        |mut request| {
            let BytecodeRequestStartFrameWireHeader::Http(header) = &mut request.header else {
                panic!("canonical Phase 0 request remains HTTP")
            };
            header.mode = "serverStream".to_string();
            request
        },
        "UnsupportedRuntimeFeature",
        "bytecode HTTP ingress only supports unary request.start, got serverStream".to_string(),
    )
    .await;
}

async fn run_negative_request(
    scenario: &str,
    correlation: Correlation,
    fixture: PublishedFixture,
    mutate: impl FnOnce(CanonicalSkbfRequest) -> CanonicalSkbfRequest,
    expected_code: &str,
    expected_message: String,
) {
    let canonical = fixture.canonical_request(&correlation, "unary");
    assert!(!canonical.frame.is_empty());
    let request = mutate(canonical);
    let bootstrap = fixture.connection_bootstrap();
    let recording = Arc::new(RecordingSink::default());
    let mut host = runtime_host(&correlation);
    host.bytecode_execution_event_sink = recording.clone();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    host.spawn_bytecode_request(
        &correlation.router_session_id,
        request.header,
        request.body,
        &bootstrap,
        sender,
    )
    .await;

    let response = super::phase_0_proof_support::receive_correlated_response(
        &mut receiver,
        &correlation.request_id,
    )
    .await;
    let CorrelatedResponse::Error {
        frame,
        header,
        error,
    } = response
    else {
        panic!("{scenario} must return response.error")
    };
    assert!(!frame.is_empty());
    assert_eq!(header.request_id(), correlation.request_id);
    let ValidatedResponseErrorFrame::Control(error) = error else {
        panic!("{scenario} must return a typed control response.error")
    };
    assert_eq!(error.code, expected_code, "{scenario} error code");
    assert_eq!(error.message, expected_message, "{scenario} error message");
    recording.assert_empty(scenario);
    timeout(
        std::time::Duration::from_secs(10),
        drain_closed_channel_without_second_terminal(&mut receiver, &correlation.request_id),
    )
    .await
    .expect("production sender must close after the terminal response");
    recording.assert_empty(scenario);
}

fn different_gateway_identity(identity: &GatewayEntryIdentity) -> GatewayEntryIdentity {
    let mut value = identity.as_str().as_bytes().to_vec();
    let last = value.last_mut().expect("gateway identity is non-empty");
    *last = if *last == b'0' { b'1' } else { b'0' };
    let value = String::from_utf8(value).expect("gateway identity remains UTF-8");
    let mutated =
        GatewayEntryIdentity::parse(value).expect("mutated gateway identity remains valid");
    assert_ne!(&mutated, identity);
    mutated
}

async fn drain_closed_channel_without_second_terminal(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    request_id: &str,
) {
    while let Some(message) = receiver.recv().await {
        let RouterWriterMessage::Binary(frame) = message else {
            continue;
        };
        let decoded = decode_binary_frame(&frame).expect("decode drained production frame");
        let header = decoded
            .header
            .as_object()
            .expect("drained production frame header is an object");
        let is_same_request =
            header.get("requestId").and_then(serde_json::Value::as_str) == Some(request_id);
        let is_terminal = matches!(
            header.get("type").and_then(serde_json::Value::as_str),
            Some("response.end" | "response.error")
        );
        assert!(
            !(is_same_request && is_terminal),
            "request {request_id} emitted a second terminal frame"
        );
    }
}
