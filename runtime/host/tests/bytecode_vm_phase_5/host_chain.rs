use std::time::Duration;

use skiff_runtime_transport::protocol::RuntimeHealthCountersFrameHeader;

use super::{
    host_harness::{
        health_counters_all_zero, health_counters_one_active_bytecode_request, RuntimeHostHarness,
    },
    tcp_server::{Phase5TcpServer, RequestObservation},
};

const VCP_PATH: &str = "/phase-5/vcp";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

struct TopLevelEvidence {
    response_status: u16,
    response_headers: Vec<(String, String)>,
    response_chunks: Vec<Vec<u8>>,
    outbound: Vec<RequestObservation>,
    pending_health: RuntimeHealthCountersFrameHeader,
    active_health: RuntimeHealthCountersFrameHeader,
    terminal_health: RuntimeHealthCountersFrameHeader,
}

pub async fn scheduler_to_request_response() {
    let evidence = drive_top_level_vcp("s6-response", "phase-5-s6").await;
    assert_eq!(evidence.response_status, 207);
    assert!(evidence.response_headers.is_empty());
    assert_eq!(
        evidence.response_chunks.concat(),
        b"U=UNARY|A=LEFT-1LEFT-2|B=RIGHT-1RIGHT-2"
    );
}

pub async fn vcp_production_composition() {
    let evidence = drive_top_level_vcp("vcp-host", "phase-5-vcp").await;
    assert_eq!(evidence.response_status, 207);
    assert_eq!(evidence.response_chunks.len(), 6);
    assert_eq!(
        evidence.response_chunks.concat(),
        b"U=UNARY|A=LEFT-1LEFT-2|B=RIGHT-1RIGHT-2"
    );
    assert_eq!(exact_routes(&evidence.outbound), expected_vcp_routes());
}

pub async fn structural_no_bypass() {
    let evidence = drive_top_level_vcp("structural", "phase-5-structural").await;
    assert_eq!(evidence.response_status, 207);
    assert_eq!(
        evidence.response_chunks.concat(),
        b"U=UNARY|A=LEFT-1LEFT-2|B=RIGHT-1RIGHT-2",
        "the structural proof must execute the production HTTP provider"
    );
    assert_eq!(
        exact_routes(&evidence.outbound),
        expected_vcp_routes(),
        "no test executor may replace the exact production outbound routes"
    );
    assert_eq!(
        evidence.pending_health.outbound_requests_pending, 0,
        "the bytecode HTTP request must not enter the legacy actor/control request registry"
    );
    assert_eq!(
        evidence.active_health.outbound_stream_leases_active, 0,
        "bytecode response streams must not enter the legacy actor/control lease registry"
    );
    assert_eq!(
        evidence.active_health.stream_runtime_streams_active, 0,
        "the legacy StreamRuntime registry must stay inactive on the bytecode path"
    );
    assert_eq!(
        evidence.active_health.task_requests_active, 1,
        "the socket-observed response streams must remain owned by one active bytecode request"
    );
    assert!(
        health_counters_all_zero(&evidence.terminal_health),
        "the single production owner inventory did not return to zero: {:?}",
        evidence.terminal_health
    );
}

async fn drive_top_level_vcp(prefix: &str, request_id: &str) -> TopLevelEvidence {
    let upstream = Phase5TcpServer::start();
    let mut host = RuntimeHostHarness::start(prefix, upstream.proxy_url()).await;
    host.send_http_request(request_id, VCP_PATH, upstream.origin_url().as_bytes(), None)
        .await;

    tokio::select! {
        reached = upstream.wait_for_path_async("/request", IO_TIMEOUT) => {
            assert!(reached, "RuntimeHost never dispatched the pinned unary HTTP target");
        }
        error = host.control_error(request_id) => {
            panic!("RuntimeHost rejected the serverStream request before unary dispatch: {error:?}");
        }
    }
    let pending_health = host
        .next_health_matching(
            "one active bytecode request with a socket-observed pending unary HTTP request",
            health_counters_one_active_bytecode_request,
        )
        .await;
    upstream.release("/request");

    for path in ["/stream/left", "/stream/right"] {
        assert!(
            upstream
                .wait_for_response_head_async(path, IO_TIMEOUT)
                .await,
            "RuntimeHost never opened exact stream target {path}"
        );
    }
    let active_health = host
        .next_health_matching(
            "one active bytecode request with two socket-observed response streams",
            health_counters_one_active_bytecode_request,
        )
        .await;
    upstream.release("/stream/left");
    upstream.release("/stream/right");

    let response = host.response(request_id).await;
    let terminal_health = host
        .next_health_matching("terminal zero owner inventory", health_counters_all_zero)
        .await;
    host.close().await;

    TopLevelEvidence {
        response_status: response.status,
        response_headers: response.headers,
        response_chunks: response.chunks,
        outbound: upstream.snapshot(),
        pending_health,
        active_health,
        terminal_health,
    }
}

fn exact_routes(observations: &[RequestObservation]) -> Vec<(&str, &str)> {
    observations
        .iter()
        .map(|entry| (entry.method.as_str(), entry.path.as_str()))
        .collect()
}

fn expected_vcp_routes() -> [(&'static str, &'static str); 3] {
    [
        ("GET", "/request"),
        ("GET", "/stream/left"),
        ("GET", "/stream/right"),
    ]
}
