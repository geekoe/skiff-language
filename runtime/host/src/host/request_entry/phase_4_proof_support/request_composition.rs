use std::sync::Arc;

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
    vm_heap::VmHeap,
};
use skiff_runtime_request::{
    drive_runtime_bytecode_request, BinaryHttpRequest, BinaryHttpRequestMetadata,
    BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput, ExecutionBudget, HttpNameValue,
    RequestEnvelope, RequestError, RequestExecutionOwnerInventorySnapshot,
};

use crate::loader::bytecode_admission::{BytecodeDeploymentRegistry, BytecodeRouteSelector};

use super::{Correlation, Phase4PublishedFixture};

/// SEAM-4 (pinned by P4G for K4): the production request driver must expose
/// the parked request as an actual-Pending carrier at the production boundary
/// so a deterministic fake host completion can settle it and resume the
/// original VM site. The pre-Phase-4 driver maps `Parked` to
/// `RequestError::Unsupported`; that pseudo-absence is exactly what K4
/// replaces, and it is exactly what this helper reports while K4 has not
/// joined. A synchronous completion (`Ready`) is a pseudo-Ready and is
/// rejected: the phase contract forbids waiting synchronously and reporting
/// Ready. The observable contract this harness pins for K4:
///
/// - park freezes the inventory with `pending.current == 1 && ever_created`
///   (the single publish);
/// - completion claims one wake exactly once; a second completion is dropped;
/// - resume restores the original site and the terminal inventory returns to
///   `pending.current == 0` with `ever_created == true` (the complete owner
///   transfer: the lease is released exactly once, never re-released).
///
/// Wake/claim cardinality is also pinned by the `k4-scheduler-*` Gate lanes
/// (`pending`, `enqueues_once`, `duplicate`, `concurrent_terminal_race`).
pub(in crate::host::request_entry) async fn park_phase_4_request(
    fixture: &Phase4PublishedFixture,
    correlation: &Correlation,
    heap: Box<dyn VmHeap + Send>,
    request_body: &[u8],
) -> Result<RequestExecutionOwnerInventorySnapshot, String> {
    let driven = drive_phase_4_seam(fixture, correlation, heap, request_body).await;
    let owner_inventory = driven.owner_inventory.into_snapshot();
    drop(driven.retention);
    match driven.result {
        Ok(_) => Err(format!(
            "pseudo-Ready: the production seam completed std.time.sleep synchronously \
             (inventory {owner_inventory:?}); the driver must return the parked request \
             as actual Pending"
        )),
        Err(RequestError::Unsupported(message)) if message.contains("parked") => Err(format!(
            "K4 seam missing: the parked bytecode request must return actual Pending for \
             deterministic controlled completion, not Unsupported: {message}"
        )),
        Err(error) => Err(format!("production bytecode drive failed: {error:?}")),
    }
}

/// Drives the Phase 4 VCP through the production composition with an injected
/// recording heap: the host's own production deployment registry loads, links
/// and verifies the canonical fixture under production limits, and the exact
/// production `drive_runtime_bytecode_request` consumes the injected heap.
/// Once K4 joins, the parked carrier is settled by the deterministic fake host
/// completion and resumed; the returned evidence carries the resumed payload
/// and the frozen terminal owner inventory.
pub(in crate::host::request_entry) async fn drive_phase_4_vcp_request(
    fixture: &Phase4PublishedFixture,
    correlation: &Correlation,
    heap: Box<dyn VmHeap + Send>,
    request_body: &[u8],
) -> Result<Phase4DriveEvidence, String> {
    // Until K4 joins, parking is the only honest observable; the fixture
    // publish already failed at the C4 admission boundary, so the test never
    // reaches this arm today.
    let parked = park_phase_4_request(fixture, correlation, heap, request_body).await?;
    Err(format!(
        "K4 seam missing: the actual-Pending carrier must support deterministic fake host \
         completion and resume before the VCP can finish (parked inventory: {parked:?})"
    ))
}

/// Evidence the VCP requires from one completed park -> complete -> resume.
pub(in crate::host::request_entry) struct Phase4DriveEvidence {
    pub(in crate::host::request_entry) payload: Vec<u8>,
    pub(in crate::host::request_entry) owner_inventory: RequestExecutionOwnerInventorySnapshot,
}

/// Production route + envelope + driver for one canonical Phase 4 request.
/// No second executor, hand-built image, or VM is invented; the Phase 3
/// composition is reused with the request body parameterized.
async fn drive_phase_4_seam(
    fixture: &Phase4PublishedFixture,
    correlation: &Correlation,
    heap: Box<dyn VmHeap + Send>,
    request_body: &[u8],
) -> skiff_runtime_request::DrivenBytecodeRequest {
    let observer = BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
        router_session_id: correlation.router_session_id.clone(),
        request_id: correlation.request_id.clone(),
    });
    let registry = BytecodeDeploymentRegistry::new();
    let route = registry
        .route(
            fixture.deployment_ref(),
            fixture.artifact_root_path(),
            BytecodeRouteSelector::Gateway {
                ingress: fixture.ingress_selector().clone(),
                gateway_entry_identity: fixture.gateway_identity().clone(),
            },
            &observer,
        )
        .await
        .expect("production deployment load must succeed once C4/V4 join")
        .expect("production deployment has no bytecode route");
    let target = route
        .execution_entry()
        .expect("bytecode gateway lookup must succeed once C4/V4 join");
    let adapter = route
        .http_adapter()
        .expect("gateway HTTP adapter must resolve once C4/V4 join");
    route.publish_admission_observations();

    let envelope = RequestEnvelope {
        request_id: correlation.request_id.clone(),
        mode: "unary".to_string(),
        target: route.target_label(),
        operation_abi_id: None,
        selector: None,
        service_id: Some(route.deployment().service_id.clone()),
        build_id: route.build_id().to_string(),
        service_protocol_identity: route.service_protocol_identity().to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(fixture.ingress_selector().clone()),
        binary_http: Some(BinaryHttpRequest {
            metadata: BinaryHttpRequestMetadata {
                method: "POST".to_string(),
                url: format!("http://phase-4.example.test{}", fixture.ingress_path()),
                path: fixture.ingress_path().to_string(),
                query: Vec::<HttpNameValue>::new(),
                headers: Vec::<HttpNameValue>::new(),
            },
            body: request_body.to_vec(),
        }),
        http_adapter: Some(adapter),
        test_effects_enabled: false,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: Default::default(),
    };

    drive_runtime_bytecode_request(BytecodeRequestExecutionInput {
        target,
        request: envelope,
        observer,
        cancellation: CancellationToken::new(),
        execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
        },
        heap: Some(heap),
    })
}
