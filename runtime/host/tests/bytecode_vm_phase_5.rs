#[path = "bytecode_vm_phase_5/fixture.rs"]
mod fixture;
#[path = "bytecode_vm_phase_5/host_chain.rs"]
mod host_chain;
#[path = "bytecode_vm_phase_5/host_harness.rs"]
mod host_harness;
#[path = "bytecode_vm_phase_5/runtime.rs"]
mod runtime;
#[path = "bytecode_vm_phase_5/stages.rs"]
mod stages;
#[path = "bytecode_vm_phase_5/tcp_server.rs"]
mod tcp_server;

use skiff_artifact_model::BoundaryUnavailableReason;
use skiff_compiler::Phase1UnsupportedCapability;

use fixture::{BuildOutcome, FixtureSpec, TypedRejection};

#[test]
fn phase_5_stage_sentinel_source_to_admission() {
    let unsupported = FixtureSpec::unsupported_sse().build("s1-unsupported-sse");
    let unsupported_date = FixtureSpec::unsupported_date_now().build("s1-unsupported-date-now");
    let illegal_stream =
        FixtureSpec::illegal_stream_placement().build("s1-illegal-stream-placement");
    let positive = FixtureSpec::positive().build("s1-positive");
    let mut failures = Vec::new();

    assert_phase1_rejection(
        unsupported,
        "std.http.client.sse",
        Phase1UnsupportedCapability::HostTarget,
        "main::run",
        &mut failures,
    );
    assert_phase1_rejection(
        unsupported_date,
        "core.date.now",
        Phase1UnsupportedCapability::HostTarget,
        "main::run",
        &mut failures,
    );
    assert_public_stream_rejection(illegal_stream, "leak", &mut failures);

    if let BuildOutcome::Rejected { error_chain, .. } = positive {
        failures.push(format!(
            "C5 has not admitted the exact request/stream/server-stream carrier: {error_chain}"
        ));
    }

    if let Ok(path) = std::env::var("SKIFF_BYTECODE_VM_PHASE5_CARRIER_ROOT") {
        println!("phase-5-router-carrier={path}");
    }

    assert!(
        failures.is_empty(),
        "S1 production carrier failures:\n- {}",
        failures.join("\n- ")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_stage_sentinel_verify_to_scheduler() {
    runtime::verify_to_scheduler().await;
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_stage_sentinel_scheduler_to_request_response() {
    host_chain::scheduler_to_request_response().await;
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_vcp_production_composition() {
    host_chain::vcp_production_composition().await;
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_lifecycle_race_matrix() {
    runtime::lifecycle_race_matrix().await;
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_single_worker_canary() {
    runtime::single_worker_canary().await;
}

#[tokio::test(flavor = "current_thread")]
async fn phase_5_structural_no_bypass() {
    // Reuse the actual production authoring boundary for every positive and
    // fail-closed companion. The RuntimeHost leg below reloads that published
    // deployment; neither half constructs an artifact, image, executor,
    // resource handle, or response frame for the VM.
    phase_5_stage_sentinel_source_to_admission();
    host_chain::structural_no_bypass().await;
}

fn assert_phase1_rejection(
    outcome: BuildOutcome,
    label: &str,
    expected_capability: Phase1UnsupportedCapability,
    expected_function: &str,
    failures: &mut Vec<String>,
) {
    match outcome {
        BuildOutcome::Published(_) => failures.push(format!(
            "{label} published despite remaining outside the Phase 5 executable surface"
        )),
        BuildOutcome::Rejected {
            error_chain,
            package_pointer_absent,
            release_pointer_absent,
            rejection,
        } => {
            if !package_pointer_absent {
                failures.push(format!(
                    "rejected {label} published a PackageArtifact pointer"
                ));
            }
            if !release_pointer_absent {
                failures.push(format!("rejected {label} published a release pointer"));
            }
            if !matches!(
                &rejection,
                Some(TypedRejection::Phase1Capability {
                    capability,
                    module_path,
                    function_key: Some(function_key),
                }) if *capability == expected_capability
                    && module_path == "main"
                    && function_key == expected_function
            ) {
                failures.push(format!(
                    "the exact {label} fixture did not reach its typed {expected_capability:?} owner: {rejection:?}; diagnostic={error_chain}"
                ));
            }
        }
    }
}

fn assert_public_stream_rejection(
    outcome: BuildOutcome,
    public_path: &str,
    failures: &mut Vec<String>,
) {
    match outcome {
        BuildOutcome::Published(_) => failures.push(format!(
            "illegal public Stream path {public_path} published despite remaining outside Phase 5"
        )),
        BuildOutcome::Rejected {
            error_chain,
            package_pointer_absent,
            release_pointer_absent,
            rejection,
        } => {
            if !package_pointer_absent {
                failures.push(format!(
                    "rejected public Stream path {public_path} published a PackageArtifact pointer"
                ));
            }
            if !release_pointer_absent {
                failures.push(format!(
                    "rejected public Stream path {public_path} published a release pointer"
                ));
            }
            let exact = matches!(
                &rejection,
                Some(TypedRejection::UnavailableServiceCalls { unavailable })
                    if unavailable.get(public_path).is_some_and(|reasons| {
                        reasons.contains(&BoundaryUnavailableReason::UnsupportedStream)
                    })
            );
            if !exact {
                failures.push(format!(
                    "public Stream path {public_path} did not fail at the typed boundary owner: {rejection:?}; diagnostic={error_chain}"
                ));
            }
        }
    }
}
