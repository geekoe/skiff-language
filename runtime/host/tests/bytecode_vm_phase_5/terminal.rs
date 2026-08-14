use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use skiff_runtime_transport::protocol::BytecodeRequestDeadlineFrameHeader;

use super::{
    host_harness::{
        health_counters_all_zero, health_counters_one_active_bytecode_request, HostResponse,
        RuntimeHostHarness,
    },
    tcp_server::Phase5TcpServer,
};

const END_THEN_EMIT_PATH: &str = "/phase-5/end-then-emit";
const END_THEN_THROW_PATH: &str = "/phase-5/end-then-throw";
const END_THEN_WAIT_PATH: &str = "/phase-5/end-then-wait";
const PRE_END_WAIT_PATH: &str = "/phase-5/pre-end-wait";

pub async fn production_terminal_arbiter_matrix() {
    let upstream = Phase5TcpServer::start();
    let mut host = RuntimeHostHarness::start("terminal-arbiter", upstream.proxy_url()).await;

    end_then_emit_is_one_wire_terminal(&mut host).await;
    end_then_throw_is_one_wire_terminal(&mut host).await;
    end_then_late_deadline_is_one_wire_terminal(&mut host).await;
    pre_end_deadline_keeps_ordinary_terminal(&mut host).await;
    end_then_session_drop_cleans_on_reconnect(&mut host).await;

    host.close().await;
}

async fn end_then_emit_is_one_wire_terminal(host: &mut RuntimeHostHarness) {
    let request_id = "phase-5-terminal-post-end-emit";
    host.send_http_request(request_id, END_THEN_EMIT_PATH, b"emit", None)
        .await;
    assert_stream_end(host.response_allowing_empty(request_id).await, 209);
    assert_terminal_cleanup(host, "post-End Emit cleanup").await;
}

async fn end_then_throw_is_one_wire_terminal(host: &mut RuntimeHostHarness) {
    let request_id = "phase-5-terminal-post-end-throw";
    host.send_http_request(request_id, END_THEN_THROW_PATH, b"throw", None)
        .await;
    assert_stream_end(host.response_allowing_empty(request_id).await, 210);
    assert_terminal_cleanup(host, "post-End VM throw cleanup").await;
}

async fn end_then_late_deadline_is_one_wire_terminal(host: &mut RuntimeHostHarness) {
    const TIMEOUT_MS: u64 = 500;
    let request_id = "phase-5-terminal-late-deadline";
    host.send_http_request(
        request_id,
        END_THEN_WAIT_PATH,
        b"deadline",
        Some(deadline(TIMEOUT_MS)),
    )
    .await;
    assert_stream_end(host.response_allowing_empty(request_id).await, 211);
    assert_terminal_cleanup(host, "post-End deadline/accounting override cleanup").await;
}

async fn pre_end_deadline_keeps_ordinary_terminal(host: &mut RuntimeHostHarness) {
    const TIMEOUT_MS: u64 = 100;
    let request_id = "phase-5-terminal-pre-enqueue-deadline";
    host.send_http_request(
        request_id,
        PRE_END_WAIT_PATH,
        b"ordinary-error",
        Some(deadline(TIMEOUT_MS)),
    )
    .await;

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
    assert_terminal_cleanup(host, "pre-End deadline ordinary terminal cleanup").await;
}

async fn end_then_session_drop_cleans_on_reconnect(host: &mut RuntimeHostHarness) {
    let request_id = "phase-5-terminal-session-drop";
    host.send_http_request(request_id, END_THEN_WAIT_PATH, b"disconnect", None)
        .await;
    assert_stream_end(host.response_allowing_empty(request_id).await, 211);
    let active = host
        .next_health_matching(
            "post-End request parked before session drop",
            health_counters_one_active_bytecode_request,
        )
        .await;
    assert!(health_counters_one_active_bytecode_request(&active));

    host.disconnect_and_reconnect().await;
    assert_terminal_cleanup(host, "session-drop reconnect cleanup").await;
}

fn assert_stream_end(response: HostResponse, status: u16) {
    assert_eq!(response.status, status);
    assert!(response.headers.is_empty());
    assert!(response.chunks.is_empty());
}

async fn assert_terminal_cleanup(host: &mut RuntimeHostHarness, context: &str) {
    let terminal = host
        .next_health_matching(context, health_counters_all_zero)
        .await;
    assert!(
        health_counters_all_zero(&terminal),
        "{context} left residual request ownership: {terminal:?}"
    );
}

fn deadline(timeout_ms: u64) -> BytecodeRequestDeadlineFrameHeader {
    let expires_at = (OffsetDateTime::now_utc() + time::Duration::seconds(60))
        .format(&Rfc3339)
        .expect("format Phase 5 terminal deadline");
    BytecodeRequestDeadlineFrameHeader {
        timeout_ms,
        expires_at,
    }
}
