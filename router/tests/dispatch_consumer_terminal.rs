//! E-dispatch composition terminal delivery tests (C-dispatch §7.5).
//!
//! A Runtime session close (disconnect / replacement / shutdown barrier)
//! must deliver every `RequestDispatcher` terminal to the awaiting HTTP
//! phase through the composition `PendingHttpRouter` immediately — the HTTP
//! phase must never be left waiting for its own deadline after the
//! dispatcher already released the permit.

mod dispatch_harness;

use std::sync::Arc;
use std::time::Duration;

use skiff_router::dispatch::{
    RequestDispatcher, RequestOutcome, RuntimeDispatcherOptions, SubmitResult, TerminalSource,
};
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::SessionConsumer;
use skiff_router::supervisor::http::{HttpDispatchEvent, PendingHttpRouter};
use skiff_router::supervisor::session_ports::{DispatcherSessionConsumer, PendingHttpHandle};

use dispatch_harness::{
    corpus_epoch, request, session_state, FakeActorMethodSpawnControl, FakeCandidateViewSource,
    FakeEpochSource, FakeLeaseRevalidate, FakeRuntimePeer, FakeSessionAbort,
};

struct Rig {
    dispatcher: Arc<RequestDispatcher>,
    session: RuntimeSessionEpoch,
    router: Arc<PendingHttpRouter>,
    consumer: DispatcherSessionConsumer,
}

impl Rig {
    fn new() -> Self {
        let sessions = vec![session_state("s1", "runtime-a", 1)];
        let session = sessions[0].epoch.clone();
        let options = RuntimeDispatcherOptions::new(
            4,
            Arc::new(FakeEpochSource {
                epoch: Some(corpus_epoch()),
            }),
            Arc::new(FakeCandidateViewSource::new(sessions)),
            Arc::new(FakeLeaseRevalidate::new()),
            Arc::new(FakeRuntimePeer::new()),
            Arc::new(FakeSessionAbort::new()),
            Arc::new(FakeActorMethodSpawnControl::new()),
        )
        .expect("dispatcher options");
        let dispatcher = Arc::new(RequestDispatcher::new(options).expect("dispatcher"));
        let router = Arc::new(PendingHttpRouter::new());
        let handle = PendingHttpHandle::new();
        handle.set(Arc::clone(&router));
        let consumer = DispatcherSessionConsumer::new(Arc::clone(&dispatcher), handle);
        Self {
            dispatcher,
            session,
            router,
            consumer,
        }
    }

    fn accept(&self, request_id: &str) {
        match self.dispatcher.submit(request(request_id, "unary")) {
            SubmitResult::Accepted { .. } => {}
            other => panic!("expected accept for {request_id}, got {other:?}"),
        }
    }

    fn assert_zero(&self, expected_releases: u64) {
        let health = self.dispatcher.health();
        assert_eq!(health.pending.unary, 0);
        assert_eq!(health.admission.permits_held, 0);
        assert_eq!(health.admission.releases, expected_releases);
        assert_eq!(self.dispatcher.pending_count(), 0);
        assert!(self.dispatcher.permit_ledger().per_session.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_disconnect_delivers_terminal_to_http_phase_immediately() {
        let rig = Rig::new();
        rig.accept("req-disconnect");
        let mut rx = rig
            .router
            .register("req-disconnect")
            .expect("register correlation");

        rig.consumer.on_session_closed(&rig.session).expect("close");

        let event = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("terminal must arrive without waiting for the HTTP deadline");
        match event {
            Some(HttpDispatchEvent::Terminal { terminal }) => {
                assert_eq!(terminal.source, TerminalSource::RuntimeDisconnect);
                assert_eq!(terminal.outcome, RequestOutcome::Cancelled);
                assert!(
                    terminal.cancel_frame.is_none(),
                    "runtime disconnect must not emit a cancel frame"
                );
            }
            other => panic!("expected terminal event, got {other:?}"),
        }
        rig.assert_zero(1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_close_delivers_every_pending_terminal() {
        let rig = Rig::new();
        rig.accept("req-a");
        rig.accept("req-b");
        let mut rx_a = rig.router.register("req-a").expect("register a");
        let mut rx_b = rig.router.register("req-b").expect("register b");

        rig.consumer.on_session_closed(&rig.session).expect("close");

        let terminal_a = tokio::time::timeout(Duration::from_millis(500), rx_a.recv())
            .await
            .expect("terminal a must arrive")
            .expect("terminal a event");
        let terminal_b = tokio::time::timeout(Duration::from_millis(500), rx_b.recv())
            .await
            .expect("terminal b must arrive")
            .expect("terminal b event");
        for event in [terminal_a, terminal_b] {
            match event {
                HttpDispatchEvent::Terminal { terminal } => {
                    assert_eq!(terminal.source, TerminalSource::RuntimeDisconnect);
                }
                other => panic!("expected terminal event, got {other:?}"),
            }
        }
        rig.assert_zero(2);
    }

    #[test]
    fn session_close_without_http_phase_does_not_panic_and_releases_permits() {
        let rig = Rig::new();
        rig.accept("req-orphan");

        // No `PendingHttpRouter` registration: the HTTP phase is already
        // gone; the close must still succeed and the permit must be
        // released exactly once.
        rig.consumer
            .on_session_closed(&rig.session)
            .expect("close must not fail");
        rig.assert_zero(1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_barrier_close_delivers_pending_terminal_to_http_phase() {
        // C-process-lifecycle S6: `SessionLayer::shutdown` closes every
        // session through the same consumer terminal path; the HTTP phase
        // must settle from the close barrier, not its own deadline.
        let rig = Rig::new();
        rig.accept("req-shutdown");
        let mut rx = rig
            .router
            .register("req-shutdown")
            .expect("register correlation");

        rig.consumer.on_session_closed(&rig.session).expect("close");

        let event = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("shutdown terminal must arrive immediately");
        assert!(
            matches!(
                event,
                Some(HttpDispatchEvent::Terminal { ref terminal })
                    if terminal.source == TerminalSource::RuntimeDisconnect
            ),
            "expected runtime_disconnect terminal, got {event:?}"
        );
        rig.assert_zero(1);
    }
}
