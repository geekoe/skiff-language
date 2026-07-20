use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use crate::StreamRuntime;

type StreamCancel = Arc<dyn Fn(&Value) + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StreamConsumptionTerminal {
    #[default]
    Open,
    EndObserved,
    ProducerErrorObserved,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamConsumptionStatus {
    terminal: StreamConsumptionTerminal,
    cleanup_requested: bool,
    stream_mismatch: bool,
    finalized: bool,
}

impl StreamConsumptionStatus {
    pub fn terminal(self) -> StreamConsumptionTerminal {
        self.terminal
    }

    pub fn cleanup_requested(self) -> bool {
        self.cleanup_requested
    }

    pub fn stream_mismatch(self) -> bool {
        self.stream_mismatch
    }

    pub fn finalized(self) -> bool {
        self.finalized
    }
}

#[derive(Debug, Default)]
struct StreamConsumptionState {
    status: Mutex<StreamConsumptionStatus>,
}

impl StreamConsumptionState {
    fn lock_status(&self) -> MutexGuard<'_, StreamConsumptionStatus> {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn status(&self) -> StreamConsumptionStatus {
        *self.lock_status()
    }

    fn observe_end(&self) {
        let mut status = self.lock_status();
        if !status.finalized && status.terminal == StreamConsumptionTerminal::Open {
            status.terminal = StreamConsumptionTerminal::EndObserved;
        }
    }

    fn observe_producer_error(&self) {
        let mut status = self.lock_status();
        if !status.finalized {
            status.terminal = StreamConsumptionTerminal::ProducerErrorObserved;
        }
    }

    fn request_cleanup(&self) {
        let mut status = self.lock_status();
        if !status.finalized {
            status.cleanup_requested = true;
        }
    }

    fn observe_stream_mismatch(&self) {
        let mut status = self.lock_status();
        if !status.finalized {
            status.stream_mismatch = true;
        }
    }

    fn claim_finalization(&self) -> Option<StreamConsumptionStatus> {
        let mut status = self.lock_status();
        if status.finalized {
            return None;
        }
        let claimed = *status;
        status.finalized = true;
        Some(claimed)
    }
}

#[derive(Clone, Debug, Default)]
pub struct StreamConsumerEndMarker {
    state: Arc<StreamConsumptionState>,
}

impl StreamConsumerEndMarker {
    pub fn mark_reached_end(&self) {
        self.state.observe_end();
    }

    pub fn has_reached_end(&self) -> bool {
        self.state.status().terminal == StreamConsumptionTerminal::EndObserved
    }
}

enum StreamConsumerCleanupOwner {
    Standalone(StreamCancel),
    Supervised(Arc<StreamConsumptionState>),
}

/// Owns the cancellation obligation after a consumer has obtained a stream.
/// Only a completed operation that observed the runtime's natural `End`
/// transition may disarm it. A standalone guard hard-cancels every other exit;
/// a supervised guard hands that obligation back to its outer lease.
pub struct StreamConsumerCleanup {
    owner: StreamConsumerCleanupOwner,
    stream_value: Value,
    end_marker: StreamConsumerEndMarker,
    disarmed: bool,
}

impl StreamConsumerCleanup {
    pub fn new(runtime: StreamRuntime, stream_value: &Value) -> Self {
        Self::from_cancel(stream_value, move |value| runtime.cancel(value))
    }

    pub fn from_cancel(
        stream_value: &Value,
        cancel: impl Fn(&Value) + Send + Sync + 'static,
    ) -> Self {
        Self::standalone(stream_value, Arc::new(cancel))
    }

    fn standalone(stream_value: &Value, cancel: StreamCancel) -> Self {
        Self {
            owner: StreamConsumerCleanupOwner::Standalone(cancel),
            stream_value: stream_value.clone(),
            end_marker: StreamConsumerEndMarker::default(),
            disarmed: false,
        }
    }

    fn supervised(stream_value: &Value, state: Arc<StreamConsumptionState>) -> Self {
        Self {
            owner: StreamConsumerCleanupOwner::Supervised(state.clone()),
            stream_value: stream_value.clone(),
            end_marker: StreamConsumerEndMarker { state },
            disarmed: false,
        }
    }

    pub fn end_marker(&self) -> StreamConsumerEndMarker {
        self.end_marker.clone()
    }

    pub fn reached_end(&mut self) {
        self.end_marker.mark_reached_end();
        self.disarm_after_end();
    }

    pub fn disarm_after_end(&mut self) {
        debug_assert!(
            self.end_marker.has_reached_end(),
            "stream cleanup can only disarm after natural End"
        );
        if self.end_marker.has_reached_end() {
            self.disarmed = true;
        }
    }
}

impl Drop for StreamConsumerCleanup {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        match &self.owner {
            StreamConsumerCleanupOwner::Standalone(cancel) => cancel(&self.stream_value),
            StreamConsumerCleanupOwner::Supervised(state) => state.request_cleanup(),
        }
    }
}

/// The unique outer owner for a prepared producer/consumer composite operation.
/// Child consumers may report typed terminal observations, but only this lease
/// may finalize or hard-cancel the stream.
pub struct SupervisedStreamConsumptionLease {
    stream_value: Value,
    cancel: StreamCancel,
    state: Arc<StreamConsumptionState>,
}

impl SupervisedStreamConsumptionLease {
    pub fn new(runtime: StreamRuntime, stream_value: &Value) -> Self {
        Self::from_cancel(stream_value, move |value| runtime.cancel(value))
    }

    pub fn from_cancel(
        stream_value: &Value,
        cancel: impl Fn(&Value) + Send + Sync + 'static,
    ) -> Self {
        Self {
            stream_value: stream_value.clone(),
            cancel: Arc::new(cancel),
            state: Arc::new(StreamConsumptionState::default()),
        }
    }

    pub fn child(&self) -> SupervisedStreamConsumptionChild {
        SupervisedStreamConsumptionChild {
            stream_value: self.stream_value.clone(),
            cancel: self.cancel.clone(),
            state: self.state.clone(),
        }
    }

    pub fn status(&self) -> StreamConsumptionStatus {
        self.state.status()
    }

    pub fn observe_end(&self) {
        self.state.observe_end();
    }

    pub fn observe_producer_error(&self) {
        self.state.observe_producer_error();
    }

    /// Finalizes a successful consumer. Natural End needs no extra cancel;
    /// a partial successful consumer releases the producer with a hard cancel.
    pub fn complete_success(&self) {
        self.finalize(|status| status.terminal != StreamConsumptionTerminal::EndObserved);
    }

    /// Finalizes after the outer owner has consumed a typed End or producer
    /// error. The concrete stream runtime already performed its terminal CAS.
    pub fn complete_terminal(&self) {
        // A failed child operation still requested cleanup even if it observed
        // End or producer Error first. Honor that obligation exactly once; the
        // concrete runtime's terminal CAS makes this callback an idempotent
        // release after the registry has already reached terminal.
        self.finalize(|status| {
            debug_assert_ne!(
                status.terminal,
                StreamConsumptionTerminal::Open,
                "terminal completion requires an observed terminal"
            );
            status.cleanup_requested
        });
    }

    pub fn hard_cancel(&self) {
        self.finalize(|_| true);
    }

    fn finalize(&self, should_cancel: impl FnOnce(StreamConsumptionStatus) -> bool) {
        if let Some(status) = self.state.claim_finalization() {
            if should_cancel(status) {
                (self.cancel)(&self.stream_value);
            }
        }
    }
}

impl Drop for SupervisedStreamConsumptionLease {
    fn drop(&mut self) {
        self.hard_cancel();
    }
}

#[derive(Clone)]
pub struct SupervisedStreamConsumptionChild {
    stream_value: Value,
    cancel: StreamCancel,
    state: Arc<StreamConsumptionState>,
}

impl SupervisedStreamConsumptionChild {
    pub fn consumer_cleanup(&self, stream_value: &Value) -> StreamConsumerCleanup {
        if self.is_expected_stream(stream_value) {
            StreamConsumerCleanup::supervised(stream_value, self.state.clone())
        } else {
            // A mismatched handle must fail closed and must not mutate the
            // prepared producer's terminal observation.
            self.state.observe_stream_mismatch();
            StreamConsumerCleanup::standalone(stream_value, self.cancel.clone())
        }
    }

    pub fn observe_end(&self, stream_value: &Value) -> bool {
        if !self.is_expected_stream(stream_value) {
            self.state.observe_stream_mismatch();
            return false;
        }
        self.state.observe_end();
        true
    }

    pub fn observe_producer_error(&self, stream_value: &Value) -> bool {
        if !self.is_expected_stream(stream_value) {
            self.state.observe_stream_mismatch();
            return false;
        }
        self.state.observe_producer_error();
        true
    }

    fn is_expected_stream(&self, stream_value: &Value) -> bool {
        self.stream_value == *stream_value
    }
}

#[cfg(test)]
mod tests {
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
}
