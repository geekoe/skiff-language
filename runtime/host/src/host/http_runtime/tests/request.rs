use std::{
    io::ErrorKind,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use serde_json::{json, Value};
use skiff_runtime_capability_context::{CancellationSignals, CancellationToken};
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    sync::mpsc,
    time::{sleep, timeout},
};

use crate::{
    capability_context::{HttpRuntimeOptions, TARGET_STD_HTTP_REQUEST},
    config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    error::RuntimeError,
    host::http_runtime::{
        request,
        request::{request_inner, request_with_cancellation_and_options},
        HTTP_REQUEST_TIMEOUT_REASON,
    },
};

use super::helpers::{
    bytes_body, empty_body, output_body_bytes, read_request, request_allowing_unsafe_targets,
    request_input, run_test_server, RequestCapture, TestResponse,
};

#[tokio::test]
async fn request_supports_plain_sensitive_headers() {
    let (tx, mut rx) = mpsc::channel::<RequestCapture>(1);
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: b"ok".to_vec(),
        delay_ms: None,
        body_delay_ms: None,
    };
    let (url, handle) = run_test_server(
        response,
        Arc::new(move |request| {
            let _ = tx.try_send(request);
        }),
    )
    .await;

    let mut input = request_input("GET", &url, empty_body(), None);
    input["headers"] = json!([
        {"name": "authorization", "value": "Bearer dashscope-key"},
        {"name": "x-api-key", "value": "raw-key"}
    ]);

    request_inner(
        &input,
        None,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        None,
        HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
    )
    .await
    .expect("request should succeed");
    let request = rx.recv().await.expect("request was captured");
    assert!(request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == "Bearer dashscope-key"
    }));
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("x-api-key") && value == "raw-key"));

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn request_sends_bytes_body_and_returns_bytes_response() {
    let (tx, mut rx) = mpsc::channel::<RequestCapture>(1);
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: b"{\"message\":\"hello\"}".to_vec(),
        delay_ms: None,
        body_delay_ms: None,
    };
    let (url, handle) = run_test_server(
        response,
        Arc::new(move |request| {
            let _ = tx.try_send(request);
        }),
    )
    .await;

    let input = request_input(
        "POST",
        &format!("{url}/chat"),
        bytes_body(br#"{"text":"hello"}"#),
        None,
    );
    let mut input = input;
    input["headers"] = json!([
        {"name": "Content-Type", "value": "application/json"}
    ]);
    let output = request_allowing_unsafe_targets(&input, None, None)
        .await
        .expect("http request should succeed");

    let request = rx.recv().await.expect("request was captured");
    assert_eq!(request.method, "POST");
    assert_eq!(output.get("status").and_then(Value::as_u64), Some(200));
    assert_eq!(
        output_body_bytes(&output),
        b"{\"message\":\"hello\"}".to_vec()
    );

    handle.await.expect("server should complete");
    assert_eq!(request.body, b"{\"text\":\"hello\"}".to_vec());
    assert!(request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type") && value == "application/json"
    }));
}

#[tokio::test]
async fn request_returns_non_2xx_status_without_error() {
    let response = TestResponse {
        status: 404,
        headers: vec![(
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )],
        body: b"not found".to_vec(),
        delay_ms: None,
        body_delay_ms: None,
    };
    let (url, handle) = run_test_server(response, Arc::new(|_| {})).await;

    let input = request_input("GET", &url, empty_body(), None);
    let output = request_allowing_unsafe_targets(&input, None, None)
        .await
        .expect("404 should not error");

    assert_eq!(output.get("status").and_then(Value::as_u64), Some(404));
    assert_eq!(output_body_bytes(&output), b"not found".to_vec());

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn request_timeout_maps_to_provider_unavailable() {
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: b"too late".to_vec(),
        delay_ms: Some(100),
        body_delay_ms: None,
    };
    let (url, handle) = run_test_server(response, Arc::new(|_| {})).await;

    let input = request_input("GET", &url, empty_body(), Some(10));
    let error = request_allowing_unsafe_targets(&input, Some(10), None)
        .await
        .expect_err("timeout should fail");

    match error {
        RuntimeError::ProviderUnavailable { target, reason } => {
            assert_eq!(target, TARGET_STD_HTTP_REQUEST);
            assert_eq!(reason, HTTP_REQUEST_TIMEOUT_REASON);
        }
        other => panic!("expected ProviderUnavailable, got {other:?}"),
    }

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn request_reuses_keep_alive_connection_for_compatible_requests() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind keep-alive listener");
    let addr = listener
        .local_addr()
        .expect("read keep-alive listener addr");
    let accept_count = Arc::new(AtomicUsize::new(0));
    let server_accept_count = accept_count.clone();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept keep-alive client");
        server_accept_count.fetch_add(1, Ordering::SeqCst);

        read_request(&mut stream).await;
        write_keep_alive_response(&mut stream, b"first", None, false).await;

        read_request(&mut stream).await;
        write_keep_alive_response(&mut stream, b"second", None, true).await;
    });

    let url = format!("http://{addr}/pool");
    let first_input = request_input("GET", &url, empty_body(), Some(1000));
    let first_output = request_allowing_unsafe_targets(&first_input, None, None)
        .await
        .expect("first keep-alive request should succeed");
    assert_eq!(output_body_bytes(&first_output), b"first".to_vec());

    let second_input = request_input("GET", &url, empty_body(), Some(750));
    let second_output = request_allowing_unsafe_targets(&second_input, None, None)
        .await
        .expect("second keep-alive request should reuse the cached client");
    assert_eq!(output_body_bytes(&second_output), b"second".to_vec());

    handle.await.expect("keep-alive server should complete");
    assert_eq!(accept_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn request_timeout_is_per_request_without_fragmenting_client_cache() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind timeout keep-alive listener");
    let addr = listener
        .local_addr()
        .expect("read timeout keep-alive listener addr");
    let (seen_tx, mut seen_rx) = mpsc::channel::<usize>(2);
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept timeout keep-alive client");

        read_request(&mut stream).await;
        let _ = seen_tx.send(1).await;
        write_keep_alive_response(&mut stream, b"fast", None, false).await;

        read_request(&mut stream).await;
        let _ = seen_tx.send(2).await;
        write_keep_alive_response(&mut stream, b"slow", Some(100), true).await;
    });

    let url = format!("http://{addr}/timeout");
    let first_input = request_input("GET", &url, empty_body(), Some(1000));
    let first_output = request_allowing_unsafe_targets(&first_input, None, None)
        .await
        .expect("first request should succeed");
    assert_eq!(output_body_bytes(&first_output), b"fast".to_vec());
    assert_eq!(seen_rx.recv().await, Some(1));

    let second_input = request_input("GET", &url, empty_body(), Some(20));
    let error = request_allowing_unsafe_targets(&second_input, None, None)
        .await
        .expect_err("second request should use its shorter per-request timeout");
    match error {
        RuntimeError::ProviderUnavailable { target, reason } => {
            assert_eq!(target, TARGET_STD_HTTP_REQUEST);
            assert_eq!(reason, HTTP_REQUEST_TIMEOUT_REASON);
        }
        other => panic!("expected ProviderUnavailable, got {other:?}"),
    }

    let second_seen = timeout(Duration::from_secs(1), seen_rx.recv())
        .await
        .expect("server should receive second request on the existing connection");
    assert_eq!(second_seen, Some(2));

    handle
        .await
        .expect("timeout keep-alive server should complete");
}

#[tokio::test]
async fn request_canceled_before_call_returns_cancelled() {
    let input = request_input("GET", "https://example.com", empty_body(), None);

    let cancelled = Arc::new(AtomicBool::new(true));
    let error = request(
        &input,
        None,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        Some(cancelled.as_ref()),
    )
    .await
    .expect_err("cancelled request should fail");
    assert!(error.is_request_cancelled());
}

#[tokio::test]
async fn request_cancellation_token_before_call_returns_cancelled() {
    let input = request_input("GET", "https://example.com", empty_body(), None);
    let token = CancellationToken::new();
    token.cancel();

    let error = request_with_cancellation_and_options(
        &input,
        None,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        CancellationSignals::from_tokens([token]),
        HttpRuntimeOptions::from_env(),
    )
    .await
    .expect_err("cancelled request should fail");
    assert!(error.is_request_cancelled());
}

#[tokio::test]
async fn request_canceled_while_waiting_for_send_returns_cancelled() {
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: b"late response".to_vec(),
        delay_ms: Some(1000),
        body_delay_ms: None,
    };
    let (url, handle) = run_test_server(response, Arc::new(|_| {})).await;

    let input = request_input("GET", &url, empty_body(), Some(2000));
    let cancelled = Arc::new(AtomicBool::new(false));
    let canceled_ref = cancelled.clone();
    let canceled_input = request_allowing_unsafe_targets(&input, None, Some(canceled_ref.as_ref()));
    let cancel_task = tokio::spawn(async move {
        sleep(std::time::Duration::from_millis(25)).await;
        cancelled.store(true, Ordering::SeqCst);
    });

    let error = canceled_input
        .await
        .expect_err("send-cancelled request should fail");
    cancel_task.await.expect("cancel timer should finish");
    assert!(error.is_request_cancelled());

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn request_canceled_while_waiting_for_body_returns_cancelled() {
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: b"hello body".to_vec(),
        delay_ms: None,
        body_delay_ms: Some(1000),
    };
    let (url, handle) = run_test_server(response, Arc::new(|_| {})).await;

    let input = request_input("GET", &url, empty_body(), Some(2000));
    let cancelled = Arc::new(AtomicBool::new(false));
    let request_task = tokio::spawn({
        let cancelled = cancelled.clone();
        async move { request_allowing_unsafe_targets(&input, None, Some(cancelled.as_ref())).await }
    });
    sleep(std::time::Duration::from_millis(25)).await;
    cancelled.store(true, Ordering::SeqCst);

    let error = request_task
        .await
        .expect("request task should resolve")
        .expect_err("body-cancelled request should fail");
    assert!(error.is_request_cancelled());

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn request_with_bytes_body_has_no_default_content_type_and_returns_bytes_response() {
    let (tx, mut rx) = mpsc::channel::<RequestCapture>(1);
    let response = TestResponse {
        status: 200,
        headers: vec![(
            "Content-Type".to_string(),
            "application/octet-stream".to_string(),
        )],
        body: vec![0, 159, 146, 150],
        delay_ms: None,
        body_delay_ms: None,
    };
    let (url, handle) = run_test_server(
        response,
        Arc::new(move |request| {
            let _ = tx.try_send(request);
        }),
    )
    .await;

    let input = request_input("POST", &url, bytes_body(b"Hello"), None);
    let output = request_allowing_unsafe_targets(&input, None, None)
        .await
        .expect("request should succeed");

    let request = rx.recv().await.expect("request should be captured");
    assert_eq!(request.body, b"Hello".to_vec());
    assert!(!request
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type")));
    assert_eq!(output_body_bytes(&output), vec![0, 159, 146, 150]);

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn request_ignores_legacy_max_response_bytes_field() {
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: b"larger than legacy request cap".to_vec(),
        delay_ms: None,
        body_delay_ms: None,
    };

    let (url, handle) = run_test_server(response, Arc::new(|_| {})).await;

    let mut input = request_input("GET", &url, empty_body(), None);
    input
        .as_object_mut()
        .expect("request input should be object")
        .insert("maxResponseBytes".to_string(), Value::Number(1.into()));

    let output = request_allowing_unsafe_targets(&input, None, None)
        .await
        .expect("legacy request-level maxResponseBytes should be ignored");
    assert_eq!(output.get("status").and_then(Value::as_u64), Some(200));

    handle.await.expect("server should complete");
}

async fn write_keep_alive_response(
    stream: &mut tokio::net::TcpStream,
    body: &[u8],
    delay_ms: Option<u64>,
    close: bool,
) {
    if let Some(delay_ms) = delay_ms {
        sleep(Duration::from_millis(delay_ms)).await;
    }

    let connection = if close { "close" } else { "keep-alive" };
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n",
        body.len()
    );
    if let Err(error) = stream.write_all(headers.as_bytes()).await {
        if matches!(
            error.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
        ) {
            return;
        }
        panic!("write keep-alive response headers: {error}");
    }
    if let Err(error) = stream.write_all(body).await {
        if matches!(
            error.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
        ) {
            return;
        }
        panic!("write keep-alive response body: {error}");
    }
}
