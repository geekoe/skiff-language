use std::sync::Arc;

use crate::{
    capability_context::TARGET_STD_HTTP_REQUEST, config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    error::RuntimeError,
};

use super::helpers::{
    empty_body, request_allowing_unsafe_targets, request_input, run_test_server, TestResponse,
};

#[tokio::test]
async fn request_rejects_oversized_response_bodies() {
    let response = TestResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
        body: vec![b'a'; DEFAULT_HTTP_RESPONSE_MAX_BYTES + 1],
        delay_ms: None,
        body_delay_ms: None,
    };

    let (url, handle) = run_test_server(response, Arc::new(|_| {})).await;

    let input = request_input("GET", &url, empty_body(), None);
    let error = request_allowing_unsafe_targets(&input, None, None)
        .await
        .expect_err("oversized body should fail");
    match error {
        RuntimeError::Protocol { target, message } => {
            assert_eq!(target, TARGET_STD_HTTP_REQUEST);
            assert!(message.contains("exceeds max size"));
        }
        other => panic!("expected ProtocolError, got {other:?}"),
    }

    handle.await.expect("server should complete");
}
