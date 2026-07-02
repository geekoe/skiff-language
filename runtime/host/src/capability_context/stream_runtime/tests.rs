use std::{future::Future, pin::Pin};

use serde_json::{json, Value};
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, StreamPoll, StreamPullSource, StreamRuntimeError,
    StreamRuntimeResult,
};
use skiff_runtime_model::error::WirePayload;

use super::StreamRuntime;

#[tokio::test]
async fn stream_runtime_reads_items_and_normal_end_in_order() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();

    tokio::spawn(async move {
        sink.send(json!(1)).await.unwrap();
        sink.send(json!(2)).await.unwrap();
        sink.end().await;
    });

    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::Item(value) if value == json!(1)
    ));
    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::Item(value) if value == json!(2)
    ));
    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::End
    ));
}

#[tokio::test]
async fn stream_runtime_marks_cancel_on_early_break() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    let cancel_flag = sink.cancel_flag();

    sink.send(json!("first")).await.unwrap();
    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::Item(value) if value == json!("first")
    ));
    runtime.cancel(&stream);

    tokio::task::yield_now().await;
    assert!(cancel_flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn stream_sink_identity_matches_clones_only() {
    let runtime = StreamRuntime::default();
    let (_first_stream, first_sink) = runtime.channel_stream();
    let first_clone = first_sink.clone();
    let (_second_stream, second_sink) = runtime.channel_stream();

    assert!(first_sink.is_same_stream(&first_clone));
    assert!(!first_sink.is_same_stream(&second_sink));
}

#[tokio::test]
async fn stream_sink_send_blocked_by_backpressure_returns_on_cancel() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();

    sink.send(json!("buffered")).await.unwrap();
    let pending_send = tokio::spawn({
        let sink = sink.clone();
        async move { sink.send(json!("blocked")).await }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), async {
            while !pending_send.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "second send should be blocked by stream backpressure"
    );

    runtime.cancel(&stream);

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending_send)
        .await
        .expect("cancel should wake blocked send")
        .expect("send task should not panic")
        .unwrap_err();
    assert!(matches!(error, StreamRuntimeError::Cancelled));
}

#[tokio::test]
async fn stream_sink_send_blocked_by_backpressure_returns_on_frame_cancel() {
    let runtime = StreamRuntime::default();
    let (_stream, sink) = runtime.channel_stream();
    let frame_cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    sink.send(json!("buffered")).await.unwrap();
    let pending_send = tokio::spawn({
        let sink = sink.clone();
        let frame_cancelled = frame_cancelled.clone();
        async move {
            sink.send_with_cancel(json!("blocked"), &[frame_cancelled])
                .await
        }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), async {
            while !pending_send.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "second send should be blocked by stream backpressure"
    );

    frame_cancelled.store(true, std::sync::atomic::Ordering::SeqCst);

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending_send)
        .await
        .expect("frame cancel should wake blocked send")
        .expect("send task should not panic")
        .unwrap_err();
    assert!(matches!(error, StreamRuntimeError::Cancelled));
}

#[tokio::test]
async fn stream_runtime_next_with_outer_cancel_cancels_inner_stream() {
    let runtime = StreamRuntime::default();
    let (inner_stream, inner_sink) = runtime.channel_stream();
    let inner_cancel_flag = inner_sink.cancel_flag();
    let (_outer_stream, outer_sink) = runtime.channel_stream();

    let pending_next = tokio::spawn({
        let runtime = runtime.clone();
        let outer_signal = outer_sink.cancel_signal();
        async move {
            runtime
                .next_with_cancel(&inner_stream, &[outer_signal], &[])
                .await
        }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), async {
            while !pending_next.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "inner next should wait for producer"
    );

    outer_sink
        .cancelled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    outer_sink.cancel_notify.notify_waiters();

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending_next)
        .await
        .expect("outer cancel should wake inner next")
        .expect("next task should not panic")
        .unwrap_err();
    assert!(matches!(error, StreamRuntimeError::Cancelled));
    assert!(inner_cancel_flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn stream_runtime_next_with_cancellation_token_cancels_inner_stream() {
    let runtime = StreamRuntime::default();
    let (inner_stream, inner_sink) = runtime.channel_stream();
    let inner_cancel_flag = inner_sink.cancel_flag();
    let token = CancellationToken::new();

    let pending_next = tokio::spawn({
        let runtime = runtime.clone();
        let token = token.clone();
        async move {
            let cancellation = CancellationSignals::from_tokens([token]);
            runtime
                .next_with_cancellation(&inner_stream, &[], &cancellation)
                .await
        }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), async {
            while !pending_next.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "inner next should wait for producer"
    );

    token.cancel();

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending_next)
        .await
        .expect("token cancel should wake inner next")
        .expect("next task should not panic")
        .unwrap_err();
    assert!(matches!(error, StreamRuntimeError::Cancelled));
    assert!(inner_cancel_flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn stream_runtime_next_with_cancellation_token_flag_store_cancels_inner_stream() {
    let runtime = StreamRuntime::default();
    let (inner_stream, inner_sink) = runtime.channel_stream();
    let inner_cancel_flag = inner_sink.cancel_flag();
    let token = CancellationToken::new();
    let token_flag = token.cancel_flag();

    let pending_next = tokio::spawn({
        let runtime = runtime.clone();
        let token = token.clone();
        async move {
            let cancellation = CancellationSignals::from_tokens([token]);
            runtime
                .next_with_cancellation(&inner_stream, &[], &cancellation)
                .await
        }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), async {
            while !pending_next.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "inner next should wait for producer"
    );

    token_flag.store(true, std::sync::atomic::Ordering::SeqCst);

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending_next)
        .await
        .expect("token flag store should wake inner next")
        .expect("next task should not panic")
        .unwrap_err();
    assert!(matches!(error, StreamRuntimeError::Cancelled));
    assert!(inner_cancel_flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn stream_runtime_pull_stream_token_cancel_wakes_pending_next() {
    let runtime = StreamRuntime::default();
    let token = CancellationToken::new();
    let stream = runtime.pull_stream_with_cancellation(PendingPullSource, token.clone());

    let pending_next = tokio::spawn({
        let runtime = runtime.clone();
        async move { runtime.next(&stream).await }
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), async {
            while !pending_next.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_err(),
        "pull next should wait for pending source"
    );

    token.cancel();

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), pending_next)
        .await
        .expect("token cancel should wake pending pull next")
        .expect("next task should not panic")
        .unwrap_err();
    assert!(matches!(error, StreamRuntimeError::Cancelled));
}

#[tokio::test]
async fn stream_runtime_outer_cancel_stops_next_read() {
    let runtime = StreamRuntime::default();
    let (stream, _sink) = runtime.channel_stream();

    runtime.cancel(&stream);

    let error = runtime.next(&stream).await.unwrap_err();
    assert!(matches!(error, StreamRuntimeError::Cancelled));
}

#[tokio::test]
async fn stream_runtime_maps_producer_error_to_consumer_error() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();

    tokio::spawn(async move {
        sink.fail(StreamRuntimeError::decode("producer failed"))
            .await;
    });

    let error = runtime.next(&stream).await.unwrap_err();
    assert!(error.to_string().contains("producer failed"));
}

#[test]
fn stream_runtime_error_root_fold_boxes_eval_producer_error_and_preserves_payload() {
    let stream_error =
        StreamRuntimeError::producer(skiff_runtime_eval::error::RuntimeError::Cancelled);
    let expected_payload = stream_error.payload();
    let error = crate::error::RuntimeError::from(stream_error);

    assert!(matches!(error, crate::error::RuntimeError::Opaque(_)));
    assert_eq!(expected_payload.code, "CancelError");
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(
        WirePayload::catch_projection(&error),
        Some((
            skiff_runtime_model::error::TypeIdentity::builtin("CancelError"),
            json!({
                "message": "request was cancelled",
            }),
        ))
    );
}

#[test]
fn stream_runtime_error_eval_fold_preserves_root_producer_wire_payload() {
    let error = StreamRuntimeError::producer(crate::error::RuntimeError::cancelled());
    let expected_payload = error.payload();
    let expected_catch_projection = error.catch_projection();
    let error = skiff_runtime_eval::error::RuntimeError::from(error);

    assert!(matches!(
        error,
        skiff_runtime_eval::error::RuntimeError::Opaque(_)
    ));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
}

struct PendingPullSource;

impl StreamPullSource for PendingPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
}
