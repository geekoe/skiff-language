use serde_json::Value;
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    vm_heap::{VmHeap, VmHeapPathSegment},
};

use super::phase_2_proof_support::{
    drive_phase_2_vcp_request, host_passthrough_note, phase_2_correlation, HeapSpyEvent,
    HeapSpyTrace, Phase2FixtureBuild, Phase2PublishedFixture, RecordingVmHeap,
    PHASE2_NEGATIVE_FIXTURE_RELATIVE, PHASE2_VCP_FIXTURE_RELATIVE,
};

const PHASE2_VCP_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-2";
const PHASE2_VCP_VERSION: &str = "1.0.0";
const PHASE2_NEGATIVE_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-2-negative";
const PHASE2_NEGATIVE_VERSION: &str = "1.0.0";

/// Phase 2 VCP on the real runtime surface. C2 published the nested aggregate
/// fixture and K2 landed the lifecycle executor plus the heap-injection seam;
/// this harness drives the exact production route composition with an injected
/// `RecordingVmHeap` and asserts both the external alias-isolation response and
/// the internal share/prepare/commit/drop primitive sequence.
///
/// Current real red (recorded, not faked): the production link of any
/// record/array construction fails in the linker stack map with
/// "bytecode control flow and stack map linking failed ... function main::run
/// pc 4: container input is absent" (`container_element`,
/// `runtime/linker/src/bytecode/stack_map/values.rs`). Reproduced verbatim with
/// C2's own published nested writable-path fixture, so this is a general
/// emitter/linker join defect, not a fixture or harness artifact.
#[tokio::test(flavor = "current_thread")]
async fn phase_2_vcp_production_composition() {
    let fixture = match Phase2PublishedFixture::build(
        "phase-2-vcp",
        PHASE2_VCP_FIXTURE_RELATIVE,
        PHASE2_VCP_PACKAGE_ID,
        PHASE2_VCP_VERSION,
        "/phase-2/vcp",
        b"1",
    ) {
        Phase2FixtureBuild::Rejected { error_chain } => panic!(
            "C2 has joined: the nested record/array VCP fixture must publish through the \
             production authoring seam; observed rejection chain: {error_chain}"
        ),
        Phase2FixtureBuild::Published(fixture) => fixture,
    };
    assert_vcp_source_fixture();
    assert_recording_heap_is_vm_heap();

    let correlation = phase_2_correlation("vcp");
    let trace = HeapSpyTrace::default();
    let spy = RecordingVmHeap::new(RequestHeapLimits::default(), trace.clone());
    let payload_bytes = drive_phase_2_vcp_request(&fixture, &correlation, Box::new(spy))
        .await
        .unwrap_or_else(|error| {
            panic!("Phase 2 VCP production drive must succeed once K2 and C2 join: {error}")
        });

    let payload =
        serde_json::from_slice::<Value>(&payload_bytes).expect("decode Phase 2 VCP JSON response");
    let expected = serde_json::json!({
        "original": {
            "inner": { "x": 1.0, "tags": [1.0, 2.0] },
            "rows": [],
        },
        "mutated": {
            "inner": { "x": 2.0, "tags": [9.0, 2.0] },
            "rows": [],
        },
    });
    assert_eq!(
        payload, expected,
        "the alias-isolation response must contain both aggregates with the exact expected values"
    );
    assert_eq!(
        payload["original"]["inner"]["x"], 1.0,
        "a.inner.x must stay at the original value"
    );
    assert_eq!(
        payload["mutated"]["inner"]["x"], 2.0,
        "b.inner.x must be the committed field mutation"
    );
    assert_eq!(
        payload["original"]["inner"]["tags"],
        serde_json::json!([1.0, 2.0]),
        "a.inner.tags must stay at the original array"
    );
    assert_eq!(
        payload["mutated"]["inner"]["tags"],
        serde_json::json!([9.0, 2.0]),
        "b.inner.tags must carry the committed index mutation"
    );

    let events = trace.events();
    assert_internal_facts(&events);
}

/// Missing-plan negative on the joined compiler chain. The source whose shape
/// has no admitted exact plan must be stably rejected at emission with a typed
/// owner, a deterministic message, and no published artifact. The typed
/// missing-plan variant itself is the compiler emission suite's authority
/// (`phase_2_bytecode_admission_missing_plan_is_a_stable_typed_rejection`),
/// already carried by the Gate matrix command `c2-emission-exact-plan`; this
/// end-to-end proof pins the fail-closed publication contract.
#[test]
fn phase_2_missing_plan_negative() {
    let first = Phase2PublishedFixture::build(
        "phase-2-negative",
        PHASE2_NEGATIVE_FIXTURE_RELATIVE,
        PHASE2_NEGATIVE_PACKAGE_ID,
        PHASE2_NEGATIVE_VERSION,
        "/phase-2/negative",
        b"1",
    );
    let second = Phase2PublishedFixture::build(
        "phase-2-negative-repeat",
        PHASE2_NEGATIVE_FIXTURE_RELATIVE,
        PHASE2_NEGATIVE_PACKAGE_ID,
        PHASE2_NEGATIVE_VERSION,
        "/phase-2/negative",
        b"1",
    );

    let (error_chain, repeated) = match (first, second) {
        (
            Phase2FixtureBuild::Rejected { error_chain },
            Phase2FixtureBuild::Rejected {
                error_chain: repeated,
            },
        ) => (error_chain, repeated),
        _ => panic!(
            "missing-plan negative published an artifact: the source without an exact plan \
             must be stably rejected with no publication"
        ),
    };
    assert_eq!(
        error_chain, repeated,
        "emission rejection owner and message must be deterministic across attempts"
    );
    assert!(
        error_chain.contains("bytecode"),
        "the negative rejection owner must be the typed bytecode emission error; observed: {error_chain}"
    );
    assert!(
        error_chain.contains("phase 2 record/array value shape"),
        "the rejection message must pin the exact unplannable shape; observed: {error_chain}"
    );
}

fn assert_recording_heap_is_vm_heap() {
    fn require<T: VmHeap + Send>() {}
    require::<RecordingVmHeap>();
}

fn assert_vcp_source_fixture() {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(PHASE2_VCP_FIXTURE_RELATIVE)
        .join("main.skiff");
    let source = std::fs::read_to_string(source_path).expect("read accepted Phase 2 VCP source");
    assert!(source.contains("var b = a"));
    assert!(source.contains("b.inner.x = 2"));
    assert!(source.contains("b.inner.tags[0] = 9"));
    assert!(source.contains("final carried = stamp(a)"));
    assert!(source.contains("return Probe { original: carried, mutated: b }"));
}

fn assert_internal_facts(events: &[HeapSpyEvent]) {
    let shares = events
        .iter()
        .filter(|event| matches!(event, HeapSpyEvent::SnapshotShare { .. }))
        .count();
    let transfers = events
        .iter()
        .filter(|event| matches!(event, HeapSpyEvent::TransferOwner { .. }))
        .count();
    let releases = events
        .iter()
        .filter(|event| matches!(event, HeapSpyEvent::ReleaseSnapshot { .. }))
        .count();
    assert!(
        shares >= 1,
        "`var b = a` must share the aggregate snapshot at least once, observed shares {shares}"
    );
    assert!(
        shares + transfers >= 3,
        "container/argument/return must each perform one exact transfer, observed \
         {shares} shares and {transfers} transfers"
    );
    assert!(
        releases >= 1,
        "frame exit must release at least one shared aggregate owner, observed {releases}"
    );

    let prepares = events
        .iter()
        .filter_map(|event| match event {
            HeapSpyEvent::PrepareWritablePath {
                root,
                segments,
                selectors,
            } => Some((root, segments, selectors)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let commits = events
        .iter()
        .filter_map(|event| match event {
            HeapSpyEvent::CommitWritablePath {
                root_before,
                root_after,
                cow,
            } => Some((root_before, root_after, cow)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        prepares.len(),
        2,
        "the VCP must pin exactly two writable paths (dense field and array index)"
    );
    assert_eq!(commits.len(), prepares.len());

    let field_prepare = &prepares[0];
    let index_prepare = &prepares[1];
    let field_commit = &commits[0];
    let index_commit = &commits[1];
    assert_eq!(
        field_prepare.1,
        &[
            VmHeapPathSegment::DenseField {
                field: "inner".to_string()
            },
            VmHeapPathSegment::DenseField {
                field: "x".to_string()
            },
        ]
    );
    assert_eq!(field_prepare.2, &0, "dense-field paths consume no selector");
    assert_eq!(
        index_prepare.1,
        &[
            VmHeapPathSegment::DenseField {
                field: "inner".to_string()
            },
            VmHeapPathSegment::DenseField {
                field: "tags".to_string()
            },
            VmHeapPathSegment::ArrayIndex,
        ]
    );
    assert_eq!(
        index_prepare.2, &1,
        "the array index path consumes one selector"
    );

    // The field mutation happens while `b` aliases `a` (owner count > 1), so
    // commit must copy-on-write and return a new root. The index mutation
    // reaches the `tags` array, which is still shared between the original
    // chain and the replacement chain, so it must copy-on-write as well; the
    // alias isolation is proven by `a.inner.tags` staying `[1, 2]`.
    assert!(
        *field_commit.2,
        "the shared-root field mutation must copy-on-write, observed roots \
         {:?} -> {:?}",
        field_commit.0, field_commit.1
    );
    assert_ne!(
        field_commit.0.handle, field_commit.1.handle,
        "the COW commit must return a replacement root handle"
    );
    assert!(
        *index_commit.2,
        "the index mutation reaches a shared intermediate container and must COW, observed roots \
         {:?} -> {:?}",
        index_commit.0, index_commit.1
    );
    assert_eq!(
        field_prepare.0.handle, field_commit.0.handle,
        "the COW commit must pair with its pinned prepare root"
    );
    assert_eq!(
        index_prepare.0.handle, index_commit.0.handle,
        "the COW commit must pair with its pinned prepare root"
    );
    assert_ne!(
        field_prepare.0.handle, index_commit.1.handle,
        "after COW the index mutation must operate on the replacement root, not the shared one"
    );

    // The spy is injected through the production driver input; the remaining
    // host spawn passthrough is a recorded integration note, not a red
    // obligation for this harness.
    assert!(!host_passthrough_note().is_empty());
}
