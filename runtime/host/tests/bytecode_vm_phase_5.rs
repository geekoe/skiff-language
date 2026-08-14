#[path = "bytecode_vm_phase_5/fixture.rs"]
mod fixture;
#[path = "bytecode_vm_phase_5/stages.rs"]
mod stages;
#[path = "bytecode_vm_phase_5/tcp_server.rs"]
mod tcp_server;

use fixture::{BuildOutcome, FixtureSpec};

#[test]
fn phase_5_stage_sentinel_source_to_admission() {
    let unsupported = FixtureSpec::unsupported_sse().build("s1-unsupported-sse");
    let unsupported_date = FixtureSpec::unsupported_date_now().build("s1-unsupported-date-now");
    let illegal_stream =
        FixtureSpec::illegal_stream_placement().build("s1-illegal-stream-placement");
    let positive = FixtureSpec::positive().build("s1-positive");
    let mut failures = Vec::new();

    assert_exact_rejection(
        unsupported,
        "std.http.client.sse",
        &["std.http.client.sse", "std.http.sse"],
        &mut failures,
    );
    assert_exact_rejection(
        unsupported_date,
        "core.date.now",
        &["core.date.now", "Date.now"],
        &mut failures,
    );
    assert_exact_rejection(
        illegal_stream,
        "illegal public Stream placement",
        &["Stream", "stream"],
        &mut failures,
    );

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

fn assert_exact_rejection(
    outcome: BuildOutcome,
    label: &str,
    exact_names: &[&str],
    failures: &mut Vec<String>,
) {
    match outcome {
        BuildOutcome::Published(_) => failures.push(format!(
            "{label} published despite remaining outside the Phase 5 executable surface"
        )),
        BuildOutcome::Rejected {
            error_chain,
            release_pointer_absent,
        } => {
            if !release_pointer_absent {
                failures.push(format!("rejected {label} published a release pointer"));
            }
            if !exact_names.iter().any(|name| error_chain.contains(name)) {
                failures.push(format!(
                    "the fail-closed owner did not name {label} exactly: {error_chain}"
                ));
            }
        }
    }
}
