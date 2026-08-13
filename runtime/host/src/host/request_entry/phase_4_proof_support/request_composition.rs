use std::sync::Arc;

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
    vm_heap::VmHeap,
};
use skiff_runtime_request::{
    drive_runtime_bytecode_request_controlled, BinaryHttpRequest, BinaryHttpRequestMetadata,
    BoundaryResponse, BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput,
    ControlledBytecodeDrive, ExecutionBudget, HttpNameValue, ParkedBytecodeRequest,
    RequestEnvelope, RequestExecutionOwnerInventorySnapshot, ResponseEnd, ResponseEvent,
};

use crate::loader::bytecode_admission::{BytecodeDeploymentRegistry, BytecodeRouteSelector};

use super::{Correlation, Phase4PublishedFixture};

/// SEAM-4 (pinned by MAP4 revision 2, delivered by K4): the production
/// request driver exposes `drive_runtime_bytecode_request_controlled`, whose
/// first drive stops at the first actual-Pending park and hands out a
/// `ParkedBytecodeRequest` carrier. The deterministic fake host completion is
/// `RequestPendingCompletion::complete()` — it wins the parked cell exactly
/// once and returns `false` for a duplicate — and `resume()` drains exactly
/// one claimed wake and restores the original VM site. A synchronous
/// completion (`ControlledBytecodeDrive::Complete` without parking) is the
/// forbidden pseudo-Ready and is rejected.
///
/// Owner transfer is the frozen terminal inventory: `pending.ever_created`
/// proves the single publish and `pending.current == 0` proves the complete
/// lease transfer after resume. Wake/claim cardinality is also pinned by the
/// `k4-scheduler-*` Gate lanes (`enqueues_once`, `park`, `duplicate`,
/// `concurrent_terminal_race`).
pub(in crate::host::request_entry) async fn park_phase_4_request(
    fixture: &Phase4PublishedFixture,
    correlation: &Correlation,
    heap: Box<dyn VmHeap + Send>,
    request_body: &[u8],
) -> Result<ParkedBytecodeRequest, String> {
    let drive = drive_phase_4_seam(fixture, correlation, heap, request_body).await;
    match drive {
        ControlledBytecodeDrive::Parked(parked) => Ok(parked),
        ControlledBytecodeDrive::Complete(driven) => {
            let inventory = driven.owner_inventory.into_snapshot();
            let result = driven.result;
            drop(driven.retention);
            Err(format!(
                "pseudo-Ready: the controlled production drive completed without parking \
                 (inventory {inventory:?}, result {result:?}); the driver must return the \
                 parked request as actual Pending"
            ))
        }
    }
}

/// Resumes a parked Phase 4 request after exactly one deterministic host
/// completion and projects the boundary payload plus the frozen terminal
/// owner inventory.
pub(in crate::host::request_entry) fn resume_phase_4_parked(
    parked: ParkedBytecodeRequest,
) -> Result<Phase4DriveEvidence, String> {
    let drive = parked.resume();
    let driven = match drive {
        ControlledBytecodeDrive::Complete(driven) => driven,
        ControlledBytecodeDrive::Parked(_) => {
            return Err(
                "the sleep fixture must resume to a root completion, not a second park".to_string(),
            );
        }
    };
    let owner_inventory = driven.owner_inventory.into_snapshot();
    let payload = match driven.result {
        Ok(BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))) => payload,
        Ok(other) => {
            return Err(format!(
                "resumed VCP returned a non-payload boundary response: {other:?}"
            ));
        }
        Err(error) => {
            return Err(format!(
                "resumed production bytecode drive failed: {error:?}"
            ))
        }
    };
    Ok(Phase4DriveEvidence {
        payload,
        owner_inventory,
    })
}

/// Drives the Phase 4 VCP through the production composition with an injected
/// recording heap: the host's own production deployment registry loads, links
/// and verifies the canonical fixture under production limits, the exact
/// production `drive_runtime_bytecode_request_controlled` parks on the pinned
/// `std.time.sleep`, one deterministic fake host completion settles the cell
/// and `resume` restores the original site. The returned evidence carries the
/// resumed payload and the frozen terminal owner inventory.
pub(in crate::host::request_entry) async fn drive_phase_4_vcp_request(
    fixture: &Phase4PublishedFixture,
    correlation: &Correlation,
    heap: Box<dyn VmHeap + Send>,
    request_body: &[u8],
) -> Result<Phase4DriveEvidence, String> {
    let parked = park_phase_4_request(fixture, correlation, heap, request_body).await?;
    assert!(
        parked.pending_completion().complete(),
        "the first deterministic host completion must win the parked pending cell"
    );
    resume_phase_4_parked(parked)
}

/// Evidence the VCP requires from one completed park -> complete -> resume.
pub(in crate::host::request_entry) struct Phase4DriveEvidence {
    pub(in crate::host::request_entry) payload: Vec<u8>,
    pub(in crate::host::request_entry) owner_inventory: RequestExecutionOwnerInventorySnapshot,
}

/// Production route + envelope + controlled driver for one canonical Phase 4
/// request. No second executor, hand-built image, or VM is invented; the
/// Phase 3 composition is reused with the request body parameterized.
async fn drive_phase_4_seam(
    fixture: &Phase4PublishedFixture,
    correlation: &Correlation,
    heap: Box<dyn VmHeap + Send>,
    request_body: &[u8],
) -> ControlledBytecodeDrive {
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

    drive_runtime_bytecode_request_controlled(BytecodeRequestExecutionInput {
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
