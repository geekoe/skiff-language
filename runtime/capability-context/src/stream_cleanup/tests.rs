use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use serde_json::json;
use tokio::sync::oneshot;

use super::*;

fn cancel_counter() -> (Arc<AtomicUsize>, impl Fn(&Value) + Send + Sync + 'static) {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    (count, move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
    })
}

#[test]
fn stream_cleanup_standalone_non_end_hard_cancels_once() {
    let stream = json!({"$stream": "standalone"});
    let (count, cancel) = cancel_counter();
    drop(StreamConsumerCleanup::from_cancel(&stream, cancel));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn stream_cleanup_standalone_natural_end_does_not_cancel() {
    let stream = json!({"$stream": "standalone-end"});
    let (count, cancel) = cancel_counter();
    let mut cleanup = StreamConsumerCleanup::from_cancel(&stream, cancel);
    cleanup.reached_end();
    drop(cleanup);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stream_cleanup_supervised_consumer_failure_waits_for_outer_owner() {
    let stream = json!({"$stream": "barrier"});
    let (count, cancel) = cancel_counter();
    let lease = SupervisedStreamConsumptionLease::from_cancel(&stream, cancel);
    let child = lease.child();
    let (consumer_failed_tx, consumer_failed_rx) = oneshot::channel();
    let (producer_release_tx, producer_release_rx) = oneshot::channel();

    let producer = tokio::spawn(async move {
        consumer_failed_rx.await.expect("consumer barrier");
        producer_release_rx.await.expect("producer release barrier");
        child.observe_producer_error(&stream)
    });

    drop(
        lease
            .child()
            .consumer_cleanup(&json!({"$stream": "barrier"})),
    );
    consumer_failed_tx
        .send(())
        .expect("signal consumer failure");
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(lease.status().cleanup_requested());

    producer_release_tx.send(()).expect("release producer");
    assert!(producer.await.expect("producer task"));
    assert_eq!(
        lease.status().terminal(),
        StreamConsumptionTerminal::ProducerErrorObserved
    );
    lease.complete_terminal();
    drop(lease);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn stream_cleanup_supervised_natural_end_finalizes_without_cancel() {
    let stream = json!({"$stream": "supervised-end"});
    let (count, cancel) = cancel_counter();
    let lease = SupervisedStreamConsumptionLease::from_cancel(&stream, cancel);
    let mut cleanup = lease.child().consumer_cleanup(&stream);
    cleanup.reached_end();
    drop(cleanup);
    lease.complete_success();
    assert!(lease.status().finalized());
    drop(lease);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn stream_cleanup_supervised_partial_success_cancels_once() {
    let stream = json!({"$stream": "supervised-partial-success"});
    let (count, cancel) = cancel_counter();
    let lease = SupervisedStreamConsumptionLease::from_cancel(&stream, cancel);
    drop(lease.child().consumer_cleanup(&stream));
    lease.complete_success();
    drop(lease);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn stream_cleanup_supervised_commit_error_after_end_cancels_once() {
    let stream = json!({"$stream": "supervised-commit-error"});
    let (count, cancel) = cancel_counter();
    let lease = SupervisedStreamConsumptionLease::from_cancel(&stream, cancel);
    let cleanup = lease.child().consumer_cleanup(&stream);
    cleanup.end_marker().mark_reached_end();
    drop(cleanup);

    assert_eq!(
        lease.status().terminal(),
        StreamConsumptionTerminal::EndObserved
    );
    assert!(lease.status().cleanup_requested());
    lease.complete_terminal();
    lease.hard_cancel();
    drop(lease);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn stream_cleanup_supervised_drop_hard_cancels_exactly_once() {
    let stream = json!({"$stream": "supervised-drop"});
    let (count, cancel) = cancel_counter();
    let lease = SupervisedStreamConsumptionLease::from_cancel(&stream, cancel);
    drop(lease.child().consumer_cleanup(&stream));
    drop(lease);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn stream_cleanup_wrong_stream_fails_closed_without_marking_terminal() {
    let stream = json!({"$stream": "expected"});
    let wrong_stream = json!({"$stream": "wrong"});
    let (count, cancel) = cancel_counter();
    let lease = SupervisedStreamConsumptionLease::from_cancel(&stream, cancel);
    let child = lease.child();
    drop(child.consumer_cleanup(&wrong_stream));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(lease.status().terminal(), StreamConsumptionTerminal::Open);
    assert!(lease.status().stream_mismatch());
    lease.hard_cancel();
    drop(lease);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}
