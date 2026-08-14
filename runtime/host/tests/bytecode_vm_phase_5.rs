#[path = "bytecode_vm_phase_5/fixture.rs"]
mod fixture;

use fixture::{BuildOutcome, FixtureSpec};

#[test]
fn phase_5_stage_sentinel_source_to_admission() {
    let unsupported = FixtureSpec::unsupported_sse().build("s1-unsupported-sse");
    let positive = FixtureSpec::positive().build("s1-positive");
    let mut failures = Vec::new();

    match unsupported {
        BuildOutcome::Published(_) => failures.push(
            "std.http.client.sse published despite remaining outside the Phase 5 executable identity set"
                .to_string(),
        ),
        BuildOutcome::Rejected {
            error_chain,
            release_pointer_absent,
        } => {
            if !release_pointer_absent {
                failures.push("rejected SSE published a release pointer".to_string());
            }
            if !(error_chain.contains("std.http.client.sse")
                || error_chain.contains("std.http.sse"))
            {
                failures.push(format!(
                    "the fail-closed owner did not name SSE exactly: {error_chain}"
                ));
            }
        }
    }

    if let BuildOutcome::Rejected { error_chain, .. } = positive {
        failures.push(format!(
            "C5 has not admitted the exact request/stream/server-stream carrier: {error_chain}"
        ));
    }

    assert!(
        failures.is_empty(),
        "S1 production carrier failures:\n- {}",
        failures.join("\n- ")
    );
}
