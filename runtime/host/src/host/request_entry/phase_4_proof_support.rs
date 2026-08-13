#![allow(dead_code, unused_imports)]

use std::time::Duration;

use serde_json::Value;
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::protocol::decode_binary_frame;
use tokio::{sync::mpsc, time::timeout};

mod fixture;
mod observation;
mod request_composition;

pub(super) use fixture::{Phase4FixtureBuild, Phase4PublishedFixture, PHASE4_VCP_FIXTURE_RELATIVE};
pub(super) use observation::RecordingSink;
pub(super) use request_composition::{
    drive_phase_4_vcp_request, park_phase_4_request, resume_phase_4_parked, Phase4DriveEvidence,
};

// The Phase 4 harness reuses the Phase 0/1 wire correlation machinery, the
// Phase 2 recording heap spy and the Phase 3 fixture-publication shape without
// touching their semantics. The Phase 4-specific seams are the production
// request driver boundary (SEAM-4, see request_composition.rs) and the
// production observation sink for single-terminal facts.
pub(super) use super::phase_0_proof_support::{
    receive_correlated_response, runtime_host, CanonicalSkbfRequest, CorrelatedResponse,
    Correlation,
};
pub(super) use super::phase_2_proof_support::{
    host_passthrough_note, HeapSpyEvent, HeapSpyTrace, RecordingVmHeap, SpySlot,
};

/// Correlation ids for the Phase 4 proof epoch. The Phase 4 harness reuses the
/// Phase 0/1 wire correlation machinery without touching its semantics.
pub(super) fn phase_4_correlation(scenario_id: &str) -> Correlation {
    Correlation {
        router_session_id: format!("skiff-router-session-v1:opaque:phase-4-{scenario_id}"),
        request_id: format!("phase-4-{scenario_id}-request"),
        scenario_id: format!("phase-4-{scenario_id}"),
    }
}

/// Spawns one canonical Phase 4 request through the production request entry
/// and returns the live router-writer receiver, so a negative can race
/// cancel/deadline/session-stop against the parked request before collecting
/// its terminal.
pub(super) async fn spawn_phase_4_request(
    host: &super::RuntimeHost,
    bootstrap: &crate::host::router_session::ConnectionBootstrap,
    correlation: &Correlation,
    request: CanonicalSkbfRequest,
) -> mpsc::UnboundedReceiver<RouterWriterMessage> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let router_session = correlation.router_session_epoch();
    host.spawn_bytecode_request(
        &router_session,
        request.header,
        request.body,
        bootstrap,
        sender,
    )
    .await;
    receiver
}

/// Runs one canonical Phase 4 request through the production request entry,
/// collecting the correlated terminal response and asserting it is the only
/// correlated terminal the request emits. Mirrors the Phase 2 helper because
/// Phase 1 keeps its drain helper private; the frame handling is identical
/// production decoding.
pub(super) async fn run_phase_4_request(
    host: &super::RuntimeHost,
    bootstrap: &crate::host::router_session::ConnectionBootstrap,
    correlation: &Correlation,
    request: CanonicalSkbfRequest,
) -> CorrelatedResponse {
    let mut receiver = spawn_phase_4_request(host, bootstrap, correlation, request).await;
    let response = receive_correlated_response(&mut receiver, &correlation.request_id).await;
    drain_after_terminal(&mut receiver, &correlation.request_id).await;
    response
}

/// Waits for the correlated request to finish without a wire terminal frame.
/// The supervisor-level terminal fact is the observation evidence: a
/// cancel/session-stop terminal performs `StopWithoutResponse`, so the router
/// writer channel must close with zero correlated response frames.
pub(super) async fn await_terminal_without_response(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    request_id: &str,
) {
    timeout(Duration::from_secs(10), async {
        while let Some(message) = receiver.recv().await {
            let RouterWriterMessage::Binary(frame) = message else {
                continue;
            };
            let decoded = decode_binary_frame(&frame)
                .expect("decode frame emitted after the Phase 4 terminal");
            let Some(header) = decoded.header.as_object() else {
                continue;
            };
            let same_request = header.get("requestId").and_then(Value::as_str) == Some(request_id);
            let terminal = matches!(
                header.get("type").and_then(Value::as_str),
                Some("response.end" | "response.error")
            );
            assert!(
                !(same_request && terminal),
                "Phase 4 request {request_id} emitted a wire terminal despite a \
                 stop-without-response terminal"
            );
        }
    })
    .await
    .expect("Phase 4 router writer channel did not close after its stop-without-response terminal");
}

async fn drain_after_terminal(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    request_id: &str,
) {
    timeout(Duration::from_secs(10), async {
        while let Some(message) = receiver.recv().await {
            let RouterWriterMessage::Binary(frame) = message else {
                continue;
            };
            let decoded = decode_binary_frame(&frame)
                .expect("decode frame emitted after the Phase 4 correlated terminal");
            let Some(header) = decoded.header.as_object() else {
                continue;
            };
            let same_request = header.get("requestId").and_then(Value::as_str) == Some(request_id);
            let terminal = matches!(
                header.get("type").and_then(Value::as_str),
                Some("response.end" | "response.error")
            );
            assert!(
                !(same_request && terminal),
                "Phase 4 production request {request_id} emitted a second terminal frame"
            );
        }
    })
    .await
    .expect("Phase 4 router writer channel did not close after its terminal");
}
