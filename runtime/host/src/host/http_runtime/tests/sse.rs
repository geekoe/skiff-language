use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use crate::{
    capability_context::HttpRuntimeOptions,
    config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    host::http_runtime::{sse::open_sse_inner, stream::collect_events},
};

use super::helpers::{
    empty_body, event_bytes, event_tag, read_request, request_input, run_chunked_test_server,
};

#[tokio::test]
async fn sse_decodes_multiline_data_comments_id_event_and_done() {
    let smile = "🙂".as_bytes();
    let chunks = vec![
        b": ignored\nid: 42\nevent: message\ndata: hello\n".to_vec(),
        b"data: ".to_vec(),
        smile[..2].to_vec(),
        smile[2..].to_vec(),
        b"\n\n".to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ];
    let (url, handle) = run_chunked_test_server(
        200,
        vec![("Content-Type".to_string(), "text/event-stream".to_string())],
        chunks,
        Arc::new(|_| {}),
    )
    .await;

    let input = request_input("GET", &url, empty_body(), None);
    let events = collect_events(
        open_sse_inner(
            &input,
            None,
            None,
            DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
        )
        .await
        .expect("sse should open"),
    )
    .await
    .expect("sse should read");

    assert_eq!(event_tag(&events[0]), Some("response"));
    assert_eq!(event_tag(&events[1]), Some("event"));
    assert_eq!(events[1].get("id").and_then(Value::as_str), Some("42"));
    assert_eq!(
        events[1].get("event").and_then(Value::as_str),
        Some("message")
    );
    assert_eq!(
        events[1].get("data").and_then(Value::as_str),
        Some("hello\n🙂")
    );
    assert_eq!(event_tag(&events[2]), Some("event"));
    assert_eq!(
        events[2].get("data").and_then(Value::as_str),
        Some("[DONE]")
    );

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn sse_non_2xx_emits_raw_body_chunks() {
    let chunks = vec![br#"{"error":"#.to_vec(), br#""bad"}"#.to_vec()];
    let (url, handle) = run_chunked_test_server(
        400,
        vec![("Content-Type".to_string(), "application/json".to_string())],
        chunks,
        Arc::new(|_| {}),
    )
    .await;

    let input = request_input("POST", &url, empty_body(), None);
    let events = collect_events(
        open_sse_inner(
            &input,
            None,
            None,
            DEFAULT_HTTP_RESPONSE_MAX_BYTES,
            HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
        )
        .await
        .expect("sse should open"),
    )
    .await
    .expect("sse should read");

    assert_eq!(event_tag(&events[0]), Some("response"));
    assert_eq!(events[0].get("status").and_then(Value::as_u64), Some(400));
    assert_eq!(event_tag(&events[1]), Some("body"));
    assert_eq!(event_bytes(&events[1]), br#"{"error":"#.to_vec());
    assert_eq!(event_tag(&events[2]), Some("body"));
    assert_eq!(event_bytes(&events[2]), br#""bad"}"#.to_vec());

    handle.await.expect("server should complete");
}

#[tokio::test]
async fn sse_cancel_while_waiting_for_body_closes_connection_early() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind sse cancel listener");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept sse cancel");
        let _request = read_request(&mut stream).await;
        stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("write sse headers");

        let mut buffer = [0u8; 1];
        loop {
            match stream.read(&mut buffer).await {
                Ok(0) | Err(_) => {
                    let _ = closed_tx.send(());
                    break;
                }
                Ok(_) => {}
            }
        }
    });

    let cancelled = Arc::new(AtomicBool::new(false));
    let input = request_input("GET", &url, empty_body(), None);
    let mut stream = open_sse_inner(
        &input,
        None,
        Some(cancelled.as_ref()),
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        HttpRuntimeOptions::allowing_unsafe_targets_for_tests(),
    )
    .await
    .expect("sse should open");
    let response_event = stream
        .next_event()
        .await
        .expect("response event should read")
        .expect("response event should exist");
    assert_eq!(event_tag(&response_event), Some("response"));

    cancelled.store(true, Ordering::SeqCst);
    let error = stream
        .next_event()
        .await
        .expect_err("cancel should stop pending sse read");
    assert!(error.is_request_cancelled());
    drop(stream);

    tokio::time::timeout(Duration::from_secs(1), closed_rx)
        .await
        .expect("server should observe early close")
        .expect("close signal");
    handle.await.expect("server should complete");
}
