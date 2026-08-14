use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use skiff_runtime_transport::protocol::BytecodeRequestDeadlineFrameHeader;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use super::{
    host_harness::{health_counters_all_zero, RuntimeHostHarness},
    tcp_server::{Phase5TcpServer, RequestObservation},
};

const VCP_PATH: &str = "/phase-5/vcp";
const DROP_PATH: &str = "/phase-5/drop-left";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// G5/S5 enters through the production RuntimeHost request entry. The proof
/// never constructs the scheduler's mandatory HTTP port, resource table,
/// executor, handle, or frame; RuntimeHost owns that composition and the
/// canonical WebSocket request is the only ingress.
pub async fn verify_to_scheduler() {
    exercise_scheduler_path("s5-scheduler", "phase-5-s5", false).await;
}

pub async fn lifecycle_race_matrix() {
    cancellation_while_http_pending().await;
    deadline_while_http_pending().await;
    early_break_releases_body_before_late_chunk().await;
}

pub async fn single_worker_canary() {
    exercise_scheduler_path("single-worker-canary", "phase-5-canary", true).await;
}

async fn exercise_scheduler_path(prefix: &str, request_id: &str, observe_worker: bool) {
    let server = Phase5TcpServer::start();
    let mut host = RuntimeHostHarness::start(prefix).await;
    let canary = observe_worker.then(WorkerCanary::start);
    host.send_http_request(request_id, VCP_PATH, server.base_url().as_bytes(), None)
        .await;

    assert!(
        server.wait_for_path_async("/request", IO_TIMEOUT).await,
        "the pinned request target never reached the deterministic upstream"
    );
    if let Some(canary) = &canary {
        canary.assert_advances("unary response Pending").await;
    }
    let pending = host
        .next_health_matching("one pending unary HTTP request", |counters| {
            counters.outbound_requests_pending == 1
                && counters.outbound_stream_leases_active == 0
                && counters.stream_runtime_streams_active == 0
                && counters.task_requests_active == 1
        })
        .await;
    assert_eq!(pending.outbound_requests_pending, 1);
    server.release("/request");

    for path in ["/stream/left", "/stream/right"] {
        assert!(
            server.wait_for_response_head_async(path, IO_TIMEOUT).await,
            "the exact stream target {path} never reached response-head Ready"
        );
    }
    if let Some(canary) = &canary {
        canary
            .assert_advances("two simultaneous body streams Pending")
            .await;
    }
    let active = host
        .next_health_matching("two coexisting table-backed stream handles", |counters| {
            counters.outbound_stream_leases_active == 2
                && counters.stream_runtime_streams_active == 0
                && counters.task_requests_active == 1
        })
        .await;
    assert_eq!(active.outbound_stream_leases_active, 2);
    server.release("/stream/left");
    server.release("/stream/right");

    let response = host.response(request_id).await;
    assert_eq!(response.status, 207);
    assert!(response.headers.is_empty());
    assert_eq!(
        response.chunks.concat(),
        b"U=UNARY|A=LEFT-1LEFT-2|B=RIGHT-1RIGHT-2"
    );
    let terminal = host
        .next_health_matching(
            "successful terminal owner cleanup",
            health_counters_all_zero,
        )
        .await;
    assert!(health_counters_all_zero(&terminal));
    assert_exact_outbound_routes(&server.snapshot());
    host.close().await;
    if let Some(canary) = canary {
        canary.stop().await;
    }
}

async fn cancellation_while_http_pending() {
    let request_id = "phase-5-cancel";
    let server = Phase5TcpServer::start();
    let mut host = RuntimeHostHarness::start("lifecycle-cancel").await;
    host.send_http_request(request_id, VCP_PATH, server.base_url().as_bytes(), None)
        .await;
    assert!(
        server.wait_for_path_async("/request", IO_TIMEOUT).await,
        "cancel scenario never reached actual HTTP Pending"
    );
    let pending = host
        .next_health_matching("cancellable pending HTTP request", |counters| {
            counters.outbound_requests_pending == 1 && counters.task_requests_active == 1
        })
        .await;
    assert_eq!(pending.stream_runtime_streams_active, 0);

    host.send_cancel(request_id, "caller_cancel").await;
    let terminal = host
        .next_health_matching("cancel winner terminal cleanup", health_counters_all_zero)
        .await;
    assert!(health_counters_all_zero(&terminal));
    assert_only_unary_route(&server.snapshot(), "cancelled");
    host.close().await;
}

async fn deadline_while_http_pending() {
    const TIMEOUT_MS: u64 = 2_000;
    let request_id = "phase-5-deadline";
    let server = Phase5TcpServer::start();
    let mut host = RuntimeHostHarness::start("lifecycle-deadline").await;
    let expires_at = (OffsetDateTime::now_utc()
        + time::Duration::milliseconds(
            i64::try_from(TIMEOUT_MS).expect("Phase 5 deadline fits i64"),
        ))
    .format(&Rfc3339)
    .expect("format Phase 5 RFC3339 deadline");
    host.send_http_request(
        request_id,
        VCP_PATH,
        server.base_url().as_bytes(),
        Some(BytecodeRequestDeadlineFrameHeader {
            timeout_ms: TIMEOUT_MS,
            expires_at,
        }),
    )
    .await;
    assert!(
        server.wait_for_path_async("/request", IO_TIMEOUT).await,
        "deadline scenario never reached actual HTTP Pending"
    );
    let pending = host
        .next_health_matching("deadline-owned pending HTTP request", |counters| {
            counters.outbound_requests_pending == 1 && counters.task_requests_active == 1
        })
        .await;
    assert_eq!(pending.stream_runtime_streams_active, 0);

    let error = host.control_error(request_id).await;
    assert_eq!(error.code, "TimeoutError");
    assert_eq!(error.message, "execution deadline exceeded");
    assert_eq!(
        error
            .details
            .as_ref()
            .and_then(|details| details.get("reason"))
            .and_then(serde_json::Value::as_str),
        Some("deadlineExceeded")
    );
    let terminal = host
        .next_health_matching("deadline winner terminal cleanup", health_counters_all_zero)
        .await;
    assert!(health_counters_all_zero(&terminal));
    assert_only_unary_route(&server.snapshot(), "expired");
    host.close().await;
}

async fn early_break_releases_body_before_late_chunk() {
    let request_id = "phase-5-early-break";
    let server = Phase5TcpServer::start();
    let mut host = RuntimeHostHarness::start("lifecycle-early-break").await;
    host.send_http_request(request_id, DROP_PATH, server.base_url().as_bytes(), None)
        .await;
    for path in ["/stream/drop-left", "/stream/drop-right"] {
        assert!(
            server.wait_for_response_head_async(path, IO_TIMEOUT).await,
            "early-break request never opened {path}"
        );
    }
    let active = host
        .next_health_matching("two early-break stream handles", |counters| {
            counters.outbound_stream_leases_active == 2
                && counters.stream_runtime_streams_active == 0
                && counters.task_requests_active == 1
        })
        .await;
    assert_eq!(active.outbound_stream_leases_active, 2);

    server.release("/stream/drop-left");
    assert!(
        server
            .wait_for_chunks_async("/stream/drop-left", 1, IO_TIMEOUT)
            .await,
        "early-break source never delivered its first item"
    );
    server.release("/stream/drop-right");
    assert!(
        server
            .wait_for_peer_close_async("/stream/drop-left", IO_TIMEOUT)
            .await,
        "breaking the body loop did not release the affine source before late data"
    );
    server.release("/stream/drop-left#late");
    assert!(
        server
            .wait_for_late_chunk_attempt_async("/stream/drop-left", IO_TIMEOUT)
            .await,
        "upstream never raced late data against the released source"
    );

    let response = host.response(request_id).await;
    assert_eq!(response.status, 208);
    assert_eq!(
        response.chunks.concat(),
        b"A=DROP-LEFT-1|B=DROP-RIGHT-1DROP-RIGHT-2"
    );
    let terminal = host
        .next_health_matching(
            "early-break terminal owner cleanup",
            health_counters_all_zero,
        )
        .await;
    assert!(health_counters_all_zero(&terminal));
    host.close().await;

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

fn assert_only_unary_route(observations: &[RequestObservation], terminal: &str) {
    assert_eq!(
        observations
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["/request"],
        "a {terminal} unary operation must not issue either stream request"
    );
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

struct WorkerCanary {
    keep_ticking: Arc<AtomicBool>,
    ticks: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl WorkerCanary {
    fn start() -> Self {
        let keep_ticking = Arc::new(AtomicBool::new(true));
        let ticks = Arc::new(AtomicU64::new(0));
        let task = {
            let keep_ticking = Arc::clone(&keep_ticking);
            let ticks = Arc::clone(&ticks);
            tokio::spawn(async move {
                while keep_ticking.load(Ordering::Acquire) {
                    ticks.fetch_add(1, Ordering::Relaxed);
                    tokio::task::yield_now().await;
                }
            })
        };
        Self {
            keep_ticking,
            ticks,
            task,
        }
    }

    async fn assert_advances(&self, phase: &str) {
        let before = self.ticks.load(Ordering::Relaxed);
        let sampled_ticks = Arc::clone(&self.ticks);
        let (sampled, observation) = tokio::sync::oneshot::channel();
        // This thread is a wall-clock observer only. It never polls the
        // RuntimeHost, provider, request, or canary.
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

    async fn stop(self) {
        self.keep_ticking.store(false, Ordering::Release);
        self.task.await.expect("join Phase 5 single-worker canary");
    }
}
