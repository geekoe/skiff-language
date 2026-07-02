use std::{
    io::ErrorKind,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde_json::{json, Value};
use skiff_runtime_boundary::value::{bytes_payload, bytes_value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::sleep,
};

use crate::{
    capability_context::HttpRuntimeOptions,
    config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    error::Result,
    host::http_runtime::{egress::HTTP_EGRESS_OVERRIDE_TEST_LOCK, request::request_inner},
};

pub(super) async fn request_allowing_unsafe_targets(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Option<&AtomicBool>,
) -> Result<Value> {
    request_inner(
        input,
        frame_deadline_ms,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        cancelled,
        HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
    )
    .await
}

pub(super) async fn request_with_runtime_proxy(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Option<&AtomicBool>,
    proxy_url: String,
) -> Result<Value> {
    request_inner(
        input,
        frame_deadline_ms,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        cancelled,
        HttpRuntimeOptions::from_env().with_egress_proxy(Some(proxy_url)),
    )
    .await
}

pub(super) async fn request_allowing_unsafe_targets_with_runtime_proxy(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Option<&AtomicBool>,
    proxy_url: String,
) -> Result<Value> {
    request_inner(
        input,
        frame_deadline_ms,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        cancelled,
        HttpRuntimeOptions::allowing_unsafe_targets_for_tests().with_egress_proxy(Some(proxy_url)),
    )
    .await
}

pub(super) async fn with_http_proxy_env_for_test<R>(
    proxy_url: &str,
    f: impl std::future::Future<Output = R>,
) -> R {
    let lock = HTTP_EGRESS_OVERRIDE_TEST_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = lock.lock().await;

    let env_names = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ];
    let previous = env_names
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect::<Vec<_>>();

    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        std::env::set_var(name, proxy_url);
    }
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");

    let output = f.await;

    for (name, value) in previous {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    output
}

pub(super) struct RequestCapture {
    pub(super) method: String,
    pub(super) target: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

pub(super) struct TestResponse {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) delay_ms: Option<u64>,
    pub(super) body_delay_ms: Option<u64>,
}

pub(super) async fn read_request(stream: &mut TcpStream) -> RequestCapture {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let n = stream.read(&mut chunk).await.expect("read request chunk");
        if n == 0 {
            panic!("connection closed before request headers");
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let header_part =
        std::str::from_utf8(&buffer[..header_end]).expect("request headers should be valid UTF-8");
    let mut lines = header_part.split("\r\n");
    let request_line = lines
        .next()
        .expect("request must have a request line")
        .split_whitespace()
        .collect::<Vec<_>>();
    let method = request_line.first().copied().unwrap_or("").to_string();
    let target = request_line.get(1).copied().unwrap_or("").to_string();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(": ")
            .or_else(|| line.split_once(":"))
            .expect("header line must contain colon");
        headers.push((name.to_string(), value.trim_start().to_string()));
    }

    let content_length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or_default();

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).await.expect("read request body");
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    RequestCapture {
        method,
        target,
        headers,
        body,
    }
}

pub(super) async fn write_response(stream: &mut TcpStream, response: TestResponse) {
    if let Some(delay_ms) = response.delay_ms {
        sleep(Duration::from_millis(delay_ms)).await;
    }

    let mut raw_headers = String::new();
    raw_headers.push_str(&format!("HTTP/1.1 {} OK\r\n", response.status));
    raw_headers.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    for (name, value) in response.headers {
        raw_headers.push_str(&format!("{name}: {value}\r\n"));
    }
    raw_headers.push_str("Connection: close\r\n\r\n");

    if let Err(error) = stream.write_all(raw_headers.as_bytes()).await {
        if error.kind() == ErrorKind::BrokenPipe {
            return;
        }
        panic!("write response headers: {error}");
    }
    if let Some(delay_ms) = response.body_delay_ms {
        sleep(Duration::from_millis(delay_ms)).await;
    }
    if let Err(error) = stream.write_all(&response.body).await {
        if error.kind() == ErrorKind::BrokenPipe {
            return;
        }
        panic!("write response body: {error}");
    }
}

pub(super) async fn run_test_server(
    response: TestResponse,
    on_request: Arc<dyn Fn(RequestCapture) + Send + Sync>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test HTTP listener");
    let addr = listener.local_addr().expect("read listener addr");
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept test HTTP connection");
        let request = read_request(&mut stream).await;
        on_request(request);
        write_response(&mut stream, response).await;
    });

    (format!("http://{addr}"), handle)
}

pub(super) async fn run_chunked_test_server(
    status: u16,
    headers: Vec<(String, String)>,
    chunks: Vec<Vec<u8>>,
    on_request: Arc<dyn Fn(RequestCapture) + Send + Sync>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind chunked test HTTP listener");
    let addr = listener.local_addr().expect("read listener addr");
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept chunked test HTTP connection");
        let request = read_request(&mut stream).await;
        on_request(request);

        let mut raw_headers = String::new();
        raw_headers.push_str(&format!("HTTP/1.1 {status} OK\r\n"));
        raw_headers.push_str("Transfer-Encoding: chunked\r\n");
        for (name, value) in headers {
            raw_headers.push_str(&format!("{name}: {value}\r\n"));
        }
        raw_headers.push_str("Connection: close\r\n\r\n");
        stream
            .write_all(raw_headers.as_bytes())
            .await
            .expect("write chunked response headers");
        for chunk in chunks {
            stream
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await
                .expect("write chunk size");
            stream.write_all(&chunk).await.expect("write chunk body");
            stream
                .write_all(b"\r\n")
                .await
                .expect("write chunk newline");
            sleep(Duration::from_millis(5)).await;
        }
        stream
            .write_all(b"0\r\n\r\n")
            .await
            .expect("write final chunk");
    });

    (format!("http://{addr}"), handle)
}

pub(super) fn request_input(
    method: &str,
    url: &str,
    body: Value,
    timeout_ms: Option<u64>,
) -> Value {
    json!({
        "method": method,
        "url": url,
        "headers": [],
        "body": body,
        "timeoutMs": timeout_ms,
    })
}

pub(super) fn bytes_body(bytes: &[u8]) -> Value {
    bytes_value(bytes)
}

pub(super) fn empty_body() -> Value {
    bytes_body(b"")
}

pub(super) fn output_body_bytes(output: &Value) -> Vec<u8> {
    output
        .get("body")
        .and_then(bytes_payload)
        .expect("output body should carry bytes")
}

pub(super) fn event_tag(event: &Value) -> Option<&str> {
    event.get("tag").and_then(Value::as_str)
}

pub(super) fn event_bytes(event: &Value) -> Vec<u8> {
    event
        .get("value")
        .and_then(bytes_payload)
        .expect("event should carry bytes")
}
