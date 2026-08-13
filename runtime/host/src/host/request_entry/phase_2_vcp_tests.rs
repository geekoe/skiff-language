use serde_json::Value;
use skiff_runtime_model::vm_heap::VmHeap;

use super::{
    phase_0_proof_support::{runtime_host, CorrelatedResponse},
    phase_2_proof_support::{
        heap_spy_seam_requirement, phase_2_correlation, run_phase_2_request, Phase2FixtureBuild,
        Phase2PublishedFixture, RecordingVmHeap, PHASE2_NEGATIVE_FIXTURE_RELATIVE,
        PHASE2_VCP_FIXTURE_RELATIVE,
    },
};

const PHASE2_VCP_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-2";
const PHASE2_VCP_VERSION: &str = "1.0.0";
const PHASE2_NEGATIVE_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-2-negative";
const PHASE2_NEGATIVE_VERSION: &str = "1.0.0";

/// Phase 2 VCP harness. Red until C2 (exact-plan emission) and K2 (lifecycle
/// kernel + heap-injection seam) join. The red is proven by real assertions:
/// the production authoring attempt is actually executed, the spy is proven to
/// be a real `VmHeap`, and the external alias-isolation response is asserted
/// against the exact expected JSON whenever publication succeeds.
#[tokio::test(flavor = "current_thread")]
async fn phase_2_vcp_production_composition() {
    let mut gaps = Vec::new();

    // The spy must be a genuine production VmHeap implementation: this line
    // fails to compile if it is not. It is the harness's only stand-in for the
    // injection seam until K2 lands the Option<Box<dyn VmHeap + Send>> input.
    assert_recording_heap_is_vm_heap();

    match Phase2PublishedFixture::build(
        "phase-2-vcp",
        PHASE2_VCP_FIXTURE_RELATIVE,
        PHASE2_VCP_PACKAGE_ID,
        PHASE2_VCP_VERSION,
        "/phase-2/vcp",
        b"1",
    ) {
        Phase2FixtureBuild::Rejected { error_chain } => {
            gaps.push(format!(
                "C2 exact-plan emission has not joined: the nested record/array VCP fixture must \
                 publish through the production authoring seam; observed deterministic rejection \
                 chain: {error_chain}"
            ));
        }
        Phase2FixtureBuild::Published(fixture) => {
            assert_vcp_source_fixture();
            prove_external_alias_isolation(&fixture, &mut gaps).await;
        }
    }

    // Internal share/COW/drop facts cannot be proven until the spy can be
    // injected; the requirement is reported verbatim and keeps the harness red.
    gaps.push(heap_spy_seam_requirement().to_string());

    assert!(
        gaps.is_empty(),
        "Phase 2 VCP stays expected-red until C2 + K2 join:\n- {}",
        gaps.join("\n- ")
    );
}

/// Missing-plan negative. Red until C2 replaces the legacy Phase-1 admission
/// boundary with the exact-plan authority. The stable-rejection, deterministic
/// owner/message, and no-artifact facts are asserted for real today.
#[test]
fn phase_2_missing_plan_negative() {
    let mut gaps = Vec::new();

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

    match (first, second) {
        (Phase2FixtureBuild::Published(_), _) | (_, Phase2FixtureBuild::Published(_)) => {
            gaps.push(
                "missing-plan negative published an artifact: the source without an exact plan \
                 must be stably rejected with no publication"
                    .to_string(),
            );
        }
        (
            Phase2FixtureBuild::Rejected { error_chain },
            Phase2FixtureBuild::Rejected {
                error_chain: repeated,
            },
        ) => {
            assert_eq!(
                error_chain, repeated,
                "emission rejection owner and message must be deterministic across attempts"
            );
            assert_typed_emission_owner(&error_chain);
            if error_chain.contains("Phase 1 bytecode admission rejected") {
                gaps.push(format!(
                    "C2 missing-plan authority has not joined: the observed rejection is the \
                     legacy Phase-1 admission boundary rather than the exact-plan typed \
                     BytecodeEmissionError; observed deterministic chain: {error_chain}"
                ));
            }
        }
    }

    assert!(
        gaps.is_empty(),
        "Phase 2 missing-plan negative stays expected-red until C2 joins:\n- {}",
        gaps.join("\n- ")
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
    assert!(source.contains("final carried = stamp(a)"));
    assert!(source.contains("return Probe { original: carried, mutated: b }"));
}

async fn prove_external_alias_isolation(fixture: &Phase2PublishedFixture, gaps: &mut Vec<String>) {
    let bootstrap = fixture.connection_bootstrap();
    let correlation = phase_2_correlation("vcp");
    let host = runtime_host(&correlation);
    let request = fixture.canonical_request(&correlation, "unary");
    assert_eq!(request.body, b"1", "the real wire seed is run(1)");

    let response = run_phase_2_request(&host, &bootstrap, &correlation, request).await;
    let CorrelatedResponse::End { header, body, .. } = response else {
        gaps.push("Phase 2 VCP must return response.end once producers join".to_string());
        return;
    };
    assert_eq!(header.request_id, correlation.request_id);
    assert!(header.payload_present);

    let payload = serde_json::from_slice::<Value>(&body).expect("decode Phase 2 VCP JSON response");
    let expected = serde_json::json!({
        "original": {
            "inner": { "x": 1.0, "tags": [1.0, 2.0] },
            "rows": [],
        },
        "mutated": {
            "inner": { "x": 2.0, "tags": [1.0, 2.0] },
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
        "b.inner.x must be the committed mutation"
    );
}

fn assert_typed_emission_owner(error_chain: &str) {
    assert!(
        error_chain.contains("bytecode")
            || error_chain.contains("BytecodeEmissionError"),
        "the negative rejection owner must be the typed bytecode emission error; observed: {error_chain}"
    );
}
