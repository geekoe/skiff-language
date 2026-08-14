#[path = "bytecode_vm_phase_5/fixture.rs"]
mod fixture;

use fixture::{BuildOutcome, FixtureSpec};

#[test]
fn phase_5_stage_sentinel_source_to_admission() {
    let unsupported = FixtureSpec::unsupported_sse().build("s1-unsupported-sse");
    let BuildOutcome::Rejected {
        error_chain,
        release_pointer_absent,
    } = unsupported
    else {
        panic!("std.http.client.sse must remain outside the Phase 5 executable identity set")
    };
    assert!(release_pointer_absent, "rejected SSE published a release pointer");
    assert!(
        error_chain.contains("std.http.client.sse") || error_chain.contains("std.http.sse"),
        "the fail-closed owner must name SSE exactly: {error_chain}"
    );

    match FixtureSpec::positive().build("s1-positive") {
        BuildOutcome::Published(_) => {}
        BuildOutcome::Rejected { error_chain, .. } => panic!(
            "C5 has not admitted the exact request/stream/server-stream carrier: {error_chain}"
        ),
    }
}
