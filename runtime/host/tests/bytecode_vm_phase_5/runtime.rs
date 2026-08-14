use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use skiff_artifact_model::{GatewayAdapterKind, GatewayAdapterSource};
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_model::{
    bytecode_execution_observation::{BytecodeExecutionCorrelation, BytecodeExecutionObserver},
    request_heap::RequestHeapLimits,
};
use skiff_runtime_request::execution_budget::{AdmittedRequestDeadline, ExecutionWinner};
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
        VCP_PATH,
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

/// G8 cancel/deadline races use the same socket Pending boundary as S5. The
/// proof never settles a pending cell directly: cancellation enters through
/// the request token, deadline through the admitted monotonic budget, and the
/// production scheduler/provider must converge each race to one terminal and
/// an empty owner inventory.
pub async fn lifecycle_race_matrix() {
    cancellation_while_http_pending().await;
    deadline_while_http_pending().await;
    early_break_releases_body_before_late_chunk().await;
}

pub async fn single_worker_canary() {
    let fixture = published_positive("single-worker-canary");
    let server = Phase5TcpServer::start();
    let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(None));
    let input = production_request_input(
        &fixture,
        &server,
        VCP_PATH,
        CancellationToken::new(),
        Arc::clone(&execution_budget),
        "phase-5-canary",
    );
    let keep_ticking = Arc::new(AtomicBool::new(true));
    let ticks = Arc::new(AtomicU64::new(0));
    let canary = {
        let keep_ticking = Arc::clone(&keep_ticking);
        let ticks = Arc::clone(&ticks);
        tokio::spawn(async move {
            while keep_ticking.load(Ordering::Acquire) {
                ticks.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        })
    };
    let drive = tokio::spawn(drive_runtime_bytecode_request_async(input));

    assert!(
        server
            .wait_for_path_async("/request", IO_OBSERVATION_TIMEOUT)
            .await,
        "single-worker request never reached real socket Pending"
    );
    assert_canary_advances(&ticks, "unary response Pending").await;
    assert!(!drive.is_finished());
    server.release("/request");
    for path in ["/stream/left", "/stream/right"] {
        assert!(
            server
                .wait_for_response_head_async(path, IO_OBSERVATION_TIMEOUT)
                .await,
            "single-worker request never opened {path}"
        );
    }
    assert_canary_advances(&ticks, "two simultaneous body streams Pending").await;
    assert!(!drive.is_finished());
    server.release("/stream/left");
    server.release("/stream/right");

    let driven = tokio::time::timeout(IO_OBSERVATION_TIMEOUT, drive)
        .await
        .expect("single-worker request did not finish")
        .expect("join single-worker request");
    keep_ticking.store(false, Ordering::Release);
    canary.await.expect("join single-worker canary");
    assert_exact_outbound_routes(&server.snapshot());
    assert_successful_cleanup(driven, &execution_budget);
}

async fn cancellation_while_http_pending() {
    let fixture = published_positive("lifecycle-cancel");
    let server = Phase5TcpServer::start();
    let cancellation = CancellationToken::new();
    let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(None));
    let input = production_request_input(
        &fixture,
        &server,
        VCP_PATH,
        cancellation.clone(),
        Arc::clone(&execution_budget),
        "phase-5-cancel",
    );
    let drive = tokio::spawn(drive_runtime_bytecode_request_async(input));
    assert!(
        server
            .wait_for_path_async("/request", IO_OBSERVATION_TIMEOUT)
            .await,
        "cancel scenario never reached actual HTTP Pending"
    );
    assert!(!drive.is_finished(), "cancel scenario was pseudo-Ready");
    cancellation.cancel();
    let driven = tokio::time::timeout(IO_OBSERVATION_TIMEOUT, drive)
        .await
        .expect("request cancellation did not wake the pending HTTP operation")
        .expect("join cancelled request");
    server.release("/request");
    assert_terminal_cleanup(driven, &execution_budget, ExecutionWinner::Cancelled);
    assert_eq!(
        server
            .snapshot()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["/request"],
        "a cancelled unary operation must not issue either stream request"
    );
}

async fn deadline_while_http_pending() {
    let fixture = published_positive("lifecycle-deadline");
    let server = Phase5TcpServer::start();
    let deadline = AdmittedRequestDeadline::new(
        Instant::now()
            .checked_add(Duration::from_millis(250))
            .expect("represent Phase 5 deadline"),
    );
    let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(Some(deadline)));
    let input = production_request_input(
        &fixture,
        &server,
        VCP_PATH,
        CancellationToken::new(),
        Arc::clone(&execution_budget),
        "phase-5-deadline",
    );
    let drive = tokio::spawn(drive_runtime_bytecode_request_async(input));
    assert!(
        server
            .wait_for_path_async("/request", IO_OBSERVATION_TIMEOUT)
            .await,
        "deadline scenario never reached actual HTTP Pending"
    );
    assert!(!drive.is_finished(), "deadline scenario was pseudo-Ready");
    let driven = tokio::time::timeout(IO_OBSERVATION_TIMEOUT, drive)
        .await
        .expect("deadline did not wake the pending HTTP operation")
        .expect("join deadline request");
    server.release("/request");
    assert_terminal_cleanup(driven, &execution_budget, ExecutionWinner::DeadlineExceeded);
    assert_eq!(
        server
            .snapshot()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["/request"],
        "an expired unary operation must not issue either stream request"
    );
}

async fn early_break_releases_body_before_late_chunk() {
    const DROP_PATH: &str = "/phase-5/drop-left";
    let fixture = published_positive("lifecycle-early-break");
    let server = Phase5TcpServer::start();
    let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(None));
    let input = production_request_input(
        &fixture,
        &server,
        DROP_PATH,
        CancellationToken::new(),
        Arc::clone(&execution_budget),
        "phase-5-early-break",
    );
    let drive = tokio::spawn(drive_runtime_bytecode_request_async(input));
    for path in ["/stream/drop-left", "/stream/drop-right"] {
        assert!(
            server
                .wait_for_response_head_async(path, IO_OBSERVATION_TIMEOUT)
                .await,
            "early-break request never opened {path}"
        );
    }
    server.release("/stream/drop-left");
    assert!(
        server
            .wait_for_chunks_async("/stream/drop-left", 1, IO_OBSERVATION_TIMEOUT)
            .await,
        "early-break source never delivered its first item"
    );
    server.release("/stream/drop-right");
    assert!(
        server
            .wait_for_peer_close_async("/stream/drop-left", IO_OBSERVATION_TIMEOUT)
            .await,
        "breaking the body loop did not release the affine source before late data"
    );
    server.release("/stream/drop-left#late");
    assert!(
        server
            .wait_for_late_chunk_attempt_async("/stream/drop-left", IO_OBSERVATION_TIMEOUT)
            .await,
        "upstream never raced late data against the released source"
    );
    let driven = tokio::time::timeout(IO_OBSERVATION_TIMEOUT, drive)
        .await
        .expect("early-break request did not reach its response terminal")
        .expect("join early-break request");
    assert_successful_cleanup(driven, &execution_budget);

    let observations = server.snapshot();
    assert_eq!(
        observations
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["/stream/drop-left", "/stream/drop-right"]
    );
    let left = observations
        .iter()
        .find(|entry| entry.path == "/stream/drop-left")
        .expect("left observation");
    assert_eq!(left.chunks_sent, 1, "late item entered the released source");
    assert!(left.peer_closed, "left source never observed exact release");
    assert!(
        left.late_chunk_attempted,
        "upstream did not race late data against the released source"
    );
}

async fn assert_canary_advances(ticks: &Arc<AtomicU64>, phase: &str) {
    let before = ticks.load(Ordering::Relaxed);
    let sampled_ticks = Arc::clone(ticks);
    let (sampled, observation) = tokio::sync::oneshot::channel();
    // This thread is a wall-clock observer only. It never polls the request,
    // provider, or canary, so it cannot manufacture progress for the sole
    // Tokio worker under test.
    let observer = thread::Builder::new()
        .name("phase-5-canary-observer".to_string())
        .spawn(move || {
            thread::sleep(Duration::from_millis(50));
            let _ = sampled.send(sampled_ticks.load(Ordering::Relaxed));
        })
        .expect("spawn Phase 5 canary wall-clock observer");
    let after = observation
        .await
        .expect("Phase 5 canary wall-clock observer exited without sampling");
    observer
        .join()
        .expect("join Phase 5 canary wall-clock observer");
    assert!(
        after >= before.saturating_add(8),
        "the single Tokio worker did not advance during the closed 50ms {phase} gate: {before} -> {after}"
    );
}

fn production_request_input(
    fixture: &super::fixture::PublishedFixture,
    server: &Phase5TcpServer,
    ingress_path: &str,
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    request_id: &str,
) -> BytecodeRequestExecutionInput {
    let gateway = fixture.gateway(ingress_path);
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
                url: format!("http://phase-5.invalid{ingress_path}"),
                path: ingress_path.to_string(),
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

fn assert_successful_cleanup(
    driven: skiff_runtime_request::DrivenBytecodeRequest,
    execution_budget: &ExecutionBudget,
) {
    let inventory = driven.owner_inventory.into_snapshot();
    assert!(
        matches!(&driven.result, Ok(BoundaryResponse::StreamSent)),
        "successful serverStream drive returned {:?}",
        driven.result
    );
    drop(driven.retention);
    assert_zero_current(inventory);
    assert!(inventory.pending.ever_created);
    assert!(inventory.resource.ever_created);
    assert_eq!(
        execution_budget
            .settlement()
            .expect("successful request has one terminal")
            .winner(),
        ExecutionWinner::Succeeded
    );
}

fn assert_terminal_cleanup(
    driven: skiff_runtime_request::DrivenBytecodeRequest,
    execution_budget: &ExecutionBudget,
    winner: ExecutionWinner,
) {
    let inventory = driven.owner_inventory.into_snapshot();
    assert!(
        driven.result.is_err(),
        "terminal race returned a successful response: {:?}",
        driven.result
    );
    drop(driven.retention);
    assert_zero_current(inventory);
    assert!(
        inventory.pending.ever_created,
        "terminal race never parked an actual pending HTTP operation"
    );
    assert_eq!(
        execution_budget
            .settlement()
            .expect("terminal race has one budget winner")
            .winner(),
        winner
    );
}

fn assert_zero_current(inventory: skiff_runtime_request::RequestExecutionOwnerInventorySnapshot) {
    assert_eq!(inventory.pending.current, 0, "pending owners leaked");
    assert_eq!(inventory.resource.current, 0, "resource owners leaked");
    assert_eq!(inventory.child.current, 0, "child owners leaked");
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
