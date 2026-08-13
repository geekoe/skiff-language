#![allow(dead_code, unused_imports)]

use std::time::Duration;

use serde_json::Value;
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::protocol::decode_binary_frame;
use tokio::{sync::mpsc, time::timeout};

mod fixture;
mod spy_heap;

pub(super) use fixture::{
    Phase2FixtureBuild, Phase2PublishedFixture, PHASE2_NEGATIVE_FIXTURE_RELATIVE,
    PHASE2_VCP_FIXTURE_RELATIVE,
};
pub(super) use spy_heap::{
    heap_spy_seam_requirement, HeapSpyEvent, HeapSpyTrace, RecordingVmHeap, SpySlot,
};

pub(super) use super::phase_0_proof_support::{
    receive_correlated_response, CanonicalSkbfRequest, CorrelatedResponse, Correlation,
};

/// Correlation ids for the Phase 2 proof epoch. The Phase 2 harness reuses the
/// Phase 0/1 wire correlation machinery without touching its semantics.
pub(super) fn phase_2_correlation(scenario_id: &str) -> Correlation {
    Correlation {
        router_session_id: format!("skiff-router-session-v1:opaque:phase-2-{scenario_id}"),
        request_id: format!("phase-2-{scenario_id}-request"),
        scenario_id: format!("phase-2-{scenario_id}"),
    }
}

/// Runs one canonical Phase 2 request through the production request entry,
/// collecting the correlated terminal response. Mirrors the Phase 1 helper
/// because Phase 1 keeps its drain helper private; the frame handling is
/// identical production decoding.
pub(super) async fn run_phase_2_request(
    host: &super::RuntimeHost,
    bootstrap: &crate::host::router_session::ConnectionBootstrap,
    correlation: &Correlation,
    request: CanonicalSkbfRequest,
) -> CorrelatedResponse {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let router_session = correlation.router_session_epoch();
    host.spawn_bytecode_request(
        &router_session,
        request.header,
        request.body,
        bootstrap,
        sender,
    )
    .await;

    let response = receive_correlated_response(&mut receiver, &correlation.request_id).await;
    drain_after_terminal(&mut receiver, &correlation.request_id).await;
    response
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
                .expect("decode frame emitted after the Phase 2 correlated terminal");
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
                "Phase 2 production request {request_id} emitted a second terminal frame"
            );
        }
    })
    .await
    .expect("Phase 2 router writer channel did not close after its terminal");
}
