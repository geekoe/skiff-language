use std::{sync::Arc, time::Duration};

use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::mpsc};

use crate::host::http_runtime::egress::with_http_admin_unsafe_override_for_test;
use crate::host::http_runtime::request;
use crate::{config::DEFAULT_HTTP_RESPONSE_MAX_BYTES, error::RuntimeError};

use super::helpers::{
    empty_body, read_request, request_allowing_unsafe_targets,
    request_allowing_unsafe_targets_with_runtime_proxy, request_input, request_with_runtime_proxy,
    run_test_server, with_http_proxy_env_for_test, write_response, RequestCapture, TestResponse,
};

fn assert_http_error_contains(error: RuntimeError, expected: &str) {
    let payload = error.payload();
    assert_eq!(payload.code, "std.http.HttpError");
    assert!(
        payload.message.contains(expected),
        "expected {expected:?} in {:?}",
        payload.message
    );
}

#[tokio::test]
async fn request_rejects_unsafe_host_targets_by_default() {
    let unsafe_urls = [
        "http://localhost:8080/path",
        "http://127.0.0.1:8080/path",
        "http://[::ffff:127.0.0.1]:8080/path",
        "http://169.254.169.254:8080",
    ];
    with_http_admin_unsafe_override_for_test(false, async {
        for url in unsafe_urls {
            let input = request_input("GET", url, empty_body(), None);
            let error = request(&input, None, DEFAULT_HTTP_RESPONSE_MAX_BYTES, None)
                .await
                .expect_err("unsafe targets should be rejected");
            assert_http_error_contains(error, "blocked network target");
        }
    })
    .await;
}

#[tokio::test]
async fn request_rejects_legacy_proxy_url_input() {
    let input = json!({
        "method": "GET",
        "url": "http://93.184.216.34/",
        "headers": [],
        "body": empty_body(),
        "timeoutMs": Value::Null,
        "proxyUrl": "http://127.0.0.1:8080",
    });

    with_http_admin_unsafe_override_for_test(false, async {
        let error = request(&input, None, DEFAULT_HTTP_RESPONSE_MAX_BYTES, None)
            .await
            .expect_err("legacy proxyUrl input should be rejected");
        assert_http_error_contains(error, "proxyUrl");
    })
    .await;
}

#[tokio::test]
async fn request_does_not_follow_redirects() {
    let redirect_target = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect target");
    let redirect_target_url = format!(
        "http://{}",
        redirect_target
            .local_addr()
            .expect("read redirect target addr")
    );
    let response = TestResponse {
        status: 302,
        headers: vec![("Location".to_string(), redirect_target_url)],
        body: Vec::new(),
        delay_ms: None,
        body_delay_ms: None,
    };
    let (url, handle) = run_test_server(response, Arc::new(|_| {})).await;

    let input = request_input("GET", &url, empty_body(), None);
    let output = request_allowing_unsafe_targets(&input, None, None)
        .await
        .expect("redirect response should be returned");

    assert_eq!(output.get("status").and_then(Value::as_u64), Some(302));
    let redirected =
        tokio::time::timeout(Duration::from_millis(50), redirect_target.accept()).await;
    assert!(redirected.is_err(), "redirect target should not be called");

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn request_ignores_proxy_environment() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let proxy_url = format!(
        "http://{}",
        proxy_listener
            .local_addr()
            .expect("read proxy listener addr")
    );
    let (proxy_tx, mut proxy_rx) = mpsc::channel::<RequestCapture>(1);
    let proxy_handle = tokio::spawn(async move {
        let (mut stream, _) = proxy_listener
            .accept()
            .await
            .expect("accept proxy connection");
        let request = read_request(&mut stream).await;
        let _ = proxy_tx.send(request).await;
        write_response(
            &mut stream,
            TestResponse {
                status: 502,
                headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                body: b"proxy was used".to_vec(),
                delay_ms: None,
                body_delay_ms: None,
            },
        )
        .await;
    });

    let (target_tx, mut target_rx) = mpsc::channel::<RequestCapture>(1);
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: b"direct target".to_vec(),
        delay_ms: None,
        body_delay_ms: None,
    };
    let (target_url, target_handle) = run_test_server(
        response,
        Arc::new(move |request| {
            let _ = target_tx.try_send(request);
        }),
    )
    .await;

    let input = request_input("GET", &target_url, empty_body(), Some(1000));
    let output = with_http_proxy_env_for_test(
        &proxy_url,
        request_allowing_unsafe_targets(&input, None, None),
    )
    .await
    .expect("direct request should succeed without using proxy env");

    assert_eq!(output.get("status").and_then(Value::as_u64), Some(200));
    assert_eq!(
        target_rx
            .recv()
            .await
            .expect("target should receive request")
            .method,
        "GET"
    );
    let proxied = tokio::time::timeout(Duration::from_millis(50), proxy_rx.recv()).await;
    assert!(
        proxied.is_err(),
        "proxy listener should not receive request"
    );

    target_handle.await.expect("target server should complete");
    proxy_handle.abort();
}

#[tokio::test]
async fn request_uses_runtime_configured_local_proxy() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let proxy_listener_url = format!(
        "http://{}",
        proxy_listener
            .local_addr()
            .expect("read proxy listener addr")
    );
    let (proxy_tx, mut proxy_rx) = mpsc::channel::<RequestCapture>(1);
    let proxy_handle = tokio::spawn(async move {
        let (mut stream, _) = proxy_listener.accept().await.expect("accept proxy request");
        let request = read_request(&mut stream).await;
        let _ = proxy_tx.send(request).await;
        write_response(
            &mut stream,
            TestResponse {
                status: 200,
                headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                body: b"from proxy".to_vec(),
                delay_ms: None,
                body_delay_ms: None,
            },
        )
        .await;
    });

    let input = request_input("GET", "http://93.184.216.34/proxied", empty_body(), None);

    let output = request_with_runtime_proxy(&input, None, None, proxy_listener_url)
        .await
        .expect("runtime proxy request should succeed");
    assert_eq!(output.get("status").and_then(Value::as_u64), Some(200));

    let proxied = proxy_rx.recv().await.expect("proxy should receive request");
    assert_eq!(proxied.method, "GET");
    assert!(
        proxied.target.contains("http://93.184.216.34/proxied"),
        "expected proxy request target to contain absolute URL, got {}",
        proxied.target
    );
    assert!(proxied
        .headers
        .iter()
        .any(|(name, value)| { name.eq_ignore_ascii_case("host") && value == "93.184.216.34" }));

    proxy_handle.await.expect("proxy handler should complete");
}

#[tokio::test]
async fn request_rejects_proxy_authorization_header_with_runtime_proxy() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let proxy_listener_url = format!(
        "http://{}",
        proxy_listener
            .local_addr()
            .expect("read proxy listener addr")
    );
    let (proxy_tx, mut proxy_rx) = mpsc::channel::<RequestCapture>(1);
    let proxy_handle = tokio::spawn(async move {
        let (mut stream, _) = proxy_listener.accept().await.expect("accept proxy request");
        let request = read_request(&mut stream).await;
        let _ = proxy_tx.send(request).await;
    });

    let mut input = request_input("GET", "http://93.184.216.34/proxy-auth", empty_body(), None);
    input["headers"] = json!([
        {"name": "pRoXy-AuThOrIzAtIoN", "value": "Basic c2VydmljZTpwYXNz"}
    ]);

    let error = request_with_runtime_proxy(&input, None, None, proxy_listener_url)
        .await
        .expect_err("service-owned proxy auth header should fail closed");
    assert_http_error_contains(error, "Proxy-Authorization");

    let proxied = tokio::time::timeout(Duration::from_millis(50), proxy_rx.recv()).await;
    assert!(
        proxied.is_err(),
        "proxy listener should not receive request with service proxy auth"
    );
    proxy_handle.abort();
}

#[tokio::test]
async fn request_blocks_unsafe_target_even_with_runtime_proxy() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let proxy_listener_url = format!(
        "http://{}",
        proxy_listener
            .local_addr()
            .expect("read proxy listener addr")
    );
    let (proxy_tx, mut proxy_rx) = mpsc::channel::<RequestCapture>(1);
    let proxy_handle = tokio::spawn(async move {
        let (mut stream, _) = proxy_listener.accept().await.expect("accept proxy request");
        let request = read_request(&mut stream).await;
        let _ = proxy_tx.send(request).await;
    });

    let input = request_input(
        "GET",
        "http://127.0.0.1:9/blocked-target",
        empty_body(),
        None,
    );

    with_http_admin_unsafe_override_for_test(false, async {
        let error = request_with_runtime_proxy(&input, None, None, proxy_listener_url)
            .await
            .expect_err("unsafe target should still be rejected");
        assert_http_error_contains(error, "url");
    })
    .await;

    let proxied = tokio::time::timeout(Duration::from_millis(50), proxy_rx.recv()).await;
    assert!(
        proxied.is_err(),
        "proxy listener should not receive blocked target request"
    );
    proxy_handle.abort();
}

#[tokio::test]
async fn request_can_use_runtime_proxy_for_unsafe_targets_when_admin_allows() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let proxy_listener_url = format!(
        "http://{}",
        proxy_listener
            .local_addr()
            .expect("read proxy listener addr")
    );
    let (proxy_tx, mut proxy_rx) = mpsc::channel::<RequestCapture>(1);
    let proxy_handle = tokio::spawn(async move {
        let (mut stream, _) = proxy_listener.accept().await.expect("accept proxy request");
        let request = read_request(&mut stream).await;
        let _ = proxy_tx.send(request).await;
        write_response(
            &mut stream,
            TestResponse {
                status: 200,
                headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                body: b"from runtime proxy".to_vec(),
                delay_ms: None,
                body_delay_ms: None,
            },
        )
        .await;
    });

    let input = request_input(
        "GET",
        "http://localhost:9/allowed-by-admin",
        empty_body(),
        None,
    );

    let output =
        request_allowing_unsafe_targets_with_runtime_proxy(&input, None, None, proxy_listener_url)
            .await
            .expect("admin-allowed runtime proxy request should succeed");

    assert_eq!(output.get("status").and_then(Value::as_u64), Some(200));
    let proxied = proxy_rx.recv().await.expect("proxy should receive request");
    assert!(proxied
        .target
        .contains("http://localhost:9/allowed-by-admin"));

    proxy_handle.await.expect("proxy handler should complete");
}
