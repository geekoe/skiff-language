//! W-dispatch invariant tests beyond the frozen corpus: writer failures,
//! deadline rechecks, stale exact-fence, task authority, closed-session race
//! and pending/permit-to-zero assertions (C-dispatch §3/§4/§5/§7).

mod dispatch_harness;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use skiff_router::dispatch::*;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_runtime_transport::protocol::ValidatedResponseErrorFrame;

use dispatch_harness::{
    corpus_epoch, request, session_state, task_attempt, FakeCandidateViewSource, FakeEpochSource,
    FakeLeaseRevalidate, FakeRuntimePeer, FakeSessionAbort, SessionState,
};

struct Rig {
    dispatcher: RequestDispatcher,
    peer: FakeRuntimePeer,
    abort: FakeSessionAbort,
    candidate: FakeCandidateViewSource,
    revalidate: FakeLeaseRevalidate,
    session: RuntimeSessionEpoch,
}

impl Rig {
    fn new(max_concurrency: usize) -> Self {
        Self::with_sessions(
            max_concurrency,
            vec![session_state("s1", "runtime-a", 1)],
            None,
        )
    }

    fn with_sessions(
        max_concurrency: usize,
        sessions: Vec<SessionState>,
        timeout_check: Option<Arc<dyn TimeoutCheck>>,
    ) -> Self {
        let session = sessions[0].epoch.clone();
        let candidate = FakeCandidateViewSource::new(sessions);
        let peer = FakeRuntimePeer::new();
        let abort = FakeSessionAbort::new();
        let revalidate = FakeLeaseRevalidate::new();
        let mut options = RuntimeDispatcherOptions::new(
            max_concurrency,
            Arc::new(FakeEpochSource {
                epoch: Some(corpus_epoch()),
            }),
            Arc::new(candidate.clone()),
            Arc::new(revalidate.clone()),
            Arc::new(peer.clone()),
            Arc::new(abort.clone()),
        )
        .expect("options");
        if let Some(timeout_check) = timeout_check {
            options = options.with_timeout_check(timeout_check);
        }
        Self {
            dispatcher: RequestDispatcher::new(options).expect("dispatcher"),
            peer,
            abort,
            candidate,
            revalidate,
            session,
        }
    }

    fn accept(&self, request_id: &str, mode: &str) -> RuntimeSessionEpoch {
        match self.dispatcher.submit(request(request_id, mode)) {
            SubmitResult::Accepted { session_epoch, .. } => session_epoch,
            other => panic!("expected accept for {request_id}, got {other:?}"),
        }
    }

    fn reject(&self, request_id: &str, mode: &str) -> SubmitRejectReason {
        match self.dispatcher.submit(request(request_id, mode)) {
            SubmitResult::Rejected { reason, .. } => reason,
            other => panic!("expected reject for {request_id}, got {other:?}"),
        }
    }

    fn assert_zero(&self, expected_releases: u64) {
        let health = self.dispatcher.health();
        assert_eq!(health.admission.permits_held, 0);
        assert_eq!(health.admission.releases, expected_releases);
        assert_eq!(self.dispatcher.pending_count(), 0);
        assert!(self.dispatcher.permit_ledger().per_session.is_empty());
    }
}

fn error_frame(request_id: &str) -> RuntimeResponseFrame {
    RuntimeResponseFrame::Error {
        request_id: request_id.to_string(),
        error: ValidatedResponseErrorFrame::Control(
            skiff_runtime_transport::protocol::RuntimeErrorFramePayload {
                code: "boom".to_string(),
                message: "boom".to_string(),
                status: Some(503),
                details: None,
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_writer_start_failure_is_callback_error_releases_permit_and_aborts_session() {
        let rig = Rig::new(2);
        rig.peer.record.lock().unwrap().fail_start = true;

        let reason = rig.reject("req-1", "unary");
        assert_eq!(reason, SubmitRejectReason::CallbackError);

        let health = rig.dispatcher.health();
        assert_eq!(
            health
                .terminal
                .by_source
                .get(&TerminalSource::CallbackError),
            Some(&1)
        );
        assert_eq!(
            rig.peer.record.lock().unwrap().cancels,
            vec![("req-1".to_string(), "protocol_error".to_string())]
        );
        assert_eq!(
            rig.abort.record.lock().unwrap().sessions,
            vec![rig.session.clone()]
        );
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_cancel_write_failure_escalates_terminal_to_callback_error() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");
        rig.peer.record.lock().unwrap().fail_cancel = true;

        let terminal = rig.dispatcher.timeout("req-1").expect("pending");
        assert_eq!(terminal.outcome, RequestOutcome::Cancelled);
        assert_eq!(terminal.source, TerminalSource::CallbackError);
        assert_eq!(terminal.cancel_frame, None);
        assert_eq!(
            rig.abort.record.lock().unwrap().sessions,
            vec![rig.session.clone()]
        );
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_unknown_runtime_cancel_reason_is_protocol_error() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");

        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Cancel {
                request_id: "req-1".to_string(),
                reason: "not-a-wire-reason".to_string(),
            },
        );
        assert_eq!(outcome.terminals.len(), 1);
        let terminal = &outcome.terminals[0];
        assert_eq!(terminal.outcome, RequestOutcome::ProtocolError);
        assert_eq!(terminal.source, TerminalSource::ProtocolError);
        assert_eq!(
            terminal.cancel_frame,
            Some(CancelFrame {
                request_id: "req-1".to_string(),
                reason: "protocol_error".to_string(),
            })
        );
        rig.assert_zero(1);
    }

    #[derive(Debug, Default)]
    struct FlipTimeoutCheck {
        calls: AtomicUsize,
    }

    impl TimeoutCheck for FlipTimeoutCheck {
        fn is_expired(&self, _deadline: &RequestDeadline) -> bool {
            // First check (submit) passes, second check (enqueue) expires.
            self.calls.fetch_add(1, Ordering::SeqCst) >= 1
        }
    }

    #[test]
    fn dispatch_deadline_expired_before_enqueue_rejects_without_reserve() {
        let rig = Rig::new(2);
        let mut dispatch_request = request("req-1", "unary");
        dispatch_request.header.deadline = Some(
        skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestDeadlineFrameHeader {
            timeout_ms: 0,
            expires_at: "2026-08-02T00:00:00Z".to_string(),
        },
    );
        let reason = match rig.dispatcher.submit(dispatch_request) {
            SubmitResult::Rejected { reason, .. } => reason,
            other => panic!("expected reject, got {other:?}"),
        };
        assert_eq!(reason, SubmitRejectReason::DeadlineExpired);
        rig.assert_zero(0);
    }

    #[test]
    fn dispatch_deadline_expired_at_enqueue_releases_reserved_permit() {
        let rig = Rig::with_sessions(
            2,
            vec![session_state("s1", "runtime-a", 1)],
            Some(Arc::new(FlipTimeoutCheck::default())),
        );
        let mut dispatch_request = request("req-1", "unary");
        dispatch_request.header.deadline = Some(
        skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestDeadlineFrameHeader {
            timeout_ms: 1000,
            expires_at: "2026-08-02T00:00:00Z".to_string(),
        },
    );
        let reason = match rig.dispatcher.submit(dispatch_request) {
            SubmitResult::Rejected { reason, .. } => reason,
            other => panic!("expected reject, got {other:?}"),
        };
        assert_eq!(reason, SubmitRejectReason::DeadlineExpired);
        // The permit was reserved then released: one release, zero held.
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_task_attempt_is_admitted_and_tracked() {
        let rig = Rig::new(2);
        let result = rig.dispatcher.task_attempt_submit(task_attempt(
            "task-attempt-1",
            "task-1",
            "attempt-1",
            "lease-1",
        ));
        let TaskAttemptSubmitResult::Accepted {
            request_id,
            session_epoch,
        } = result
        else {
            panic!("task attempt must be accepted");
        };
        assert_eq!(request_id, "task-attempt-1");
        assert_eq!(session_epoch, rig.session);
        assert_eq!(rig.dispatcher.pending_count(), 1);
        assert_eq!(rig.dispatcher.health().admission.permits_held, 1);
        assert!(rig.dispatcher.is_task_attempt("task-attempt-1"));
        assert_eq!(rig.peer.record.lock().unwrap().attempts, vec!["task-attempt-1"]);
    }

    #[test]
    fn dispatch_closed_session_stale_lease_race_is_fail_closed() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");
        let terminals = rig.dispatcher.on_session_closed(&rig.session);
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].source, TerminalSource::RuntimeDisconnect);

        // The fake query still returns the stale lease; the dispatcher's closed
        // session observation must refuse it (race between directory cancellation
        // and enqueue).
        let reason = rig.reject("req-2", "unary");
        assert_eq!(reason, SubmitRejectReason::NoCandidate);
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_cancelled_session_is_excluded_by_candidate_query() {
        let rig = Rig::new(2);
        rig.candidate.mark_cancelled("s1");
        let reason = rig.reject("req-1", "unary");
        assert_eq!(reason, SubmitRejectReason::NoCandidate);
        rig.assert_zero(0);
    }

    #[test]
    fn dispatch_shutdown_terminates_pending_and_rejects_new_admission() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");

        let terminals = rig.dispatcher.shutdown();
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].outcome, RequestOutcome::Cancelled);
        assert_eq!(terminals[0].source, TerminalSource::RouterShutdown);
        assert_eq!(
            terminals[0].cancel_frame,
            Some(CancelFrame {
                request_id: "req-1".to_string(),
                reason: "router_shutdown".to_string(),
            })
        );

        let reason = rig.reject("req-2", "unary");
        assert_eq!(reason, SubmitRejectReason::Shutdown);
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_stream_protocol_violations_terminate_with_cancel() {
        // End before start.
        let rig = Rig::new(2);
        rig.accept("req-1", "serverStream");
        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::End {
                request_id: "req-1".to_string(),
                payload_present: false,
                payload: Vec::new(),
            },
        );
        assert_eq!(outcome.terminals[0].source, TerminalSource::ProtocolError);
        assert_eq!(
            outcome.terminals[0].cancel_frame.as_ref().unwrap().reason,
            "protocol_error"
        );
        rig.assert_zero(1);

        // Chunk seq gap after start.
        let rig = Rig::new(2);
        rig.accept("req-1", "serverStream");
        let _ = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Start {
                request_id: "req-1".to_string(),
            },
        );
        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Chunk {
                request_id: "req-1".to_string(),
                seq: 1,
                payload: Vec::new(),
            },
        );
        assert_eq!(outcome.terminals[0].source, TerminalSource::ProtocolError);
        rig.assert_zero(1);

        // Duplicate start.
        let rig = Rig::new(2);
        rig.accept("req-1", "serverStream");
        let _ = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Start {
                request_id: "req-1".to_string(),
            },
        );
        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Start {
                request_id: "req-1".to_string(),
            },
        );
        assert_eq!(outcome.terminals[0].source, TerminalSource::ProtocolError);
        rig.assert_zero(1);

        // Stream end with payload present.
        let rig = Rig::new(2);
        rig.accept("req-1", "serverStream");
        let _ = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Start {
                request_id: "req-1".to_string(),
            },
        );
        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::End {
                request_id: "req-1".to_string(),
                payload_present: true,
                payload: Vec::new(),
            },
        );
        assert_eq!(outcome.terminals[0].source, TerminalSource::ProtocolError);
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_unary_start_frame_is_protocol_error() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");
        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Start {
                request_id: "req-1".to_string(),
            },
        );
        assert_eq!(outcome.terminals[0].source, TerminalSource::ProtocolError);
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_task_attempt_end_with_payload_is_protocol_error() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");
        let result = rig.dispatcher.task_attempt_submit(task_attempt(
            "task-attempt-1",
            "task-1",
            "attempt-1",
            "lease-1",
        ));
        assert!(matches!(result, TaskAttemptSubmitResult::Accepted { .. }));

        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::End {
                request_id: "task-attempt-1".to_string(),
                payload_present: true,
                payload: Vec::new(),
            },
        );
        assert_eq!(outcome.terminals[0].source, TerminalSource::ProtocolError);
        assert!(!rig.dispatcher.is_task_attempt("task-attempt-1"));

        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::End {
                request_id: "req-1".to_string(),
                payload_present: true,
                payload: Vec::new(),
            },
        );
        assert_eq!(
            outcome.terminals[0].source,
            TerminalSource::RuntimeResponseEnd
        );
        rig.assert_zero(2);
    }

    #[test]
    fn dispatch_stale_response_from_wrong_session_is_ignored() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");
        let wrong_session = RuntimeSessionEpoch {
            replica_id: "other-runtime".to_string(),
            connection_generation: 9,
        };
        let outcome = rig.dispatcher.on_frame(
            &wrong_session,
            RuntimeResponseFrame::End {
                request_id: "req-1".to_string(),
                payload_present: true,
                payload: Vec::new(),
            },
        );
        assert!(outcome.frames.is_empty());
        assert!(outcome.terminals.is_empty());
        assert_eq!(rig.dispatcher.pending_count(), 1);
        assert_eq!(rig.dispatcher.health().admission.permits_held, 1);

        let outcome = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::End {
                request_id: "req-1".to_string(),
                payload_present: true,
                payload: Vec::new(),
            },
        );
        assert_eq!(
            outcome.terminals[0].source,
            TerminalSource::RuntimeResponseEnd
        );
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_caller_abort_uses_custom_wire_reason() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");
        let terminal = rig
            .dispatcher
            .caller_abort("req-1", Some("client_disconnect"))
            .expect("pending");
        assert_eq!(terminal.source, TerminalSource::CallerAbort);
        assert_eq!(
            terminal.cancel_frame,
            Some(CancelFrame {
                request_id: "req-1".to_string(),
                reason: "client_disconnect".to_string(),
            })
        );
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_client_disconnect_and_backpressure_terminals_send_cancel() {
        let rig = Rig::new(2);
        rig.accept("req-1", "unary");
        let terminal = rig.dispatcher.client_disconnect("req-1").expect("pending");
        assert_eq!(terminal.source, TerminalSource::ClientDisconnect);
        assert_eq!(terminal.cancel_frame.unwrap().reason, "client_disconnect");

        rig.accept("req-2", "unary");
        let terminal = rig.dispatcher.backpressure("req-2").expect("pending");
        assert_eq!(terminal.source, TerminalSource::Backpressure);
        assert_eq!(terminal.cancel_frame.unwrap().reason, "backpressure");
        rig.assert_zero(2);
    }

    #[test]
    fn dispatch_duplicate_task_attempt_request_id_is_rejected() {
        let rig = Rig::new(2);
        let task = || {
            task_attempt(
                "task-attempt-1",
                "task-1",
                "attempt-1",
                "lease-1",
            )
        };
        assert!(matches!(
            rig.dispatcher.task_attempt_submit(task()),
            TaskAttemptSubmitResult::Accepted { .. }
        ));
        assert_eq!(
            rig.dispatcher.task_attempt_submit(task()),
            TaskAttemptSubmitResult::Rejected {
                request_id: "task-attempt-1".to_string(),
                reason: SubmitRejectReason::Duplicate,
            }
        );
        assert_eq!(rig.dispatcher.health().admission.permits_held, 1);
    }

    #[test]
    fn dispatch_revalidate_failure_without_reselect_capacity_fails_closed() {
        let rig = Rig::new(1);
        rig.revalidate
            .state
            .lock()
            .unwrap()
            .injected
            .insert("req-1".to_string(), RevalidateOutcome::Cancelled);
        let reason = rig.reject("req-1", "unary");
        assert_eq!(reason, SubmitRejectReason::RevalidateFailClosed);
        let health = rig.dispatcher.health();
        assert_eq!(health.admission.revalidate_failures, 1);
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_no_epoch_source_fails_closed() {
        let candidate = FakeCandidateViewSource::new(vec![session_state("s1", "runtime-a", 1)]);
        let peer = FakeRuntimePeer::new();
        let abort = FakeSessionAbort::new();
        let revalidate = FakeLeaseRevalidate::new();
        let options = RuntimeDispatcherOptions::new(
            2,
            Arc::new(FakeEpochSource { epoch: None }),
            Arc::new(candidate),
            Arc::new(revalidate),
            Arc::new(peer),
            Arc::new(abort),
        )
        .expect("options");
        let dispatcher = RequestDispatcher::new(options).expect("dispatcher");
        let reason = match dispatcher.submit(request("req-1", "unary")) {
            SubmitResult::Rejected { reason, .. } => reason,
            other => panic!("expected reject, got {other:?}"),
        };
        assert_eq!(reason, SubmitRejectReason::NoCandidate);
        assert_eq!(dispatcher.health().admission.permits_held, 0);
    }

    #[test]
    fn dispatch_stream_error_after_start_fails_without_cancel() {
        let rig = Rig::new(2);
        rig.accept("req-1", "serverStream");
        let _ = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Start {
                request_id: "req-1".to_string(),
            },
        );
        let outcome = rig.dispatcher.on_frame(&rig.session, error_frame("req-1"));
        assert_eq!(
            outcome.terminals[0].source,
            TerminalSource::RuntimeResponseError
        );
        assert_eq!(outcome.terminals[0].cancel_frame, None);
        assert!(matches!(&outcome.frames[0], DispatchedFrame::Error { .. }));
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_response_frames_forward_opaque_payloads() {
        let rig = Rig::new(2);
        rig.accept("req-1", "serverStream");
        let start = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Start {
                request_id: "req-1".to_string(),
            },
        );
        assert!(matches!(start.frames[0], DispatchedFrame::Start { .. }));
        let chunk = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::Chunk {
                request_id: "req-1".to_string(),
                seq: 0,
                payload: vec![1, 2, 3],
            },
        );
        assert!(matches!(
            &chunk.frames[0],
            DispatchedFrame::Chunk { payload, .. } if payload == &vec![1, 2, 3]
        ));
        let end = rig.dispatcher.on_frame(
            &rig.session,
            RuntimeResponseFrame::End {
                request_id: "req-1".to_string(),
                payload_present: false,
                payload: Vec::new(),
            },
        );
        assert_eq!(end.terminals[0].source, TerminalSource::RuntimeResponseEnd);
        rig.assert_zero(1);
    }

    #[test]
    fn dispatch_pending_holds_epoch_lease_and_deadline() {
        let rig = Rig::new(2);
        let mut dispatch_request = request("req-1", "unary");
        dispatch_request.header.deadline = Some(
        skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestDeadlineFrameHeader {
            timeout_ms: 5000,
            expires_at: "2026-08-02T00:01:00Z".to_string(),
        },
    );
        let session_epoch = match rig.dispatcher.submit(dispatch_request) {
            SubmitResult::Accepted { session_epoch, .. } => session_epoch,
            other => panic!("expected accept, got {other:?}"),
        };
        assert_eq!(session_epoch, rig.session);
        assert!(rig.dispatcher.pending_epoch("req-1").is_some());
        let lease = rig.dispatcher.pending_lease("req-1").expect("lease");
        assert_eq!(lease.session_epoch, rig.session);
        assert_eq!(
            rig.dispatcher.pending_deadline("req-1"),
            Some(RequestDeadline {
                timeout_ms: 5000,
                expires_at: "2026-08-02T00:01:00Z".to_string(),
            })
        );
    }
}
