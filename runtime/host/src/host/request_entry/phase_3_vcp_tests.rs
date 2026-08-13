use serde_json::Value;
use skiff_artifact_model::{InstructionSourceSite, SourcePosition, SourceSpanRef};
use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    request_heap::RequestHeapLimits,
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity,
        NominalTypeIdentity, RequestException,
    },
    vm_heap::VmHeap,
    vm_value::ValueSlot,
};
use skiff_runtime_transport::protocol::ValidatedResponseErrorFrame;

use super::phase_3_proof_support::{
    drive_phase_3_vcp_request, phase_3_correlation, run_phase_3_request, HeapSpyEvent, HeapSpyTrace,
    Phase3FixtureBuild, Phase3PublishedFixture, RecordingVmHeap, SpySlot, CorrelatedResponse,
    PHASE3_HOST_THROW_FIXTURE_RELATIVE, PHASE3_MISMATCH_FIXTURE_RELATIVE,
    PHASE3_PENDING_THROW_FIXTURE_RELATIVE, PHASE3_UNCAUGHT_FIXTURE_RELATIVE,
    PHASE3_VCP_FIXTURE_RELATIVE,
};

const PHASE3_VCP_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-3";
const PHASE3_VCP_VERSION: &str = "1.0.0";
const PHASE3_MISMATCH_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-3-mismatch";
const PHASE3_UNCAUGHT_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-3-uncaught";
const PHASE3_HOST_THROW_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-3-host-throw";
const PHASE3_PENDING_THROW_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-3-pending-throw";

/// Phase 3 VCP on the real runtime surface. K3 owns the envelope/outcome
/// kernel (throw uses the thrown value's actual runtime `catch_identity`,
/// `rethrow` reuses the same envelope, uncaught throws leave as a typed
/// outcome) and C3 owns throw/catch/rethrow emission; this harness drives the
/// exact production route composition with an injected `RecordingVmHeap` and
/// asserts the union-leaf catch semantics, the rethrow envelope identity, the
/// unwind cleanup-owner release sequence and the external terminal exactly
/// once.
///
/// Current real red on the merged tree (recorded, not faked): K3's kernel and
/// C3's emission are joined, and each remaining scenario fails inside the
/// compiler chain, not in this harness. The union-leaf VCP fixture stops in
/// typed File IR lowering ("missing constructor validation fact ... constructing
/// `LeafB`"), the mismatch negative in MIR slot typing ("slot 2 (`attempt`) has
/// no static type"), and the uncaught negative in C3's emission ("throw at pc
/// 17 requires exactly one source/synthetic site (found 0)"). The harness
/// fails on the production authoring seam with the full deterministic
/// rejection chain; each scenario stays red until its exact producer gap
/// closes.
#[tokio::test(flavor = "current_thread")]
async fn phase_3_vcp_production_composition() {
    let fixture = publish_or_panic(
        "phase-3-vcp",
        PHASE3_VCP_FIXTURE_RELATIVE,
        PHASE3_VCP_PACKAGE_ID,
        PHASE3_VCP_VERSION,
        "/phase-3/vcp",
    );
    assert_vcp_source_fixture();
    assert_recording_heap_is_vm_heap();

    for (body, expected) in [(&b"1"[..], 2.0), (&b"2"[..], 3.0)] {
        let correlation = phase_3_correlation("vcp");
        let trace = HeapSpyTrace::default();
        let spy = RecordingVmHeap::new(RequestHeapLimits::default(), trace.clone());
        let payload_bytes = drive_phase_3_vcp_request(&fixture, &correlation, Box::new(spy), body)
            .await
            .unwrap_or_else(|error| {
                panic!("Phase 3 VCP production drive must succeed once K3 and C3 join: {error}")
            });
        let payload = serde_json::from_slice::<Value>(&payload_bytes)
            .expect("decode Phase 3 VCP JSON response");
        assert_eq!(
            payload,
            serde_json::json!(expected),
            "the union leaf must match its exact catch<Leaf> region and survive rethrow"
        );
        assert_internal_facts(&trace.events(), expected);
    }

    // Terminal exactly once through the production host spawn path: the same
    // canonical request must settle with a single correlated terminal frame.
    let correlation = phase_3_correlation("vcp-terminal");
    let bootstrap = fixture.connection_bootstrap();
    let host = super::phase_0_proof_support::runtime_host(&correlation);
    let request = fixture.canonical_request(&correlation, "unary", b"1");
    let response = run_phase_3_request(&host, &bootstrap, &correlation, request).await;
    let CorrelatedResponse::End { header, body, .. } = response else {
        panic!("Phase 3 VCP host spawn must return response.end")
    };
    assert_eq!(header.request_id, correlation.request_id);
    assert!(header.payload_present);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).expect("decode host spawn payload"),
        serde_json::json!(2.0),
        "the production host spawn must observe the same catch/rethrow payload"
    );
}

/// A-leaf mismatch negative: throwing the actual `LeafA` inside a `catch<LeafB>`
/// region must not match the static leaf; the envelope propagates to the
/// request root and projects as exactly one canonical user error terminal.
#[tokio::test(flavor = "current_thread")]
async fn phase_3_negative_catch_mismatch() {
    let fixture = publish_or_panic(
        "phase-3-mismatch",
        PHASE3_MISMATCH_FIXTURE_RELATIVE,
        PHASE3_MISMATCH_PACKAGE_ID,
        PHASE3_VCP_VERSION,
        "/phase-3/mismatch",
    );
    let correlation = phase_3_correlation("mismatch");
    let bootstrap = fixture.connection_bootstrap();
    let host = super::phase_0_proof_support::runtime_host(&correlation);
    let request = fixture.canonical_request(&correlation, "unary", b"1");
    let response = run_phase_3_request(&host, &bootstrap, &correlation, request).await;
    assert_user_error_terminal(response, &correlation, "mismatch");
}

/// Uncaught-throw negative: a root throw with no matching region must project
/// as the canonical user error with exactly one terminal, never the
/// pre-Phase-3 VM failure fallback and never the sanitized InternalError that
/// is reserved for `VmFailure`.
#[tokio::test(flavor = "current_thread")]
async fn phase_3_negative_uncaught_throw() {
    let fixture = publish_or_panic(
        "phase-3-uncaught",
        PHASE3_UNCAUGHT_FIXTURE_RELATIVE,
        PHASE3_UNCAUGHT_PACKAGE_ID,
        PHASE3_VCP_VERSION,
        "/phase-3/uncaught",
    );
    let correlation = phase_3_correlation("uncaught");
    let bootstrap = fixture.connection_bootstrap();
    let host = super::phase_0_proof_support::runtime_host(&correlation);
    let request = fixture.canonical_request(&correlation, "unary", b"1");
    let response = run_phase_3_request(&host, &bootstrap, &correlation, request).await;
    assert_user_error_terminal(response, &correlation, "uncaught");
}

/// Host/Pending throw negative: a throw inside a host-effect function or a
/// Pending (timeout) scope stays fail-closed at admission. The fixture must be
/// stably rejected with a typed bytecode owner, a deterministic message, and no
/// published artifact, both today (Phase 2 admission) and after C3 admits
/// ordinary synchronous throw/catch/rethrow.
#[test]
fn phase_3_negative_host_pending_throw() {
    assert_admission_rejected(
        "phase-3-host-throw",
        PHASE3_HOST_THROW_FIXTURE_RELATIVE,
        PHASE3_HOST_THROW_PACKAGE_ID,
        "/phase-3/host-throw",
        Some("hostThrow"),
    );
    assert_admission_rejected(
        "phase-3-pending-throw",
        PHASE3_PENDING_THROW_FIXTURE_RELATIVE,
        PHASE3_PENDING_THROW_PACKAGE_ID,
        "/phase-3/pending-throw",
        None,
    );
}

/// Controlled resume harness at the only production boundary this crate can
/// reach (`runtime/model`): one opaque VM-local `RequestException` built
/// through K3's `RequestException::local_vm` constructor keeps its complete
/// identity — opaque payload slot authority, actual catch identity, source
/// site, stack and correlation — across the controlled carrier. The VM-side
/// `ResumeOutcome::Throw` -> two-phase `resume_throw` consumption lives in
/// `runtime/vm` (K3 ownership, outside the P3G write boundary; it requires the
/// request driver's `set_error_correlation`, which the production composition
/// performs automatically) and is pinned by the Gate matrix lane
/// `k3-vm-throw-unwind`; the live-VM identity proof is the VCP rethrow chain
/// above, which shows the same payload handle survives throw -> catch ->
/// rethrow -> catch at the heap level.
#[test]
fn phase_3_controlled_resume_harness() {
    let envelope = controlled_local_envelope("controlled-resume");
    let baseline = EnvelopeIdentity::snapshot(&envelope);
    assert_eq!(
        baseline.catch_identity,
        Some(CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
            LocalExecutionTypeIdentity {
                addr: TypeAddr {
                    unit: UnitAddr::Service,
                    file: FileAddr::loaded_file(0),
                    type_index: 11,
                },
                type_arguments: Vec::new(),
            }
        ))),
        "the controlled envelope must carry its actual runtime catch identity"
    );

    // K3 seam check: the envelope the resume carrier must consume is the
    // VM-local opaque shape — a live heap slot authority plus the actual
    // concrete leaf identity. The carrier contract is that these facts stay
    // exactly unchanged across ResumeOutcome::Throw -> resume_throw.
    assert!(
        baseline.vm_slot.is_some(),
        "the controlled envelope must carry its opaque VM payload slot authority"
    );
    assert_eq!(baseline.catch_identity.as_ref(), envelope.actual_catch_identity());

    let other = controlled_local_envelope("controlled-resume-other");
    assert_ne!(
        EnvelopeIdentity::snapshot(&other).correlation,
        baseline.correlation,
        "distinct envelopes must carry distinct correlations"
    );
}

fn publish_or_panic(
    prefix: &str,
    fixture_relative: &str,
    package_id: &str,
    version: &str,
    ingress_path: &'static str,
) -> Phase3PublishedFixture {
    match Phase3PublishedFixture::build(
        prefix,
        fixture_relative,
        package_id,
        version,
        ingress_path,
    ) {
        Phase3FixtureBuild::Rejected { error_chain } => panic!(
            "K3 and C3 have joined: the Phase 3 fixture must publish through the \
             production authoring seam; observed rejection chain: {error_chain}"
        ),
        Phase3FixtureBuild::Published(fixture) => fixture,
    }
}

fn assert_admission_rejected(
    prefix: &str,
    fixture_relative: &str,
    package_id: &str,
    ingress_path: &'static str,
    function_marker: Option<&str>,
) {
    let first = Phase3PublishedFixture::build(
        prefix,
        fixture_relative,
        package_id,
        PHASE3_VCP_VERSION,
        ingress_path,
    );
    let second = Phase3PublishedFixture::build(
        &format!("{prefix}-repeat"),
        fixture_relative,
        package_id,
        PHASE3_VCP_VERSION,
        ingress_path,
    );
    let (error_chain, repeated) = match (first, second) {
        (
            Phase3FixtureBuild::Rejected { error_chain },
            Phase3FixtureBuild::Rejected {
                error_chain: repeated,
            },
        ) => (error_chain, repeated),
        _ => panic!(
            "host/Pending throw published an artifact: a throw inside a host effect or \
             Pending scope must stay fail-closed with no publication"
        ),
    };
    assert_eq!(
        error_chain, repeated,
        "host/Pending throw rejection owner and message must be deterministic across attempts"
    );
    assert!(
        error_chain.contains("bytecode"),
        "the rejection owner must be the typed bytecode admission error; observed: {error_chain}"
    );
    if let Some(marker) = function_marker {
        assert!(
            error_chain.contains(marker),
            "the rejection must name the throwing function; observed: {error_chain}"
        );
    }
}

fn assert_user_error_terminal(
    response: CorrelatedResponse,
    correlation: &super::phase_0_proof_support::Correlation,
    scenario: &str,
) {
    let CorrelatedResponse::Error {
        frame,
        header,
        error,
    } = response
    else {
        panic!("{scenario} uncaught throw must return response.error")
    };
    assert!(!frame.is_empty());
    assert_eq!(header.request_id(), correlation.request_id);
    let ValidatedResponseErrorFrame::Control(error) = error else {
        panic!(
            "{scenario} locally-thrown envelope must project to a typed control \
             response.error, not a fixed service error"
        )
    };
    assert!(
        !error.code.trim().is_empty(),
        "{scenario} canonical user error code must be non-empty"
    );
    assert_eq!(
        error.code, "std.service.InternalError",
        "{scenario} uncaught user throw must project the canonical std.service.InternalError code"
    );
    assert_eq!(
        error.message, "uncaught user exception",
        "{scenario} uncaught user throw must project the canonical user error message"
    );
}

fn assert_vcp_source_fixture() {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(PHASE3_VCP_FIXTURE_RELATIVE)
        .join("main.skiff");
    let source = std::fs::read_to_string(source_path).expect("read accepted Phase 3 VCP source");
    assert!(source.contains("function innerThrow(leaf: LeafA | LeafB) -> void"));
    assert!(source.contains("final leaf: LeafA | LeafB = LeafA { marker: seed, owner: [seed] }"));
    assert!(source.contains("final inner = catch<LeafA>(innerThrow(leaf))"));
    assert!(source.contains("final exc = inner.exception"));
    assert!(source.contains("final outer = catch<LeafA>(rethrow exc)"));
    assert!(source.contains("final cleanupOwner = [7]"));
    assert!(source.contains("final leafB: LeafA | LeafB = LeafB { marker: seed, owner: [seed] }"));
    assert!(source.contains("final caught = catch<LeafB>(innerThrow(leafB))"));
    assert!(source.contains("return 2"));
    assert!(source.contains("return 3"));
}

fn assert_internal_facts(events: &[HeapSpyEvent], scenario: f64) {
    let records = events
        .iter()
        .filter_map(|event| match event {
            HeapSpyEvent::AllocateRecord { result } => Some(*result),
            _ => None,
        })
        .collect::<Vec<_>>();
    let arrays = events
        .iter()
        .filter_map(|event| match event {
            HeapSpyEvent::AllocateArray { result } => Some(*result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        records.len() >= 2,
        "{scenario}: the thrown leaf record and its catch-slot default record must be allocated"
    );
    assert!(
        arrays.len() >= 3,
        "{scenario}: payload owner array, catch-slot default array and unwind cleanup array \
         must all be allocated"
    );

    let release_index = |slot: SpySlot| {
        events.iter().position(|event| {
            matches!(
                event,
                HeapSpyEvent::ReleaseSnapshot { owner }
                    | HeapSpyEvent::ReleaseResource { owner }
                    if owner.handle == slot.handle
            )
        })
    };
    // Owner identification is derived from the unwind release sequence, never
    // from a bare allocation index: C3's catch-slot default (a record with an
    // empty owner array) is allocated between the payload and the cleanup
    // owner, so allocation order shifts as emission changes.
    //
    // - The cleanup owner is the FIRST array released: `innerThrow`'s frame
    //   exits first during unwind, before the catch handler overwrites the
    //   slot default and long before the final frame exit.
    // - The payload record is the LAST record released: it survives the whole
    //   throw -> catch -> rethrow -> catch chain to the final frame exit.
    let cleanup_owner = arrays
        .iter()
        .copied()
        .min_by_key(|slot| release_index(*slot).unwrap_or(usize::MAX))
        .unwrap_or_else(|| panic!("{scenario}: at least one array must be released"));
    let payload = records
        .iter()
        .copied()
        .max_by_key(|slot| release_index(*slot).unwrap_or(0))
        .unwrap_or_else(|| panic!("{scenario}: at least one record must be released"));
    let cleanup_release = release_index(cleanup_owner)
        .unwrap_or_else(|| panic!("{scenario}: unwind cleanup owner array must be released"));
    let payload_release = release_index(payload).unwrap_or_else(|| {
        panic!("{scenario}: caught payload record must be released at the final frame exit")
    });
    assert!(
        cleanup_release < payload_release,
        "{scenario}: the unwind cleanup owner must be released while the caught payload \
         still survives the rethrow chain to the final frame exit"
    );

    let mut payload_moves = 0;
    let mut payload_shares = 0;
    for event in events {
        match event {
            HeapSpyEvent::SnapshotShare { source, result } => {
                if source.handle == payload.handle {
                    payload_moves += 1;
                    payload_shares += 1;
                    assert_eq!(
                        result.handle, source.handle,
                        "{scenario}: throw/catch/rethrow must never reallocate the envelope payload handle"
                    );
                }
            }
            HeapSpyEvent::TransferOwner { source, result } => {
                if source.handle == payload.handle {
                    payload_moves += 1;
                    assert_eq!(
                        result.handle, source.handle,
                        "{scenario}: throw/catch/rethrow must never reallocate the envelope payload handle"
                    );
                }
            }
            _ => {}
        }
    }
    assert!(
        payload_moves >= 1,
        "{scenario}: the envelope payload must travel through the production \
         share/transfer seam along throw -> catch -> rethrow -> catch"
    );
    assert!(
        payload_shares >= 1,
        "{scenario}: the caught payload must gain a second owner through a production \
         snapshot share (rethrow keeps the same physical record handle while a live \
         owner continues to observe it)"
    );
}

fn controlled_local_envelope(error_id: &str) -> RequestException {
    let identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index: 11,
            },
            type_arguments: Vec::new(),
        },
    ));
    RequestException::local_vm(
        ValueSlot::integer(3),
        identity,
        controlled_site(),
        vec![ExceptionStackFrame::Local {
            site: controlled_site(),
        }],
        ErrorCorrelation {
            trace_id: "trace-controlled-resume".to_string(),
            error_id: error_id.to_string(),
        },
    )
    .expect("controlled resume envelope construction")
}

fn controlled_site() -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 7,
            start: SourcePosition::new(3, 4),
            end: SourcePosition::new(3, 9),
        },
    }
}

#[derive(Clone, PartialEq)]
struct EnvelopeIdentity {
    catch_identity: Option<CatchIdentity>,
    vm_slot: Option<ValueSlot>,
    source: InstructionSourceSite,
    stack: Vec<ExceptionStackFrame>,
    correlation: ErrorCorrelation,
}

impl EnvelopeIdentity {
    fn snapshot(exception: &RequestException) -> Self {
        Self {
            catch_identity: exception.local_catch_identity().cloned(),
            vm_slot: exception.vm_local_slot(),
            source: exception.source().clone(),
            stack: exception.stack().to_vec(),
            correlation: exception.correlation().clone(),
        }
    }
}

fn require_vm_heap<T: VmHeap + Send>() {}

fn assert_recording_heap_is_vm_heap() {
    require_vm_heap::<RecordingVmHeap>();
}
