use std::{sync::Arc, time::Duration};

use skiff_artifact_model::{GatewayAdapterKind, GatewayAdapterSource};
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
};
use skiff_runtime_request::{
    drive_runtime_bytecode_request_async, BinaryHttpRequest, BinaryHttpRequestMetadata,
    BoundaryResponse, BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput,
    ExecutionBudget, GatewayAdapterArg, GatewayAdapterSource as RequestGatewayAdapterSource,
    HttpAdapter, HttpAdapterCallable, HttpAdapterKind, HttpNameValue, RequestEnvelope,
};

use super::{
    stages::published_positive,
    tcp_server::{Phase5TcpServer, RequestObservation},
};

const VCP_PATH: &str = "/phase-5/vcp";
const IO_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(3);

/// G5/S5 proves the scheduler boundary with a real published gateway and a
/// real socket. Nothing completes a pending cell from the proof: the
/// production HTTP executor owns wake/claim/resume. Keeping all three gates
/// closed lets the assertions distinguish actual Pending from pseudo-Ready,
/// and seeing both stream sockets before either body gate opens proves that
/// the two affine handles coexist.
pub async fn verify_to_scheduler() {
    let fixture = published_positive("s5-scheduler");
    let server = Phase5TcpServer::start();
    let cancellation = CancellationToken::new();
    let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(None));
    let input = production_request_input(
        &fixture,
        &server,
        cancellation,
        Arc::clone(&execution_budget),
        "phase-5-s5",
    );

    let drive = tokio::spawn(drive_runtime_bytecode_request_async(input));
    assert!(
        server
            .wait_for_path_async("/request", IO_OBSERVATION_TIMEOUT)
            .await,
        "the pinned request target never reached the deterministic upstream"
    );
    assert!(
        !drive.is_finished(),
        "the closed unary response gate must produce actual Pending, not pseudo-Ready"
    );
    server.release("/request");

    for path in ["/stream/left", "/stream/right"] {
        assert!(
            server
                .wait_for_response_head_async(path, IO_OBSERVATION_TIMEOUT)
                .await,
            "the exact stream target {path} never reached response-head Ready"
        );
    }
    assert!(
        !drive.is_finished(),
        "two open stream handles must remain pending while both body gates are closed"
    );
    assert_exact_outbound_routes(&server.snapshot());

    server.release("/stream/left");
    server.release("/stream/right");
    let driven = tokio::time::timeout(IO_OBSERVATION_TIMEOUT, drive)
        .await
        .expect("production drive did not resume after both real socket gates opened")
        .expect("join production request drive");
    let inventory = driven.owner_inventory.into_snapshot();
    assert!(
        matches!(&driven.result, Ok(BoundaryResponse::StreamSent)),
        "serverStream must finish through the production response-stream boundary: {:?}",
        driven.result
    );
    drop(driven.retention);

    assert_eq!(inventory.pending.current, 0, "pending owners leaked");
    assert_eq!(inventory.resource.current, 0, "resource owners leaked");
    assert_eq!(inventory.child.current, 0, "child owners leaked");
    assert!(
        inventory.pending.ever_created,
        "no actual pending owner existed"
    );
    assert!(
        inventory.resource.ever_created,
        "the two HTTP stream handles never entered the resource table"
    );
    assert_eq!(
        execution_budget
            .settlement()
            .expect("completed request has one budget winner")
            .winner(),
        skiff_runtime_request::execution_budget::ExecutionWinner::Succeeded
    );
}

fn production_request_input(
    fixture: &super::fixture::PublishedFixture,
    server: &Phase5TcpServer,
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    request_id: &str,
) -> BytecodeRequestExecutionInput {
    let gateway = fixture.gateway(VCP_PATH);
    let image = fixture.link();
    let target = image
        .http_gateway_entry(&gateway.ingress, &gateway.identity)
        .expect("production image resolves the exact VCP gateway");
    let deployment = fixture.deployment_artifact();
    let binding = deployment
        .ingress
        .iter()
        .find(|binding| binding.selector == gateway.ingress)
        .expect("VCP ingress binding remains in the published deployment");
    let gateway_entry = deployment
        .gateway_entries
        .get(&binding.gateway_entry_key)
        .expect("VCP gateway entry remains in the published deployment");
    let adapter = request_adapter_from_published_plan(
        &fixture.deployment.service_id,
        binding.gateway_entry_key.as_str(),
        &gateway_entry.adapter_plan,
    );
    let observer = BytecodeExecutionObserver::noop(BytecodeExecutionCorrelation {
        router_session_id: "phase-5-proof-session".to_string(),
        request_id: request_id.to_string(),
    });
    let request = RequestEnvelope {
        request_id: request_id.to_string(),
        mode: "serverStream".to_string(),
        target: gateway.identity.as_str().to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some(fixture.deployment.service_id.clone()),
        build_id: image.owner().build_id().as_str().to_string(),
        service_protocol_identity: image.service_protocol_identity().as_str().to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(gateway.ingress),
        binary_http: Some(BinaryHttpRequest {
            metadata: BinaryHttpRequestMetadata {
                method: "POST".to_string(),
                url: format!("http://phase-5.invalid{VCP_PATH}"),
                path: VCP_PATH.to_string(),
                query: Vec::<HttpNameValue>::new(),
                headers: Vec::<HttpNameValue>::new(),
            },
            body: server.base_url().into_bytes(),
        }),
        http_adapter: Some(adapter),
        test_effects_enabled: false,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: Default::default(),
    };

    BytecodeRequestExecutionInput {
        target,
        request,
        observer,
        cancellation,
        execution_budget,
        handles: BytecodeRequestExecutionHandles {
            request_heap_limits: RequestHeapLimits::default(),
        },
        heap: None,
    }
}

fn request_adapter_from_published_plan(
    service_id: &str,
    gateway_key: &str,
    plan: &skiff_artifact_model::GatewayAdapterPlan,
) -> HttpAdapter {
    let kind = match plan.kind {
        GatewayAdapterKind::RawHttp => HttpAdapterKind::RawHttp,
        GatewayAdapterKind::TypedJson => HttpAdapterKind::TypedJson,
        other => panic!("Phase 5 VCP published a non-HTTP adapter kind: {other:?}"),
    };
    let adapter_args = plan
        .args
        .iter()
        .map(|arg| GatewayAdapterArg {
            param: arg.param.clone(),
            source: match arg.source {
                GatewayAdapterSource::HttpRequest => RequestGatewayAdapterSource::HttpRequest,
                GatewayAdapterSource::HttpBody => RequestGatewayAdapterSource::HttpBody,
                GatewayAdapterSource::HttpContext => RequestGatewayAdapterSource::HttpContext,
                other => panic!("Phase 5 VCP published a non-HTTP adapter source: {other:?}"),
            },
        })
        .collect();
    HttpAdapter {
        kind,
        handler: HttpAdapterCallable::PackageFunction {
            package_id: service_id.to_string(),
            symbol_path: gateway_key.to_string(),
        },
        guard: None,
        pre: None,
        adapter_args,
    }
}

fn assert_exact_outbound_routes(observations: &[RequestObservation]) {
    let routes = observations
        .iter()
        .map(|entry| (entry.method.as_str(), entry.path.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        routes,
        [
            ("GET", "/request"),
            ("GET", "/stream/left"),
            ("GET", "/stream/right"),
        ],
        "the production executor must issue one unary request and the exact A/B stream pair"
    );
}
