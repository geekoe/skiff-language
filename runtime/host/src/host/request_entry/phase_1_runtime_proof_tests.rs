use std::sync::Arc;

use serde_json::Value;
use skiff_runtime_transport::protocol::{
    BytecodeRequestDeadlineFrameHeader, BytecodeRequestStartFrameWireHeader,
    ValidatedResponseErrorFrame,
};

use super::{
    phase_0_proof_support::{runtime_host, CorrelatedResponse, PublishedFixture, FIXTURE_RELATIVE},
    phase_1_runtime_proof_support::{
        phase_1_correlation, phase_1_observation_gaps, run_phase_1_request, shared_sink,
        Phase1RecordingSink,
    },
};

#[tokio::test(flavor = "current_thread")]
async fn phase_1_runtime_vcp_and_expected_red_obligations() {
    let fixture = PublishedFixture::build("phase-1-runtime-proof");
    let bootstrap = fixture.connection_bootstrap();
    let host_identity = phase_1_correlation("host");
    let recording = Arc::new(Phase1RecordingSink::default());
    let mut host = runtime_host(&host_identity);
    host.bytecode_execution_event_sink = shared_sink(&recording);

    let mut expected_red = Vec::new();
    prove_scalar_local_call_vcp(&host, &bootstrap, &fixture, &recording, &mut expected_red).await;
    prove_expired_budget_is_a_single_terminal(&host, &bootstrap, &fixture, &recording).await;

    assert!(
        expected_red.is_empty(),
        "Phase 1 runtime proof must be fully green after the O1 projection joins:\n- {}",
        expected_red.join("\n- ")
    );
}

async fn prove_scalar_local_call_vcp(
    host: &super::RuntimeHost,
    bootstrap: &crate::host::router_session::ConnectionBootstrap,
    fixture: &PublishedFixture,
    recording: &Phase1RecordingSink,
    expected_red: &mut Vec<String>,
) {
    let correlation = phase_1_correlation("scalar-local-call-success");
    assert!(
        host.request_supervisor
            .start_session(correlation.router_session_epoch()),
        "the scalar proof session must be open before its request is reserved"
    );
    let request = fixture.canonical_request(&correlation, "unary");
    assert_eq!(request.body, b"2", "the real wire scalar is helper(2)");
    assert_canonical_scalar_fixture();

    let response = run_phase_1_request(host, bootstrap, &correlation, request).await;
    let CorrelatedResponse::End { header, body, .. } = response else {
        panic!("Phase 1 scalar/local-call VCP must return response.end")
    };
    assert_eq!(header.request_id, correlation.request_id);
    assert!(header.payload_present);
    assert_eq!(body, b"3.0");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).expect("decode Phase 1 scalar response"),
        serde_json::json!(3.0)
    );

    let observations = recording.for_correlation(&correlation);
    expected_red.extend(phase_1_observation_gaps(&observations));
}

fn assert_canonical_scalar_fixture() {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(FIXTURE_RELATIVE)
        .join("main.skiff");
    let source = std::fs::read_to_string(source_path).expect("read accepted scalar VCP source");
    assert!(source.contains("final result = helper(value)"));
    assert!(source.contains("if result == 7"));
    assert!(source.contains("return result - 4"));
}

async fn prove_expired_budget_is_a_single_terminal(
    host: &super::RuntimeHost,
    bootstrap: &crate::host::router_session::ConnectionBootstrap,
    fixture: &PublishedFixture,
    recording: &Phase1RecordingSink,
) {
    let correlation = phase_1_correlation("expired-deadline");
    let mut request = fixture.canonical_request(&correlation, "unary");
    let BytecodeRequestStartFrameWireHeader::Http(header) = &mut request.header else {
        panic!("canonical scalar fixture remains HTTP")
    };
    header.deadline = Some(BytecodeRequestDeadlineFrameHeader {
        timeout_ms: 0,
        expires_at: "2999-01-01T00:00:00Z".to_string(),
    });
    let response = run_phase_1_request(host, bootstrap, &correlation, request).await;
    assert_control_error(
        response,
        "TimeoutError",
        "execution deadline exceeded",
        Some(("reason", serde_json::json!("deadlineExceeded"))),
    );
    assert!(
        recording.for_correlation(&correlation).is_empty(),
        "expired deadline must stop before image/entry/VM ownership"
    );
}

fn assert_control_error(
    response: CorrelatedResponse,
    expected_code: &str,
    expected_message: &str,
    expected_detail: Option<(&str, Value)>,
) {
    let CorrelatedResponse::Error { error, .. } = response else {
        panic!("expected a correlated response.error")
    };
    let ValidatedResponseErrorFrame::Control(error) = error else {
        panic!("expected a typed control response.error")
    };
    assert_eq!(
        error.code, expected_code,
        "unexpected Phase 1 control error: {error:?}"
    );
    assert_eq!(
        error.message, expected_message,
        "unexpected Phase 1 control error: {error:?}"
    );
    if let Some((name, expected)) = expected_detail {
        assert_eq!(
            error.details.as_ref().and_then(|details| details.get(name)),
            Some(&expected)
        );
    }
}
