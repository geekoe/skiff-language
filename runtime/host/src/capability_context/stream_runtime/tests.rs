use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use serde_json::{json, Value};
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, OutboundRequestCancelSendError,
    OutboundRequestCancelSender, OutboundRequestLease, OutboundRequestRegistry, StreamInternalItem,
    StreamLifetimeGuard, StreamLifetimeGuardApi, StreamPoll, StreamPullSource, StreamRuntimeError,
    StreamRuntimeResult,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue},
};

use super::StreamRuntime;

#[tokio::test]
async fn stream_runtime_reads_items_and_normal_end_in_order() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    assert_eq!(runtime.active_stream_count(), 1);

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
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_preserves_internal_item_with_its_owned_heap() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    let mut item_heap = RequestHeap::default();
    let handle = item_heap
        .alloc_array(vec![RuntimeValue::String(
            "opaque-runtime-item".to_string(),
        )])
        .unwrap();
    let cancellation = CancellationSignals::none();

    sink.send_internal_with_stream_cancellation(
        StreamInternalItem::new(RuntimeValue::Heap(handle), item_heap),
        &[],
        &cancellation,
    )
    .await
    .unwrap();

    let StreamPoll::InternalItem(item) = runtime.next(&stream).await.unwrap() else {
        panic!("internal stream item must not be converted to the JSON carrier")
    };
    let (value, item_heap) = item.into_parts();
    let RuntimeValue::Heap(handle) = value else {
        panic!("internal item should retain its owned heap handle")
    };
    assert!(matches!(
        item_heap.get(handle).unwrap(),
        HeapNode::Array(items)
            if items == &[RuntimeValue::String("opaque-runtime-item".to_string())]
    ));
    runtime.cancel(&stream);
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
    assert_eq!(runtime.active_stream_count(), 0);
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
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_sink_terminal_publication_is_independent_of_item_backpressure() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    sink.send(json!("buffered")).await.unwrap();
    let pending_end = tokio::spawn({
        let sink = sink.clone();
        async move { sink.end().await }
    });

    tokio::time::timeout(std::time::Duration::from_millis(100), pending_end)
        .await
        .expect("terminal publication must not wait for item capacity")
        .expect("terminal publisher should not panic");
    runtime.cancel(&stream);
    assert_eq!(runtime.active_stream_count(), 0);
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
async fn stream_runtime_next_with_cancellation_token_removes_inner_stream() {
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
    assert_eq!(runtime.active_stream_count(), 0);
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
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_pull_stream_normal_end_does_not_cancel_request_token() {
    let runtime = StreamRuntime::default();
    let token = CancellationToken::new();
    let stream = runtime.pull_stream_with_cancellation(EndPullSource, token.clone());

    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::End
    ));

    assert!(!token.is_cancelled());
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_pull_source_error_finishes_once_and_cannot_be_polled_again() {
    let registry = OutboundRequestRegistry::default();
    let (response_sender, _response_rx) = tokio::sync::mpsc::unbounded_channel();
    let (cancel_sender, mut cancel_rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel_sender: OutboundRequestCancelSender = Arc::new(move |request_id, reason| {
        cancel_sender
            .send((request_id.to_string(), reason.to_string()))
            .map_err(|_| OutboundRequestCancelSendError::Closed)
    });
    let lease = registry
        .insert_with_lease(
            "pull-source-error".to_string(),
            response_sender,
            Some(cancel_sender),
            "stream_cancelled",
        )
        .expect("pull source lease should register");
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let runtime = StreamRuntime::default();
    let request_generation = 91;
    runtime.open_scope(request_generation);
    let stream = runtime.pull_stream_with_cancellation_in_scope(
        ErrorThenItemPullSource {
            _lease: lease,
            polls: Arc::clone(&polls),
            drops: Arc::clone(&drops),
        },
        CancellationToken::new(),
        request_generation,
    );
    assert_eq!(runtime.active_stream_count(), 1);
    assert_eq!(runtime.active_stream_count_in_scope(request_generation), 1);
    assert_eq!(registry.pending_count(), 1);
    assert_eq!(registry.active_lease_count(), 1);

    let error = runtime.next(&stream).await.unwrap_err();
    assert!(error.to_string().contains("pull source failed"));
    assert_eq!(runtime.active_stream_count(), 0);
    assert_eq!(runtime.active_stream_count_in_scope(request_generation), 0);
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let (request_id, reason) =
        tokio::time::timeout(std::time::Duration::from_millis(100), cancel_rx.recv())
            .await
            .expect("source error should drop and cancel its lease")
            .expect("cancel receiver should stay open");
    assert_eq!(request_id, "pull-source-error");
    assert_eq!(reason, "stream_cancelled");

    let second_error = runtime.next(&stream).await.unwrap_err();
    assert!(second_error.to_string().contains("unknown Stream value"));
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stream_runtime_pull_stream_explicit_cancel_cancels_source_token() {
    let runtime = StreamRuntime::default();
    let token = CancellationToken::new();
    let stream = runtime.pull_stream_with_cancellation(PendingPullSource, token.clone());

    runtime.cancel(&stream);

    assert!(token.is_cancelled());
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_outer_cancel_stops_next_read() {
    let runtime = StreamRuntime::default();
    let (stream, _sink) = runtime.channel_stream();
    assert_eq!(runtime.active_stream_count(), 1);

    runtime.cancel(&stream);
    runtime.cancel(&stream);

    let error = runtime.next(&stream).await.unwrap_err();
    assert!(error.to_string().contains("unknown Stream value"));
    assert_eq!(runtime.active_stream_count(), 0);
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
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_buffered_item_then_end_never_blocks_terminal_publication() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    sink.send(json!("accepted")).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_millis(100), sink.end())
        .await
        .expect("End publication must not wait for the capacity-one item buffer");
    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::Item(value) if value == json!("accepted")
    ));
    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::End
    ));
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_buffered_item_then_error_preserves_item_before_error() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    sink.send(json!("accepted")).await.unwrap();

    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        sink.fail(StreamRuntimeError::decode("after accepted item")),
    )
    .await
    .expect("Error publication must not wait for the capacity-one item buffer");
    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::Item(value) if value == json!("accepted")
    ));
    let error = runtime.next(&stream).await.unwrap_err();
    assert!(error.to_string().contains("after accepted item"));
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_request_scope_drop_cancels_producer_clone_while_root_stays_alive() {
    let runtime = StreamRuntime::default();
    let drops = Arc::new(AtomicUsize::new(0));
    let request_generation = 41;
    runtime.open_scope(request_generation);
    let (stream, sink) = runtime.channel_stream_with_lifetime_in_scope(
        request_generation,
        StreamLifetimeGuard::new(DropProbe(Arc::clone(&drops))),
    );
    let producer_runtime = runtime.clone();
    let cancel_signal = sink.cancel_signal();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let producer = tokio::spawn(async move {
        sink.send(json!("unconsumed")).await.unwrap();
        sink.end().await;
        ready_tx.send(()).unwrap();
        cancel_signal.wait_cancelled().await;
        producer_runtime.active_stream_count()
    });

    tokio::time::timeout(std::time::Duration::from_millis(100), ready_rx)
        .await
        .expect("producer must publish an item and terminal")
        .expect("producer readiness sender must stay open");
    runtime.close_scope(request_generation);
    runtime.close_scope(request_generation);

    assert_eq!(runtime.active_stream_count(), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_millis(100), producer)
            .await
            .expect("request owner drop must wake the producer clone")
            .expect("producer task must not panic"),
        0
    );
    let (other_stream, other_sink) = runtime.channel_stream();
    assert_eq!(
        runtime.active_stream_count(),
        1,
        "root runtime remains usable"
    );
    runtime.cancel(&other_stream);
    drop(other_sink);
    drop(stream);
}

#[tokio::test]
async fn stream_runtime_removes_entry_on_source_drop() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    assert_eq!(runtime.active_stream_count(), 1);

    drop(sink);

    assert!(matches!(
        runtime.next(&stream).await.unwrap(),
        StreamPoll::End
    ));
    assert_eq!(runtime.active_stream_count(), 0);
}

#[tokio::test]
async fn stream_runtime_repeated_terminal_is_idempotent() {
    let runtime = StreamRuntime::default();
    let (stream, sink) = runtime.channel_stream();
    let cancel_flag = sink.cancel_flag();

    runtime.cancel(&stream);
    runtime.cancel(&stream);

    assert!(cancel_flag.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(runtime.active_stream_count(), 0);
    assert!(sink.send(json!("after-terminal")).await.is_err());
}

#[tokio::test]
async fn stream_runtime_lifetime_guard_closes_exactly_once_on_every_terminal_path() {
    for terminal in ["end", "cancel", "source-drop"] {
        let runtime = StreamRuntime::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let (stream, sink) = runtime
            .channel_stream_with_lifetime(StreamLifetimeGuard::new(DropProbe(Arc::clone(&drops))));

        match terminal {
            "end" => {
                sink.end().await;
                assert!(matches!(
                    runtime.next(&stream).await.unwrap(),
                    StreamPoll::End
                ));
            }
            "cancel" => {
                runtime.cancel(&stream);
                runtime.cancel(&stream);
            }
            "source-drop" => {
                drop(sink);
                assert!(matches!(
                    runtime.next(&stream).await.unwrap(),
                    StreamPoll::End
                ));
            }
            _ => unreachable!(),
        }

        assert_eq!(drops.load(Ordering::SeqCst), 1, "terminal={terminal}");
        assert_eq!(runtime.active_stream_count(), 0);
    }
}

#[tokio::test]
async fn stream_runtime_cancel_signal_wakes_without_polling_race() {
    for _ in 0..100 {
        let runtime = StreamRuntime::default();
        let (stream, sink) = runtime.channel_stream();
        let signal = sink.cancel_signal();
        let waiter = tokio::spawn(async move { signal.wait_cancelled().await });

        runtime.cancel(&stream);

        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("stream cancellation should wake signal waiter")
            .expect("signal waiter should not panic");
        assert_eq!(runtime.active_stream_count(), 0);
    }
}

#[tokio::test]
async fn unconsumed_outbound_server_stream_cleans_up_on_request_owner_drop_with_runtime_clone() {
    let registry = OutboundRequestRegistry::default();
    let (response_sender, _response_rx) = tokio::sync::mpsc::unbounded_channel();
    let (cancel_sender, mut cancel_rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel_sender: OutboundRequestCancelSender =
        std::sync::Arc::new(move |request_id, reason| {
            cancel_sender
                .send((request_id.to_string(), reason.to_string()))
                .map_err(|_| OutboundRequestCancelSendError::Closed)
        });
    let lease = registry
        .insert_with_lease(
            "request-unconsumed-stream".to_string(),
            response_sender,
            Some(cancel_sender),
            "stream_cancelled",
        )
        .expect("outbound stream lease should register");

    let runtime = StreamRuntime::default();
    let request_generation = 73;
    runtime.open_scope(request_generation);
    let _stream = runtime.pull_stream_with_cancellation_in_scope(
        LeaseHoldingPullSource { _lease: lease },
        CancellationToken::new(),
        request_generation,
    );
    assert_eq!(runtime.active_stream_count(), 1);
    assert_eq!(registry.pending_count(), 1);
    assert_eq!(registry.active_lease_count(), 1);

    let producer_runtime_clone = runtime.clone();
    runtime.close_scope(request_generation);

    let (request_id, reason) =
        tokio::time::timeout(std::time::Duration::from_secs(1), cancel_rx.recv())
            .await
            .expect("request owner drop should cancel unconsumed stream")
            .expect("cancel receiver should stay open");
    assert_eq!(request_id, "request-unconsumed-stream");
    assert_eq!(reason, "stream_cancelled");
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(producer_runtime_clone.active_stream_count(), 0);

    let (root_stream, root_sink) = runtime.channel_stream();
    assert_eq!(runtime.active_stream_count(), 1, "root owner remains live");
    runtime.cancel(&root_stream);
    drop(root_sink);
}

#[test]
fn stream_runtime_error_root_fold_preserves_cancellation_as_terminal() {
    let error = crate::error::RuntimeError::from(StreamRuntimeError::Cancelled);

    assert!(matches!(error, crate::error::RuntimeError::Cancelled));
    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);
}

#[test]
fn stream_runtime_error_eval_fold_preserves_cancellation_as_terminal() {
    let error = skiff_runtime_eval::error::RuntimeError::from(StreamRuntimeError::Cancelled);

    assert!(matches!(
        error,
        skiff_runtime_eval::error::RuntimeError::Cancelled
    ));
    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);
}

struct PendingPullSource;

#[derive(Debug)]
struct DropProbe(Arc<AtomicUsize>);

impl StreamLifetimeGuardApi for DropProbe {}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl StreamPullSource for PendingPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
}

struct EndPullSource;

impl StreamPullSource for EndPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        Box::pin(async { Ok(None) })
    }
}

struct ErrorThenItemPullSource {
    _lease: OutboundRequestLease,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl StreamPullSource for ErrorThenItemPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        let poll = self.polls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if poll == 0 {
                Err(StreamRuntimeError::decode("pull source failed"))
            } else {
                Ok(Some(json!("must not be polled")))
            }
        })
    }
}

impl Drop for ErrorThenItemPullSource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct LeaseHoldingPullSource {
    _lease: OutboundRequestLease,
}

impl StreamPullSource for LeaseHoldingPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
}
