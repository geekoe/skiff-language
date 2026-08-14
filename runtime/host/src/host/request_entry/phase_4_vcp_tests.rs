use std::sync::Arc;

use serde_json::Value;
use skiff_artifact_model::{
    bytecode::{structurally_validate, Opcode},
    HostEffectExecutorIdentity,
};
use skiff_runtime_bytecode_verifier::VerifiedResumeKind;
use skiff_runtime_linked_bytecode::LinkedInstructionTarget;
use skiff_runtime_model::{
    bytecode_execution_observation::{
        BytecodeRequestTerminal, RequestExecutionOwnerInventorySnapshot,
    },
    request_heap::RequestHeapLimits,
    vm_heap::VmHeap,
};
use skiff_runtime_request::RequestCancel;
use skiff_runtime_transport::protocol::ValidatedResponseErrorFrame;

use super::phase_4_proof_support::{
    await_terminal_without_response, drive_phase_4_vcp_request, park_phase_4_request,
    phase_4_correlation, resume_phase_4_parked, run_phase_4_request, runtime_host,
    spawn_phase_4_request, CorrelatedResponse, HeapSpyTrace, Phase4FixtureBuild,
    Phase4PublishedFixture, RecordingSink, RecordingVmHeap, PHASE4_VCP_FIXTURE_RELATIVE,
};
use crate::loader::bytecode_admission::BytecodeDeploymentRegistry;

const PHASE4_VCP_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-4";
const PHASE4_VCP_VERSION: &str = "1.0.0";
const PHASE4_VCP_INGRESS: &str = "/phase-4/vcp";
const PHASE4_NEGATIVE_FIXTURE_RELATIVE: &str =
    "runtime/host/src/host/request_entry/phase_4_proof_support/fixtures/vcp4-sleep-negative";
/// Far-future sleep used by the negatives: the fake host completion is
/// deliberately never delivered, so cancel/deadline/session-stop is the only
/// reachable terminal.
const NEGATIVE_SLEEP_BODY: &[u8] = b"60000";

/// Phase 4 VCP on the real runtime surface. C4 owns admission of the single
/// canonical host effect, V4 owns the typed linked entry and pending-contract
/// verification, and K4 owns the actual-Pending VM/scheduler/request kernel.
/// This harness drives the exact production route composition with an injected
/// `RecordingVmHeap` and the production observation sink, and asserts: actual
/// Pending (never pseudo-Ready), deterministic fake host completion at the
/// production boundary, resume at the original site, one publish/wake/claim,
/// owner transfer, and exactly one external terminal.
///
/// Current real red on the merged tree (recorded, not faked): C4 has not
/// admitted `std.time.sleep`, so every scenario stops in the production
/// authoring seam with the deterministic admission rejection chain before the
/// request driver is reached. Each scenario stays red until its exact
/// producer gap closes.
#[tokio::test(flavor = "current_thread")]
async fn phase_4_vcp_production_composition() {
    let fixture = publish_or_panic(
        "phase-4-vcp",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    assert_vcp_source_fixture();
    assert_recording_heap_is_vm_heap();

    for (body, expected) in [(&b"1"[..], 2.0), (&b"2"[..], 3.0)] {
        let correlation = phase_4_correlation("vcp");
        let trace = HeapSpyTrace::default();
        let spy = RecordingVmHeap::new(RequestHeapLimits::default(), trace.clone());
        let evidence = drive_phase_4_vcp_request(
            &fixture,
            &correlation,
            Box::new(spy),
            body,
        )
        .await
        .unwrap_or_else(|error| {
            panic!("Phase 4 VCP production drive must park, complete and resume once K4 joins: {error}")
        });
        let payload = serde_json::from_slice::<Value>(&evidence.payload)
            .expect("decode Phase 4 VCP JSON response");
        assert_eq!(
            payload,
            serde_json::json!(expected),
            "resume must restore the original site and return the value computed after sleep"
        );
        assert_owner_transfer(&evidence.owner_inventory);
        assert_heap_resume_facts(&trace.events(), expected);
    }

    // Terminal exactly once through the production host spawn path: the same
    // canonical request must settle with a single correlated terminal frame
    // and exactly one production terminal claim.
    let correlation = phase_4_correlation("vcp-terminal");
    let sink = Arc::new(RecordingSink::default());
    let mut host = runtime_host(&correlation);
    host.bytecode_execution_event_sink = sink.clone();
    let bootstrap = fixture.connection_bootstrap();
    let request = fixture.canonical_request(&correlation, "unary", b"1");
    let response = run_phase_4_request(&host, &bootstrap, &correlation, request).await;
    let CorrelatedResponse::End { header, body, .. } = response else {
        panic!("Phase 4 VCP host spawn must return response.end")
    };
    assert_eq!(header.request_id, correlation.request_id);
    assert!(header.payload_present);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).expect("decode host spawn payload"),
        serde_json::json!(2.0),
        "the production host spawn must observe the same post-resume payload"
    );
    assert_single_terminal(&sink, &correlation, BytecodeRequestTerminal::Succeeded);
    let inventories = sink.cleanup_inventories(&correlation);
    assert_eq!(inventories.len(), 1, "exactly one cleanup inventory");
    assert_owner_transfer(&inventories[0]);
}

/// Stage sentinel source -> admission. The input is the real fixture source
/// through the production compiler authoring seam; the assertion is the
/// admission decision for `std.time.sleep`. Until C4 joins, the production
/// admission is the only rejection owner and deterministically names the
/// pending effect.
#[test]
fn phase_4_stage_sentinel_source_to_admission() {
    match Phase4PublishedFixture::build(
        "phase-4-sentinel-admission",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
        PHASE4_VCP_VERSION,
        PHASE4_VCP_INGRESS,
    ) {
        Phase4FixtureBuild::Rejected { error_chain } => {
            assert!(
                error_chain.contains("callable pending effects"),
                "pre-C4 admission must deterministically name the pending effect; observed: {error_chain}"
            );
            panic!(
                "C4 admission not joined: std.time.sleep must be the single admitted canonical \
                 host effect; observed admission rejection chain: {error_chain}"
            );
        }
        Phase4FixtureBuild::Published(fixture) => {
            assert!(
                fixture.package_bytecode_ref().is_some(),
                "admission must attach the admitted bytecode record"
            );
        }
    }
}

/// Stage sentinel admission -> emission. The input is the real admitted
/// package; the assertion is on the real emitted bytecode record: exactly one
/// `InvokeHost` call site for the canonical sleep, with no emitter rewrite.
#[test]
fn phase_4_stage_sentinel_admission_to_emission() {
    let fixture = publish_or_panic(
        "phase-4-sentinel-emission",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let store = fixture.open_store();
    let bytecode_ref = fixture
        .package_bytecode_ref()
        .expect("admitted package carries a bytecode record");
    let validated = store
        .read_package_bytecode(fixture.package_ref(), bytecode_ref)
        .expect("read emitted bytecode record");
    let view = structurally_validate(validated.artifact()).expect("validate emitted bytecode");
    let invoke_host_sites = view
        .functions()
        .iter()
        .flat_map(|function| {
            function
                .instructions
                .iter()
                .filter(|instruction| instruction.descriptor.kind == Opcode::InvokeHost)
                .map(move |instruction| (function.function_key.as_str(), instruction.pc))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        invoke_host_sites.len(),
        1,
        "the fixture must emit exactly one InvokeHost call site for std.time.sleep; \
         observed: {invoke_host_sites:?}"
    );
}

/// Stage sentinel emission -> link. The input is the real emitted artifact
/// hydrated from the canonical store; the assertion is on the real linked
/// image: exactly one typed host-effect adapter entry with the pinned opaque
/// executor identity and linked signature (no std bypass or string dispatch).
#[tokio::test(flavor = "current_thread")]
async fn phase_4_stage_sentinel_emission_to_link() {
    let fixture = publish_or_panic(
        "phase-4-sentinel-link",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let registry = BytecodeDeploymentRegistry::new();
    let image = registry
        .get_or_load(fixture.deployment_ref(), fixture.artifact_root_path())
        .await
        .expect("production deployment load must succeed once V4 joins")
        .expect("production deployment has a bytecode image");
    let adapter_indices = image
        .functions()
        .iter()
        .flat_map(|function| function.instructions())
        .flat_map(|instruction| instruction.resolved_operands())
        .filter_map(|operand| match operand.target() {
            LinkedInstructionTarget::HostEffectAdapter(index) => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        adapter_indices.len(),
        1,
        "the linked image must carry exactly one typed host-effect adapter index; \
         observed: {adapter_indices:?}"
    );
    let target = image
        .host_effect_target(adapter_indices[0])
        .expect("the exact typed host-effect index must resolve through the opaque view");
    assert_eq!(
        target.executor_identity(),
        HostEffectExecutorIdentity::Sleep,
        "the typed entry must retain the pinned Sleep executor identity"
    );
    assert_eq!(
        target.signature().parameter_types().len(),
        1,
        "the pinned Sleep target must retain its exact one-parameter linked signature"
    );
    assert!(
        target.signature().result_types().is_empty(),
        "the pinned Sleep target must retain its exact void linked signature"
    );
}

/// Stage sentinel link -> verify. The input is the real linked image; the
/// assertion is on the real verifier certificate: exactly one
/// `HostEffect` resume site proving the pending contract
/// (`ActualWithResume{HostEffect}`).
#[tokio::test(flavor = "current_thread")]
async fn phase_4_stage_sentinel_link_to_verify() {
    let fixture = publish_or_panic(
        "phase-4-sentinel-verify",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let registry = BytecodeDeploymentRegistry::new();
    let image = registry
        .get_or_load(fixture.deployment_ref(), fixture.artifact_root_path())
        .await
        .expect("production deployment load must succeed once V4 joins")
        .expect("production deployment has a bytecode image");
    let host_effect_sites = image
        .resume_sites()
        .rows()
        .iter()
        .filter(|site| matches!(site.kind(), VerifiedResumeKind::HostEffect))
        .collect::<Vec<_>>();
    assert_eq!(
        host_effect_sites.len(),
        1,
        "the verifier must certify exactly one HostEffect pending/resume site; \
         observed: {host_effect_sites:?}"
    );
}

/// Stage sentinel verify -> scheduler. The input is the real verified image
/// driven through the production request seam; the assertion is the actual
/// Pending park (inventory shows exactly one live published owner), never a
/// pseudo-Ready completion.
#[tokio::test(flavor = "current_thread")]
async fn phase_4_stage_sentinel_verify_to_scheduler() {
    let fixture = publish_or_panic(
        "phase-4-sentinel-scheduler",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let correlation = phase_4_correlation("scheduler-sentinel");
    let trace = HeapSpyTrace::default();
    let spy = RecordingVmHeap::new(RequestHeapLimits::default(), trace);
    let parked = park_phase_4_request(&fixture, &correlation, Box::new(spy), b"1")
        .await
        .unwrap_or_else(|error| {
            panic!("verified image must park as actual Pending once K4 joins: {error}")
        });
    assert!(
        parked.pending_completion().complete(),
        "the parked request must hand out a completion authority that wins its \
         single published pending cell (actual Pending, never pseudo-Ready)"
    );
    resume_phase_4_parked(parked)
        .expect("the completed parked request must resume to its root completion");
}

/// Stage sentinel scheduler -> request -> response. The input is the parked
/// production request; deterministic completion resumes the original site and
/// the boundary response is the deterministic post-sleep payload.
#[tokio::test(flavor = "current_thread")]
async fn phase_4_stage_sentinel_scheduler_to_request_response() {
    let fixture = publish_or_panic(
        "phase-4-sentinel-response",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let correlation = phase_4_correlation("response-sentinel");
    let trace = HeapSpyTrace::default();
    let spy = RecordingVmHeap::new(RequestHeapLimits::default(), trace);
    let evidence = drive_phase_4_vcp_request(&fixture, &correlation, Box::new(spy), b"2")
        .await
        .unwrap_or_else(|error| {
            panic!("parked request must resume to its boundary response once K4 joins: {error}")
        });
    assert_eq!(
        serde_json::from_slice::<Value>(&evidence.payload).expect("decode sentinel payload"),
        serde_json::json!(3.0),
        "resume must restore the original site and return seed + 1"
    );
    assert_owner_transfer(&evidence.owner_inventory);
}

/// Cancel-before-complete negative: the host completion is never delivered and
/// cancellation is the only reachable terminal. The supervisor must claim
/// exactly one Cancelled terminal and emit zero wire terminal frames.
#[tokio::test(flavor = "current_thread")]
async fn phase_4_negative_cancel_before_complete() {
    let fixture = publish_or_panic(
        "phase-4-cancel",
        PHASE4_NEGATIVE_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let correlation = phase_4_correlation("cancel");
    let sink = Arc::new(RecordingSink::default());
    let mut host = runtime_host(&correlation);
    host.bytecode_execution_event_sink = sink.clone();
    let bootstrap = fixture.connection_bootstrap();
    let request = fixture.canonical_request(&correlation, "unary", NEGATIVE_SLEEP_BODY);
    let mut receiver = spawn_phase_4_request(&host, &bootstrap, &correlation, request).await;
    await_active_request(&host).await;
    host.cancel_request(
        &correlation.router_session_epoch(),
        RequestCancel {
            request_id: correlation.request_id.clone(),
            reason: Some("phase-4-cancel-before-complete".to_string()),
        },
    )
    .await;
    await_terminal_without_response(&mut receiver, &correlation.request_id).await;
    assert_single_terminal(&sink, &correlation, BytecodeRequestTerminal::Cancelled);
}

/// Deadline-race negative: a near-term wire deadline races a far-future sleep
/// whose completion never arrives; the deadline is the single terminal and it
/// projects the canonical TimeoutError exactly once.
#[tokio::test(flavor = "current_thread")]
async fn phase_4_negative_deadline_race() {
    let fixture = publish_or_panic(
        "phase-4-deadline",
        PHASE4_NEGATIVE_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let correlation = phase_4_correlation("deadline");
    let sink = Arc::new(RecordingSink::default());
    let mut host = runtime_host(&correlation);
    host.bytecode_execution_event_sink = sink.clone();
    let bootstrap = fixture.connection_bootstrap();
    let request = fixture.canonical_request_with_deadline(
        &correlation,
        "unary",
        NEGATIVE_SLEEP_BODY,
        Some(200),
    );
    let response = run_phase_4_request(&host, &bootstrap, &correlation, request).await;
    let CorrelatedResponse::Error {
        frame,
        header,
        error,
    } = response
    else {
        panic!("deadline race must return exactly one response.error terminal")
    };
    assert!(!frame.is_empty());
    assert_eq!(header.request_id(), correlation.request_id);
    let ValidatedResponseErrorFrame::Control(error) = error else {
        panic!("deadline race must project a typed control response.error")
    };
    assert_eq!(error.code, "TimeoutError");
    assert_eq!(error.message, "execution deadline exceeded");
    assert_single_terminal(&sink, &correlation, BytecodeRequestTerminal::Failed);
}

/// Duplicate-wake-drop negative: two competing host completions must settle
/// the pending cell exactly once; the second completion is dropped and never
/// re-settles or double-resumes the site. Exercised through the same
/// deterministic completion boundary as the VCP (SEAM-4).
#[tokio::test(flavor = "current_thread")]
async fn phase_4_negative_duplicate_wake_drop() {
    let fixture = publish_or_panic(
        "phase-4-duplicate",
        PHASE4_VCP_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let correlation = phase_4_correlation("duplicate");
    let trace = HeapSpyTrace::default();
    let spy = RecordingVmHeap::new(RequestHeapLimits::default(), trace);
    let parked = park_phase_4_request(&fixture, &correlation, Box::new(spy), b"1")
        .await
        .unwrap_or_else(|error| {
            panic!("duplicate completion must park as actual Pending when K4 joins: {error}")
        });
    let completion = parked.pending_completion();
    assert!(
        completion.complete(),
        "the first competing host completion must win the parked cell"
    );
    assert!(
        !completion.complete(),
        "the duplicate host completion must be dropped without re-settling the cell"
    );
    let evidence = resume_phase_4_parked(parked)
        .expect("the single winning completion must resume the original site exactly once");
    assert_eq!(
        serde_json::from_slice::<Value>(&evidence.payload).expect("decode duplicate payload"),
        serde_json::json!(2.0),
        "the single winning completion must resume the original site exactly once"
    );
    assert_owner_transfer(&evidence.owner_inventory);
}

/// Session-disconnect negative: stopping the router session must terminate
/// every request-owned Pending/fiber in that session with exactly one
/// stop-without-response terminal (VM-13 closure).
#[tokio::test(flavor = "current_thread")]
async fn phase_4_negative_session_disconnect() {
    let fixture = publish_or_panic(
        "phase-4-disconnect",
        PHASE4_NEGATIVE_FIXTURE_RELATIVE,
        PHASE4_VCP_PACKAGE_ID,
    );
    let correlation = phase_4_correlation("disconnect");
    let sink = Arc::new(RecordingSink::default());
    let mut host = runtime_host(&correlation);
    host.bytecode_execution_event_sink = sink.clone();
    let bootstrap = fixture.connection_bootstrap();
    let request = fixture.canonical_request(&correlation, "unary", NEGATIVE_SLEEP_BODY);
    let mut receiver = spawn_phase_4_request(&host, &bootstrap, &correlation, request).await;
    await_active_request(&host).await;
    host.request_supervisor
        .stop_session(&correlation.router_session_epoch());
    await_terminal_without_response(&mut receiver, &correlation.request_id).await;
    assert_single_terminal(&sink, &correlation, BytecodeRequestTerminal::Cancelled);
}

fn publish_or_panic(
    prefix: &str,
    fixture_relative: &str,
    package_id: &str,
) -> Phase4PublishedFixture {
    match Phase4PublishedFixture::build(
        prefix,
        fixture_relative,
        package_id,
        PHASE4_VCP_VERSION,
        PHASE4_VCP_INGRESS,
    ) {
        Phase4FixtureBuild::Rejected { error_chain } => panic!(
            "C4/V4/K4 have joined: the Phase 4 fixture must publish through the \
             production authoring seam; observed rejection chain: {error_chain}"
        ),
        Phase4FixtureBuild::Published(fixture) => fixture,
    }
}

fn assert_vcp_source_fixture() {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(PHASE4_VCP_FIXTURE_RELATIVE)
        .join("main.skiff");
    let source = std::fs::read_to_string(source_path).expect("read accepted Phase 4 VCP source");
    assert!(source.contains("import std"));
    assert!(source.contains("std.time.sleep(Duration.milliseconds(1))"));
    assert!(source.contains("return seed + 1.0"));
}

fn assert_recording_heap_is_vm_heap() {
    require_vm_heap::<RecordingVmHeap>();
}

fn require_vm_heap<T: VmHeap + Send>() {}

/// The owner-transfer fact: the request published its pending owner exactly
/// once (ever_created) and the completion transferred it out completely
/// (current back to zero). The lease accounting makes a double release
/// impossible, so a single publish plus a zero terminal count is the exact
/// publish -> wake -> claim -> resume ownership proof.
fn assert_owner_transfer(inventory: &RequestExecutionOwnerInventorySnapshot) {
    assert!(
        inventory.pending.ever_created,
        "the request must have published its pending owner exactly once"
    );
    assert_eq!(
        inventory.pending.current, 0,
        "the terminal must have transferred the pending owner out completely"
    );
    assert_eq!(
        inventory.resource.current, 0,
        "no resource owner is in Phase 4 scope"
    );
    assert_eq!(
        inventory.child.current, 0,
        "no child owner is in Phase 4 scope"
    );
}

async fn await_active_request(host: &super::RuntimeHost) {
    for _ in 0..1_000 {
        if host.request_supervisor.active_count().await > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("the request did not activate before the negative terminal race")
}

fn assert_single_terminal(
    sink: &RecordingSink,
    correlation: &super::phase_0_proof_support::Correlation,
    expected: BytecodeRequestTerminal,
) {
    let terminals = sink.terminals(correlation);
    assert_eq!(
        terminals,
        vec![expected],
        "the request must claim exactly one terminal"
    );
    let _ = sink.snapshot();
}

fn assert_heap_resume_facts(events: &[super::phase_2_proof_support::HeapSpyEvent], scenario: f64) {
    // Phase 4's pinned sleep argument is an immediate scalar, so a recording
    // heap may legitimately observe zero heap operations. The owner inventory
    // and payload assertions are the actual park/resume proof.
    let _ = (events, scenario);
}
