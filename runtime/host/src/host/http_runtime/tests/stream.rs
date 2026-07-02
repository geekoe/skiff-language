use std::{sync::Arc, time::Duration};

use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use crate::{
    capability_context::{HttpRuntimeOptions, TARGET_STD_HTTP_STREAM},
    config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    error::RuntimeError,
    host::http_runtime::stream::{collect_events, open_stream_inner},
};

use super::helpers::{
    empty_body, event_bytes, event_tag, read_request, request_input, run_chunked_test_server,
};

#[tokio::test]
async fn stream_emits_response_then_body_chunks() {
    let chunks = vec![b"hello ".to_vec(), b"world".to_vec()];
    let (url, handle) = run_chunked_test_server(
        200,
        vec![("Content-Type".to_string(), "text/plain".to_string())],
        chunks,
        Arc::new(|_| {}),
    )
    .await;

    let input = request_input("GET", &url, empty_body(), None);
    let events = collect_events(
        open_stream_inner(
            &input,
            None,
            None,
            DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
        )
        .await
        .expect("stream should open"),
    )
    .await
    .expect("stream should read");

    assert_eq!(event_tag(&events[0]), Some("response"));
    assert_eq!(events[0].get("status").and_then(Value::as_u64), Some(200));
    assert_eq!(event_tag(&events[1]), Some("chunk"));
    assert_eq!(event_bytes(&events[1]), b"hello ".to_vec());
    assert_eq!(event_tag(&events[2]), Some("chunk"));
    assert_eq!(event_bytes(&events[2]), b"world".to_vec());

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn stream_rejects_oversized_body_from_call_context_limit() {
    let chunks = vec![b"abcd".to_vec(), b"e".to_vec()];
    let (url, handle) = run_chunked_test_server(
        200,
        vec![("Content-Type".to_string(), "text/plain".to_string())],
        chunks,
        Arc::new(|_| {}),
    )
    .await;

    let input = request_input("GET", &url, empty_body(), None);
    let mut stream = open_stream_inner(
        &input,
        None,
        None,
        4,
        HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
    )
    .await
    .expect("stream should open");

    let response_event = stream
        .next_event()
        .await
        .expect("response event should read")
        .expect("response event should exist");
    assert_eq!(event_tag(&response_event), Some("response"));
    let first_chunk = stream
        .next_event()
        .await
        .expect("first chunk should read")
        .expect("first chunk should exist");
    assert_eq!(event_bytes(&first_chunk), b"abcd".to_vec());

    let error = stream
        .next_event()
        .await
        .expect_err("second chunk should exceed the shared call context limit");
    match error {
        RuntimeError::Protocol { target, message } => {
            assert_eq!(target, TARGET_STD_HTTP_STREAM);
            assert!(message.contains("exceeds max size"));
        }
        other => panic!("expected Protocol error, got {other:?}"),
    }

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn dropping_stream_closes_connection_early() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind early close listener");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept early close");
        let _request = read_request(&mut stream).await;
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write headers");
        stream
            .write_all(b"5\r\nhello\r\n")
            .await
            .expect("write first chunk");
        let mut buffer = [0u8; 1];
        loop {
            match stream.read(&mut buffer).await {
                Ok(0) => {
                    let _ = closed_tx.send(());
                    break;
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = closed_tx.send(());
                    break;
                }
            }
        }
    });

    let input = request_input("GET", &url, empty_body(), None);
    let mut stream = open_stream_inner(
        &input,
        None,
        None,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
    )
    .await
    .expect("stream should open");
    let response_event = stream
        .next_event()
        .await
        .expect("response event should read")
        .expect("response event should exist");
    assert_eq!(event_tag(&response_event), Some("response"));
    let chunk_event = stream
        .next_event()
        .await
        .expect("chunk event should read")
        .expect("chunk event should exist");
    assert_eq!(event_bytes(&chunk_event), b"hello".to_vec());
    drop(stream);

    tokio::time::timeout(Duration::from_secs(1), closed_rx)
        .await
        .expect("server should observe early close")
        .expect("close signal");
    handle.await.expect("server should complete");
}
