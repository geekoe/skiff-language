//! `RequestDispatcher`: ordinary unary/stream and derived function-spawn
//! pending, terminal and reservation correlation (plan §3.2, C-dispatch §2-§5).
//!
//! The dispatcher is a synchronous reducer: every method takes the state lock
//! for the duration of one decision and never crosses `.await` while holding
//! it. Session truth, sockets and the active routing epoch stay outside this
//! owner; they enter through the typed ports in `candidate`/`frame`.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::cancel_reason::RequestCancelReason;
use skiff_runtime_transport::protocol::ValidatedResponseErrorFrame;

use crate::bootstrap::RoutingEpoch;
use crate::routing::{DispatchMode, RegisteredSessionLease, RuntimeCandidateQuery};
use crate::session::identity::RuntimeSessionEpoch;

use super::admission::{Permit, PermitLedger, RuntimeAdmissionPool, SelectedLease};
use super::candidate::{
    candidate_query_from_request, dispatch_mode_from_wire, CandidateViewSource, LeaseRevalidate,
    RevalidateOutcome, RoutingEpochSource,
};
use super::frame::{
    ActorMethodSpawnControl, RuntimePeer, RuntimeResponseFrame, SessionAbortControl, TimeoutCheck,
    WireTimeoutCheck,
};
use super::health::{
    AdmissionHealth, DispatcherHealthSnapshot, PendingHealth, SpawnHealth, TerminalHealth,
    TerminalSource,
};
use super::types::{
    ActorMethodSpawnDispatch, DerivedSpawnResult, DispatchRequest, RequestAuthority,
    RequestDeadline, SpawnSubmit, SpawnTargetKind,
};

/// Dispatcher construction options (C-dispatch §3 `RuntimeDispatcherOptions`).
#[derive(Debug)]
pub struct RuntimeDispatcherOptions {
    pub max_concurrency: usize,
    pub epoch_source: Arc<dyn RoutingEpochSource>,
    pub candidate_view: Arc<dyn CandidateViewSource>,
    pub revalidate: Arc<dyn LeaseRevalidate>,
    pub peer: Arc<dyn RuntimePeer>,
    pub session_abort: Arc<dyn SessionAbortControl>,
    pub actor_spawn_control: Arc<dyn ActorMethodSpawnControl>,
    pub timeout_check: Arc<dyn TimeoutCheck>,
}

impl RuntimeDispatcherOptions {
    pub fn new(
        max_concurrency: usize,
        epoch_source: Arc<dyn RoutingEpochSource>,
        candidate_view: Arc<dyn CandidateViewSource>,
        revalidate: Arc<dyn LeaseRevalidate>,
        peer: Arc<dyn RuntimePeer>,
        session_abort: Arc<dyn SessionAbortControl>,
        actor_spawn_control: Arc<dyn ActorMethodSpawnControl>,
    ) -> Result<Self, String> {
        if max_concurrency < 1 {
            return Err("maxConcurrency must be >= 1".to_string());
        }
        Ok(Self {
            max_concurrency,
            epoch_source,
            candidate_view,
            revalidate,
            peer,
            session_abort,
            actor_spawn_control,
            timeout_check: Arc::new(WireTimeoutCheck),
        })
    }

    pub fn with_timeout_check(mut self, timeout_check: Arc<dyn TimeoutCheck>) -> Self {
        self.timeout_check = timeout_check;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Unary,
    Stream,
    DerivedSpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    WaitingStart,
    Streaming,
}

/// One dispatcher-owned pending (plan §3.3 step 6).
///
/// Invariant: every pending holds the captured routing epoch, the exact
/// registered session lease and one admission permit; terminal removes the
/// pending and releases the permit exactly once.
#[derive(Debug)]
struct Pending {
    kind: PendingKind,
    session_epoch: RuntimeSessionEpoch,
    epoch: Arc<RoutingEpoch>,
    lease: RegisteredSessionLease,
    permit: Permit,
    stream_phase: Option<StreamPhase>,
    next_seq: u64,
    authority: RequestAuthority,
    deadline: Option<RequestDeadline>,
}

#[derive(Debug)]
struct DispatcherInner {
    pool: RuntimeAdmissionPool,
    pending: BTreeMap<String, Pending>,
    /// Terminal observation of closed sessions (idempotent cleanup state,
    /// not session truth): selection skips these even if a stale query returns
    /// them, closing the race between directory cancellation and enqueue.
    closed_sessions: HashSet<RuntimeSessionEpoch>,
    stopped: bool,
    terminal_by_source: BTreeMap<TerminalSource, u64>,
    spawn_derived: u64,
    spawn_actor_lane: u64,
    spawn_ambiguous: u64,
}

/// `RequestDispatcher` owner (plan §3.2).
#[derive(Debug)]
pub struct RequestDispatcher {
    options: Arc<RuntimeDispatcherOptions>,
    inner: Arc<Mutex<DispatcherInner>>,
}

impl RequestDispatcher {
    pub fn new(options: RuntimeDispatcherOptions) -> Result<Self, String> {
        let pool = RuntimeAdmissionPool::new(options.max_concurrency);
        Ok(Self {
            options: Arc::new(options),
            inner: Arc::new(Mutex::new(DispatcherInner {
                pool,
                pending: BTreeMap::new(),
                closed_sessions: HashSet::new(),
                stopped: false,
                terminal_by_source: BTreeMap::new(),
                spawn_derived: 0,
                spawn_actor_lane: 0,
                spawn_ambiguous: 0,
            })),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DispatcherInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Ordinary unary/stream admission (C-dispatch §3).
    ///
    /// Fail-closed rejections never enqueue and never leak a permit; a permit
    /// reserved before the enqueue-time deadline recheck is released and
    /// counted.
    pub fn submit(&self, request: DispatchRequest) -> SubmitResult {
        let request_id = request.request_id().to_string();
        let deadline = request.deadline();
        let mut inner = self.lock();

        if inner.stopped {
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::Shutdown,
            };
        }
        if inner.pending.contains_key(&request_id) {
            inner.pool.record_duplicate_request_id();
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::Duplicate,
            };
        }
        if deadline
            .as_ref()
            .is_some_and(|deadline| self.options.timeout_check.is_expired(deadline))
        {
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::DeadlineExpired,
            };
        }

        if dispatch_mode_from_wire(&request.header.mode).is_none() {
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::InvalidMode,
            };
        }
        let Some(epoch) = self.options.epoch_source.capture() else {
            inner.pool.record_no_candidate();
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::NoCandidate,
            };
        };
        let query = candidate_query_from_request(&request);
        let view = self.options.candidate_view.view();
        let leases = match RuntimeCandidateQuery.query(&epoch, &view, &query) {
            Ok(leases) => leases,
            Err(_) => {
                inner.pool.record_no_candidate();
                return SubmitResult::Rejected {
                    request_id,
                    reason: SubmitRejectReason::NoCandidate,
                };
            }
        }
        .into_iter()
        .filter(|lease| {
            !lease.cancellation.cancelled && !inner.closed_sessions.contains(&lease.session_epoch)
        })
        .collect::<Vec<_>>();
        if leases.is_empty() {
            inner.pool.record_no_candidate();
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::NoCandidate,
            };
        }

        let Some(selected) = inner.pool.select(&leases, request.prefer_session.as_ref()) else {
            inner.pool.record_queue_full();
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::QueueFull,
            };
        };

        if deadline
            .as_ref()
            .is_some_and(|deadline| self.options.timeout_check.is_expired(deadline))
        {
            selected.reservation.release();
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::DeadlineExpired,
            };
        }

        if !matches!(
            self.options
                .revalidate
                .revalidate(&request_id, &selected.lease),
            RevalidateOutcome::Ok
        ) {
            inner.pool.record_revalidate_failure();
            selected.reservation.release();
            let Some(reselected) = inner
                .pool
                .select_after_revalidate_failure(&leases, &selected.lease.session_epoch)
            else {
                return SubmitResult::Rejected {
                    request_id,
                    reason: SubmitRejectReason::RevalidateFailClosed,
                };
            };
            inner.pool.record_reselect();
            return self.enqueue_locked(&mut inner, request, reselected, epoch);
        }

        self.enqueue_locked(&mut inner, request, selected, epoch)
    }

    fn enqueue_locked(
        &self,
        inner: &mut DispatcherInner,
        request: DispatchRequest,
        selected: SelectedLease,
        epoch: Arc<RoutingEpoch>,
    ) -> SubmitResult {
        let request_id = request.request_id().to_string();
        let session_epoch = selected.lease.session_epoch.clone();
        let permit = selected.reservation.commit();
        if let Err(write_error) = self
            .options
            .peer
            .send_request_start(&session_epoch, &request)
        {
            permit.release();
            *inner
                .terminal_by_source
                .entry(TerminalSource::CallbackError)
                .or_insert(0) += 1;
            let _ = self.options.peer.send_request_cancel(
                &session_epoch,
                &request_id,
                "protocol_error",
            );
            self.options.session_abort.abort_session(&session_epoch);
            let _ = write_error;
            return SubmitResult::Rejected {
                request_id,
                reason: SubmitRejectReason::CallbackError,
            };
        }
        let kind = match request.mode() {
            DispatchMode::Unary => PendingKind::Unary,
            DispatchMode::ServerStream => PendingKind::Stream,
        };
        let authority = request.authority(&session_epoch);
        let deadline = request.deadline();
        inner.pending.insert(
            request_id.clone(),
            Pending {
                kind,
                session_epoch: session_epoch.clone(),
                epoch,
                lease: selected.lease,
                permit,
                stream_phase: (kind == PendingKind::Stream).then_some(StreamPhase::WaitingStart),
                next_seq: 0,
                authority,
                deadline,
            },
        );
        SubmitResult::Accepted {
            request_id,
            session_epoch,
        }
    }

    /// Runtime response/cancel frame delivery (exact-socket fence, §4.4).
    pub fn on_frame(
        &self,
        session: &RuntimeSessionEpoch,
        frame: RuntimeResponseFrame,
    ) -> FrameOutcome {
        let request_id = frame.request_id().to_string();
        let mut inner = self.lock();
        if self.is_stale(&inner, session, &request_id) {
            return FrameOutcome::default();
        }
        match frame {
            RuntimeResponseFrame::Start { .. } => self.on_start(&mut inner, &request_id),
            RuntimeResponseFrame::Chunk { seq, payload, .. } => {
                self.on_chunk(&mut inner, &request_id, seq, payload)
            }
            RuntimeResponseFrame::End {
                payload_present,
                payload,
                ..
            } => self.on_end(&mut inner, &request_id, payload_present, payload),
            RuntimeResponseFrame::Error { error, .. } => {
                self.on_error(&mut inner, &request_id, error)
            }
            RuntimeResponseFrame::Cancel { reason, .. } => {
                self.on_runtime_cancel(&mut inner, &request_id, reason)
            }
        }
    }

    fn is_stale(
        &self,
        inner: &DispatcherInner,
        session: &RuntimeSessionEpoch,
        request_id: &str,
    ) -> bool {
        inner
            .pending
            .get(request_id)
            .is_none_or(|pending| pending.session_epoch != *session)
    }

    fn on_start(&self, inner: &mut DispatcherInner, request_id: &str) -> FrameOutcome {
        let accepted = inner.pending.get(request_id).is_some_and(|pending| {
            pending.kind == PendingKind::Stream
                && pending.stream_phase == Some(StreamPhase::WaitingStart)
        });
        if accepted {
            inner
                .pending
                .get_mut(request_id)
                .expect("pending exists")
                .stream_phase = Some(StreamPhase::Streaming);
            FrameOutcome {
                frames: vec![DispatchedFrame::Start {
                    request_id: request_id.to_string(),
                }],
                terminals: Vec::new(),
            }
        } else {
            self.protocol_error_terminal(inner, request_id)
        }
    }

    fn on_chunk(
        &self,
        inner: &mut DispatcherInner,
        request_id: &str,
        seq: u64,
        payload: Vec<u8>,
    ) -> FrameOutcome {
        let accepted = inner.pending.get(request_id).is_some_and(|pending| {
            pending.kind == PendingKind::Stream
                && pending.stream_phase == Some(StreamPhase::Streaming)
                && seq == pending.next_seq
        });
        if accepted {
            let pending = inner.pending.get_mut(request_id).expect("pending exists");
            pending.next_seq += 1;
            FrameOutcome {
                frames: vec![DispatchedFrame::Chunk {
                    request_id: request_id.to_string(),
                    seq,
                    payload,
                }],
                terminals: Vec::new(),
            }
        } else {
            self.protocol_error_terminal(inner, request_id)
        }
    }

    fn on_end(
        &self,
        inner: &mut DispatcherInner,
        request_id: &str,
        payload_present: bool,
        payload: Vec<u8>,
    ) -> FrameOutcome {
        let completes = match inner.pending.get(request_id) {
            None => false,
            Some(pending) => match pending.kind {
                PendingKind::Unary => true,
                PendingKind::Stream => {
                    pending.stream_phase == Some(StreamPhase::Streaming) && !payload_present
                }
                PendingKind::DerivedSpawn => !payload_present,
            },
        };
        if !completes {
            return self.protocol_error_terminal(inner, request_id);
        }
        let terminal = self
            .terminal_locked(
                inner,
                request_id,
                RequestOutcome::Completed,
                TerminalSource::RuntimeResponseEnd,
                None,
            )
            .expect("completing pending exists");
        FrameOutcome {
            frames: vec![DispatchedFrame::End {
                request_id: request_id.to_string(),
                payload,
            }],
            terminals: vec![terminal],
        }
    }

    fn on_error(
        &self,
        inner: &mut DispatcherInner,
        request_id: &str,
        error: ValidatedResponseErrorFrame,
    ) -> FrameOutcome {
        let terminal = self
            .terminal_locked(
                inner,
                request_id,
                RequestOutcome::Failed,
                TerminalSource::RuntimeResponseError,
                None,
            )
            .expect("pending exists");
        FrameOutcome {
            frames: vec![DispatchedFrame::Error {
                request_id: request_id.to_string(),
                error,
            }],
            terminals: vec![terminal],
        }
    }

    fn on_runtime_cancel(
        &self,
        inner: &mut DispatcherInner,
        request_id: &str,
        reason: String,
    ) -> FrameOutcome {
        let known = RequestCancelReason::from_wire(&reason)
            .is_some_and(|parsed| RequestCancelReason::CONTRACT_H.contains(&parsed));
        if !known {
            return self.protocol_error_terminal(inner, request_id);
        }
        let terminal = self
            .terminal_locked(
                inner,
                request_id,
                RequestOutcome::Cancelled,
                TerminalSource::RuntimeRequestCancel,
                None,
            )
            .expect("pending exists");
        FrameOutcome {
            frames: Vec::new(),
            terminals: vec![terminal],
        }
    }

    fn protocol_error_terminal(
        &self,
        inner: &mut DispatcherInner,
        request_id: &str,
    ) -> FrameOutcome {
        let terminal = self
            .terminal_locked(
                inner,
                request_id,
                RequestOutcome::ProtocolError,
                TerminalSource::ProtocolError,
                Some("protocol_error"),
            )
            .expect("pending exists");
        FrameOutcome {
            frames: Vec::new(),
            terminals: vec![terminal],
        }
    }

    /// Runtime disconnect / replacement terminal for one session
    /// (C-dispatch §7.5): all pending on the exact session terminate
    /// `runtime_disconnect` without cancel frames.
    pub fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Vec<PendingTerminal> {
        let mut inner = self.lock();
        inner.closed_sessions.insert(session.clone());
        let request_ids = inner
            .pending
            .iter()
            .filter(|(_, pending)| pending.session_epoch == *session)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        request_ids
            .into_iter()
            .filter_map(|request_id| {
                self.terminal_locked(
                    &mut inner,
                    &request_id,
                    RequestOutcome::Cancelled,
                    TerminalSource::RuntimeDisconnect,
                    None,
                )
            })
            .collect()
    }

    /// Process shutdown: refuse new admission and terminate every pending
    /// `router_shutdown` with a cancel frame (C-dispatch §7.5).
    pub fn shutdown(&self) -> Vec<PendingTerminal> {
        let mut inner = self.lock();
        inner.stopped = true;
        let request_ids = inner.pending.keys().cloned().collect::<Vec<_>>();
        request_ids
            .into_iter()
            .filter_map(|request_id| {
                self.terminal_locked(
                    &mut inner,
                    &request_id,
                    RequestOutcome::Cancelled,
                    TerminalSource::RouterShutdown,
                    Some("router_shutdown"),
                )
            })
            .collect()
    }

    /// Deadline timeout terminal (C-dispatch §4.3: sends `request.cancel`).
    pub fn timeout(&self, request_id: &str) -> Option<PendingTerminal> {
        let mut inner = self.lock();
        self.terminal_locked(
            &mut inner,
            request_id,
            RequestOutcome::Cancelled,
            TerminalSource::Timeout,
            Some("timeout"),
        )
    }

    /// Caller abort terminal (C-dispatch §4.3: `caller_cancel` by default,
    /// caller-specified wire reason allowed).
    pub fn caller_abort(
        &self,
        request_id: &str,
        wire_reason: Option<&str>,
    ) -> Option<PendingTerminal> {
        let reason = wire_reason
            .filter(|reason| !reason.is_empty())
            .unwrap_or("caller_cancel");
        let mut inner = self.lock();
        self.terminal_locked(
            &mut inner,
            request_id,
            RequestOutcome::Cancelled,
            TerminalSource::CallerAbort,
            Some(reason),
        )
    }

    /// Client disconnect terminal (C-dispatch §4.3: `client_disconnect`).
    pub fn client_disconnect(&self, request_id: &str) -> Option<PendingTerminal> {
        let mut inner = self.lock();
        self.terminal_locked(
            &mut inner,
            request_id,
            RequestOutcome::Cancelled,
            TerminalSource::ClientDisconnect,
            Some("client_disconnect"),
        )
    }

    /// Backpressure terminal (C-dispatch §4.3: `backpressure`).
    pub fn backpressure(&self, request_id: &str) -> Option<PendingTerminal> {
        let mut inner = self.lock();
        self.terminal_locked(
            &mut inner,
            request_id,
            RequestOutcome::Cancelled,
            TerminalSource::Backpressure,
            Some("backpressure"),
        )
    }

    /// Spawn correlation (C-dispatch §5).
    ///
    /// Function spawns enter dispatcher-owned `derivedSpawn` pending on the
    /// parent session and consume one per-session permit; actor-method spawns
    /// are forwarded to the actor lane and never enter this owner.
    pub fn spawn_submit(&self, spawn: SpawnSubmit) -> SpawnSubmitResult {
        let mut inner = self.lock();
        if inner.stopped {
            return SpawnSubmitResult::Rejected {
                request_id: spawn.spawn_request_id,
                reason: SpawnRejectReason::Shutdown,
            };
        }
        let request_parent = inner.pending.contains_key(&spawn.caller_request_id);
        let actor_parent = self
            .options
            .actor_spawn_control
            .is_active_invocation_parent(&spawn.caller_request_id);
        match (request_parent, actor_parent) {
            (true, true) => {
                inner.spawn_ambiguous += 1;
                return SpawnSubmitResult::Rejected {
                    request_id: spawn.spawn_request_id,
                    reason: SpawnRejectReason::Ambiguous,
                };
            }
            (false, false) => {
                return SpawnSubmitResult::Rejected {
                    request_id: spawn.spawn_request_id,
                    reason: SpawnRejectReason::NoParent,
                };
            }
            _ => {}
        }
        match spawn.target_kind {
            SpawnTargetKind::Function => self.spawn_function_locked(&mut inner, spawn),
            SpawnTargetKind::ActorMethod => {
                inner.spawn_actor_lane += 1;
                let dispatch = ActorMethodSpawnDispatch {
                    spawn_request_id: spawn.spawn_request_id,
                    caller_request_id: spawn.caller_request_id,
                    target: spawn.target,
                };
                self.options
                    .actor_spawn_control
                    .submit_spawn(dispatch.clone());
                SpawnSubmitResult::ForwardedActorMethod(dispatch)
            }
        }
    }

    fn spawn_function_locked(
        &self,
        inner: &mut DispatcherInner,
        spawn: SpawnSubmit,
    ) -> SpawnSubmitResult {
        if !inner.pending.contains_key(&spawn.caller_request_id) {
            return SpawnSubmitResult::Rejected {
                request_id: spawn.spawn_request_id,
                reason: SpawnRejectReason::WrongParentKind,
            };
        }
        if inner.pending.contains_key(&spawn.spawn_request_id) {
            return SpawnSubmitResult::Rejected {
                request_id: spawn.spawn_request_id,
                reason: SpawnRejectReason::Duplicate,
            };
        }
        let parent = inner
            .pending
            .get(&spawn.caller_request_id)
            .expect("request parent exists");
        if parent.authority != spawn.authority {
            return SpawnSubmitResult::Rejected {
                request_id: spawn.spawn_request_id,
                reason: SpawnRejectReason::ParentAuthorityMismatch,
            };
        }
        let parent_session = parent.session_epoch.clone();
        if inner.closed_sessions.contains(&parent_session) {
            return SpawnSubmitResult::Rejected {
                request_id: spawn.spawn_request_id,
                reason: SpawnRejectReason::ParentTerminal,
            };
        }
        let Some(reservation) = inner.pool.reserve_exact(&parent_session) else {
            inner.pool.record_queue_full();
            return SpawnSubmitResult::Rejected {
                request_id: spawn.spawn_request_id,
                reason: SpawnRejectReason::QueueFull,
            };
        };
        let permit = reservation.commit();
        if let Err(write_error) = self.options.peer.send_spawn_submit(&parent_session, &spawn) {
            permit.release();
            *inner
                .terminal_by_source
                .entry(TerminalSource::CallbackError)
                .or_insert(0) += 1;
            let _ = self.options.peer.send_request_cancel(
                &parent_session,
                &spawn.spawn_request_id,
                "protocol_error",
            );
            self.options.session_abort.abort_session(&parent_session);
            let _ = write_error;
            return SpawnSubmitResult::Rejected {
                request_id: spawn.spawn_request_id,
                reason: SpawnRejectReason::CallbackError,
            };
        }
        let parent_request_id = spawn.caller_request_id.clone();
        let spawn_request_id = spawn.spawn_request_id.clone();
        inner.spawn_derived += 1;
        inner.pending.insert(
            spawn_request_id.clone(),
            Pending {
                kind: PendingKind::DerivedSpawn,
                session_epoch: parent_session.clone(),
                epoch: parent.epoch.clone(),
                lease: parent.lease.clone(),
                permit,
                stream_phase: None,
                next_seq: 0,
                authority: parent.authority.clone(),
                deadline: spawn.deadline,
            },
        );
        SpawnSubmitResult::AcceptedDerived(DerivedSpawnResult {
            spawn_request_id,
            parent_request_id,
            session_epoch: parent_session,
        })
    }

    /// Terminal reducer: detach pending, release permit exactly once, send the
    /// cancel frame when the source requires it, and count the terminal.
    ///
    /// Cancel-frame write failure escalates the source to `callback_error` and
    /// requests an abort of the exact session (C-dispatch §7.4); the cancel
    /// frame is never awaited.
    fn terminal_locked(
        &self,
        inner: &mut DispatcherInner,
        request_id: &str,
        outcome: RequestOutcome,
        source: TerminalSource,
        cancel_reason: Option<&str>,
    ) -> Option<PendingTerminal> {
        let pending = inner.pending.remove(request_id)?;
        pending.permit.release();
        let mut effective_source = source;
        let mut cancel_frame = None;
        if let Some(reason) = cancel_reason {
            match self
                .options
                .peer
                .send_request_cancel(&pending.session_epoch, request_id, reason)
            {
                Ok(()) => {
                    cancel_frame = Some(CancelFrame {
                        request_id: request_id.to_string(),
                        reason: reason.to_string(),
                    });
                }
                Err(_) => {
                    effective_source = TerminalSource::CallbackError;
                    self.options
                        .session_abort
                        .abort_session(&pending.session_epoch);
                }
            }
        }
        *inner
            .terminal_by_source
            .entry(effective_source)
            .or_insert(0) += 1;
        Some(PendingTerminal {
            request_id: request_id.to_string(),
            outcome,
            source: effective_source,
            cancel_frame,
        })
    }

    /// Health snapshot (C-dispatch §7.6). Never exposes request ids or
    /// payload bytes.
    pub fn health(&self) -> DispatcherHealthSnapshot {
        let inner = self.lock();
        let counters = inner.pool.counters();
        let mut pending = PendingHealth::default();
        for entry in inner.pending.values() {
            match entry.kind {
                PendingKind::Unary => pending.unary += 1,
                PendingKind::Stream => pending.stream += 1,
                PendingKind::DerivedSpawn => pending.derived_spawn += 1,
            }
        }
        DispatcherHealthSnapshot {
            pending,
            terminal: TerminalHealth {
                by_source: inner.terminal_by_source.clone(),
            },
            admission: AdmissionHealth {
                permits_held: inner.pool.permits_held(),
                releases: counters.releases,
                queue_full_rejects: counters.queue_full_rejects,
                revalidate_failures: counters.revalidate_failures,
                reselects: counters.reselects,
                no_candidate_rejects: counters.no_candidate_rejects,
                duplicate_request_id_rejects: counters.duplicate_request_id_rejects,
            },
            spawn: SpawnHealth {
                derived_spawns: inner.spawn_derived,
                actor_lane_spawns: inner.spawn_actor_lane,
                ambiguous_rejects: inner.spawn_ambiguous,
            },
            stopped: inner.stopped,
        }
    }

    /// Observable permit ledger for pending/permit-to-zero assertions.
    pub fn permit_ledger(&self) -> PermitLedger {
        let inner = self.lock();
        PermitLedger::from_pool(&inner.pool)
    }

    pub fn pending_count(&self) -> usize {
        self.lock().pending.len()
    }

    /// Captured epoch held by one pending (integration seam; pending invariant
    /// §3.3 step 6).
    pub fn pending_epoch(&self, request_id: &str) -> Option<Arc<RoutingEpoch>> {
        self.lock()
            .pending
            .get(request_id)
            .map(|pending| pending.epoch.clone())
    }

    /// Registered session lease held by one pending (integration seam).
    pub fn pending_lease(&self, request_id: &str) -> Option<RegisteredSessionLease> {
        self.lock()
            .pending
            .get(request_id)
            .map(|pending| pending.lease.clone())
    }

    /// Deadline held by one pending (W-http timer integration seam).
    pub fn pending_deadline(&self, request_id: &str) -> Option<RequestDeadline> {
        self.lock()
            .pending
            .get(request_id)
            .and_then(|pending| pending.deadline.clone())
    }
}

/// Admission result (C-dispatch §3/§7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitResult {
    Accepted {
        request_id: String,
        session_epoch: RuntimeSessionEpoch,
    },
    Rejected {
        request_id: String,
        reason: SubmitRejectReason,
    },
}

/// Fail-closed admission rejection vocabulary (C-dispatch §3/§7.4).
///
/// The corpus-pinned strings are `duplicate`, `queue_full`, `no_candidate`,
/// `revalidate_fail_closed`, `shutdown`; the remaining reasons cover
/// implementation-level fail-closed paths (not part of the frozen corpus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitRejectReason {
    Duplicate,
    QueueFull,
    NoCandidate,
    RevalidateFailClosed,
    Shutdown,
    DeadlineExpired,
    InvalidMode,
    CallbackError,
}

impl SubmitRejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Duplicate => "duplicate",
            Self::QueueFull => "queue_full",
            Self::NoCandidate => "no_candidate",
            Self::RevalidateFailClosed => "revalidate_fail_closed",
            Self::Shutdown => "shutdown",
            Self::DeadlineExpired => "deadline_expired",
            Self::InvalidMode => "invalid_mode",
            Self::CallbackError => "callback_error",
        }
    }
}

/// Spawn correlation result (C-dispatch §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnSubmitResult {
    AcceptedDerived(DerivedSpawnResult),
    ForwardedActorMethod(ActorMethodSpawnDispatch),
    Rejected {
        request_id: String,
        reason: SpawnRejectReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnRejectReason {
    Ambiguous,
    NoParent,
    WrongParentKind,
    Duplicate,
    ParentTerminal,
    QueueFull,
    Shutdown,
    ParentAuthorityMismatch,
    CallbackError,
}

impl SpawnRejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ambiguous => "ambiguous",
            Self::NoParent => "no_parent",
            Self::WrongParentKind => "wrong_parent_kind",
            Self::Duplicate => "duplicate",
            Self::ParentTerminal => "parent_terminal",
            Self::QueueFull => "queue_full",
            Self::Shutdown => "shutdown",
            Self::ParentAuthorityMismatch => "parent_authority_mismatch",
            Self::CallbackError => "callback_error",
        }
    }
}

/// Terminal outcome classes (C-dispatch §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    Completed,
    Failed,
    Cancelled,
    ProtocolError,
}

impl RequestOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::ProtocolError => "protocolError",
        }
    }
}

/// Router-to-Runtime `request.cancel` frame emitted by a terminal
/// (C-dispatch §4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelFrame {
    pub request_id: String,
    pub reason: String,
}

/// Terminal observation returned to the caller (C-dispatch §7.2
/// `PendingTerminal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTerminal {
    pub request_id: String,
    pub outcome: RequestOutcome,
    pub source: TerminalSource,
    pub cancel_frame: Option<CancelFrame>,
}

/// Response frame accepted by the dispatcher and forwarded to the ingress
/// consumer (W-http). Payload bytes stay opaque.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchedFrame {
    Start {
        request_id: String,
    },
    Chunk {
        request_id: String,
        seq: u64,
        payload: Vec<u8>,
    },
    End {
        request_id: String,
        payload: Vec<u8>,
    },
    Error {
        request_id: String,
        error: ValidatedResponseErrorFrame,
    },
}

/// Combined outcome of one inbound frame (exact-fence filtering, dispatch
/// events and terminals).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameOutcome {
    pub frames: Vec<DispatchedFrame>,
    pub terminals: Vec<PendingTerminal>,
}
