#![allow(dead_code, unused_imports)]

mod fixture;
mod request_composition;

pub(super) use fixture::{
    Phase3FixtureBuild, Phase3PublishedFixture, PHASE3_HOST_THROW_FIXTURE_RELATIVE,
    PHASE3_MISMATCH_FIXTURE_RELATIVE, PHASE3_PENDING_THROW_FIXTURE_RELATIVE,
    PHASE3_UNCAUGHT_FIXTURE_RELATIVE, PHASE3_VCP_FIXTURE_RELATIVE,
};
pub(super) use request_composition::drive_phase_3_vcp_request;

// The Phase 3 harness reuses the Phase 2 heap spy (the same `VmHeap` recorder,
// event stream and `host_passthrough_note`) without touching its semantics:
// unwind cleanup-owner release and rethrow envelope identity are still proven
// through the exact production share/transfer/release primitive sequence.
pub(super) use super::phase_0_proof_support::{
    receive_correlated_response, CanonicalSkbfRequest, CorrelatedResponse, Correlation,
};
pub(super) use super::phase_2_proof_support::{
    host_passthrough_note, HeapSpyEvent, HeapSpyTrace, RecordingVmHeap, SpySlot,
};

/// Correlation ids for the Phase 3 proof epoch. The Phase 3 harness reuses the
/// Phase 0/1 wire correlation machinery without touching its semantics.
pub(super) fn phase_3_correlation(scenario_id: &str) -> Correlation {
    Correlation {
        router_session_id: format!("skiff-router-session-v1:opaque:phase-3-{scenario_id}"),
        request_id: format!("phase-3-{scenario_id}-request"),
        scenario_id: format!("phase-3-{scenario_id}"),
    }
}

/// Runs one canonical Phase 3 request through the production request entry,
/// collecting the correlated terminal response. The frame handling is the
/// exact Phase 2 production helper, reused verbatim through the same wire
/// correlation machinery.
pub(super) async fn run_phase_3_request(
    host: &super::RuntimeHost,
    bootstrap: &crate::host::router_session::ConnectionBootstrap,
    correlation: &Correlation,
    request: CanonicalSkbfRequest,
) -> CorrelatedResponse {
    super::phase_2_proof_support::run_phase_2_request(host, bootstrap, correlation, request).await
}
