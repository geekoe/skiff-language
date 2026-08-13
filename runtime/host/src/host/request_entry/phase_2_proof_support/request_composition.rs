use std::sync::Arc;

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
    vm_heap::VmHeap,
};
use skiff_runtime_request::{
    drive_runtime_bytecode_request, BinaryHttpRequest, BinaryHttpRequestMetadata, BoundaryResponse,
    BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput, ExecutionBudget, HttpNameValue,
    RequestEnvelope, ResponseEnd, ResponseEvent,
};

use crate::loader::bytecode_admission::{BytecodeDeploymentRegistry, BytecodeRouteSelector};

use super::{Correlation, Phase2PublishedFixture};

/// Drives the Phase 2 VCP through the production composition with an injected
/// recording heap: the host's own production deployment registry loads, links
/// and verifies the canonical fixture under production limits, publishes the
/// production admission observations, and the exact production
/// `drive_runtime_bytecode_request` consumes the injected heap. No second
/// executor, hand-built image, or VM is invented.
pub(in crate::host::request_entry) async fn drive_phase_2_vcp_request(
    fixture: &Phase2PublishedFixture,
    correlation: &Correlation,
    heap: Box<dyn VmHeap + Send>,
) -> Result<Vec<u8>, String> {
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
        .map_err(|error| format!("production deployment load failed: {error}"))?
        .ok_or_else(|| "production deployment has no bytecode route".to_string())?;
    route.publish_admission_observations();
    let target = route
        .execution_entry()
        .map_err(|error| format!("bytecode gateway lookup failed: {error}"))?;
    let adapter = route
        .http_adapter()
        .map_err(|error| format!("gateway HTTP adapter failed: {error}"))?;

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
                url: format!("http://phase-2.example.test{}", fixture.ingress_path()),
                path: fixture.ingress_path().to_string(),
                query: Vec::<HttpNameValue>::new(),
                headers: Vec::<HttpNameValue>::new(),
            },
            body: fixture.request_body().to_vec(),
        }),
        http_adapter: Some(adapter),
        test_effects_enabled: false,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: Default::default(),
    };

    let driven = drive_runtime_bytecode_request(BytecodeRequestExecutionInput {
        target,
        request: envelope,
        observer,
        cancellation: CancellationToken::new(),
        execution_budget: Arc::new(ExecutionBudget::for_runtime_request(None)),
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
        },
        heap: Some(heap),
    });
    let owner_inventory = driven.owner_inventory.into_snapshot();
    for (domain_name, domain) in [
        ("pending", owner_inventory.pending),
        ("resource", owner_inventory.resource),
        ("child", owner_inventory.child),
    ] {
        assert_eq!(
            domain.current, 0,
            "synchronous Phase 2 request created a live {domain_name} owner"
        );
        assert!(
            !domain.ever_created,
            "synchronous Phase 2 request ever created a {domain_name} owner"
        );
    }
    // The retention carrier holds the spy heap; dropping it completes the
    // boundary-carrier release sequence before the harness reads the trace.
    drop(driven.retention);
    let result = driven.result;
    match result {
        Ok(BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))) => {
            Ok(payload)
        }
        Ok(other) => Err(format!(
            "VCP returned a non-payload boundary response: {other:?}"
        )),
        Err(error) => Err(format!("production bytecode drive failed: {error:?}")),
    }
}
