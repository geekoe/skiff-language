use std::{
    collections::{hash_map::Entry, HashMap},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use serde_json::{Map, Value};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionEvent, BytecodeExecutionObserver, BytecodeRequestTerminal,
    RequestCleanupComplete, RequestTerminalClaimed,
};
use skiff_runtime_model::service_error::{
    ErrorCorrelation, OpaqueServiceError, ServiceErrorEnvelope,
};
use skiff_runtime_request::{
    cancellation::CancellationToken, execution_budget::ExecutionBudget,
    execution_budget_trace_attrs, response_error_to_telemetry_map, RequestCancel, RequestEnvelope,
    ResponseError,
};

use crate::telemetry::RequestTelemetryContext;

#[derive(Clone)]
struct ReservedRequest {
    row_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
}
#[derive(Clone)]
struct ActiveRequest {
    row_identity: Arc<()>,
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    telemetry: RequestTelemetryContext,
    started_at: Instant,
    cancel_requested: Arc<AtomicBool>,
    cancel_event_emitted: Arc<AtomicBool>,
    observer: BytecodeExecutionObserver,
}

struct CompletingRequest {
    row_identity: Arc<()>,
}

struct CleanupRequest {
    guard_identity: Arc<()>,
}

enum RequestRow {
    Reserved(ReservedRequest),
    Active(ActiveRequest),
    Completing(CompletingRequest),
    Cleanup(CleanupRequest),
}

#[derive(Clone)]
pub(crate) struct SupervisedRequest {
    request_id: String,
    active: ActiveRequest,
}

/// RAII ownership of one vacant request row during fallible admission.
///
/// Dropping an unactivated reservation removes only its exact reserved row and
/// emits no terminal or cleanup observation.
pub(crate) struct RequestReservation {
    supervisor: Arc<RequestSupervisor>,
    request_id: String,
    row_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
    armed: bool,
}

/// Uncloneable proof that this lane won the exact active-to-completing transition.
struct CompletionWinner {
    supervisor: Arc<RequestSupervisor>,
    request_id: String,
    active: ActiveRequest,
    cancelled: bool,
}

/// Uncloneable authority for the request-task finalizer to mint cleanup.
pub(crate) struct CleanupPermit {
    supervisor: Arc<RequestSupervisor>,
    request_id: String,
    row_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
    response_owned: bool,
}

/// Short-lived exact guard installed before the cleanup observer is called.
struct CleanupGuard {
    supervisor: Arc<RequestSupervisor>,
    request_id: String,
    guard_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
}

#[derive(Clone, Copy)]
pub(crate) struct CompletionTrace {
    include_duration: bool,
    include_budget_attrs: bool,
}

impl CompletionTrace {
    pub(crate) const RUNTIME: Self = Self {
        include_duration: true,
        include_budget_attrs: true,
    };
}

#[derive(Default)]
pub(crate) struct RequestSupervisor {
    rows: Mutex<HashMap<String, RequestRow>>,
}

impl RequestSupervisor {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Atomically inserts a reserved row only when the request id is vacant.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        request_id: String,
        observer: BytecodeExecutionObserver,
    ) -> Option<RequestReservation> {
        let row_identity = Arc::new(());
        let reserved = ReservedRequest {
            row_identity: Arc::clone(&row_identity),
            observer: observer.clone(),
        };
        let mut rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        match rows.entry(request_id.clone()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                entry.insert(RequestRow::Reserved(reserved));
                Some(RequestReservation {
                    supervisor: Arc::clone(self),
                    request_id,
                    row_identity,
                    observer,
                    armed: true,
                })
            }
        }
    }

    pub(crate) async fn complete_success(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        let winner = self.claim_completion(request)?;
        if winner.cancelled {
            return Some(finish_cancelled_request(winner, trace));
        }
        winner.active.execution_budget.finish(Instant::now());
        observe_terminal(&winner.active, BytecodeRequestTerminal::Succeeded);
        let duration_ms = elapsed_ms(winner.active.started_at);
        emit_request_duration_metric(&winner.active, duration_ms, "ok");
        Some(winner.into_cleanup_permit(true))
    }

    pub(crate) async fn complete_error(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        event_name: &'static str,
        error: &ResponseError,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        let winner = self.claim_completion(request)?;
        if winner.cancelled {
            return Some(finish_cancelled_request(winner, trace));
        }
        winner.active.execution_budget.finish(Instant::now());
        observe_terminal(&winner.active, BytecodeRequestTerminal::Failed);

        let duration_ms = elapsed_ms(winner.active.started_at);
        let duplicate_cancel = event_name == "request.cancel"
            && winner
                .active
                .cancel_event_emitted
                .swap(true, Ordering::SeqCst);
        if !duplicate_cancel {
            winner.active.telemetry.emit_trace(
                event_name,
                trace.include_duration.then_some(duration_ms),
                Some(response_error_to_telemetry_map(error)),
                budget_attrs(&winner.active, duration_ms, trace),
            );
            emit_request_duration_metric(&winner.active, duration_ms, "error");
        }
        Some(winner.into_cleanup_permit(true))
    }

    pub(crate) async fn complete_fixed_service_failure(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        event_name: &'static str,
        error: &OpaqueServiceError,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        let winner = self.claim_completion(request)?;
        if winner.cancelled {
            return Some(finish_cancelled_request(winner, trace));
        }
        winner.active.execution_budget.finish(Instant::now());
        observe_terminal(&winner.active, BytecodeRequestTerminal::Failed);

        let duration_ms = elapsed_ms(winner.active.started_at);
        let correlation = ErrorCorrelation {
            trace_id: error.envelope().trace_id().to_string(),
            error_id: error.envelope().error_id().to_string(),
        };
        winner.active.telemetry.emit_trace_with_error_correlation(
            event_name,
            trace.include_duration.then_some(duration_ms),
            Some(fixed_service_failure_telemetry_map(error)),
            budget_attrs(&winner.active, duration_ms, trace),
            &correlation,
        );
        emit_request_duration_metric(&winner.active, duration_ms, "error");
        Some(winner.into_cleanup_permit(true))
    }

    pub(crate) async fn complete_cancelled(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        self.claim_completion(request)
            .map(|winner| finish_cancelled_request(winner, trace))
    }

    pub(crate) async fn cancel(&self, cancel: &RequestCancel) -> bool {
        let (active, duration_ms, emit_cancel) = {
            let rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
            let Some(RequestRow::Active(active)) = rows.get(&cancel.request_id) else {
                return false;
            };
            active.cancel_requested.store(true, Ordering::SeqCst);
            active.cancellation.cancel();
            active.execution_budget.record_cancelled();
            let duration_ms = elapsed_ms(active.started_at);
            let emit_cancel = !active.cancel_event_emitted.swap(true, Ordering::SeqCst);
            (active.clone(), duration_ms, emit_cancel)
        };

        if emit_cancel {
            let mut attrs = execution_budget_trace_attrs(&active.execution_budget, duration_ms);
            if let Some(reason) = cancel.reason.as_deref() {
                attrs.insert("reason".to_string(), Value::String(reason.to_string()));
            }
            active
                .telemetry
                .emit_trace("request.cancel", Some(duration_ms), None, Some(attrs));
            emit_request_duration_metric(&active, duration_ms, "cancel");
        }
        true
    }

    /// Reserved rows reject duplicates but are not counted as admitted work.
    pub(crate) async fn active_count(&self) -> usize {
        self.rows
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|row| matches!(row, RequestRow::Active(_)))
            .count()
    }

    fn claim_completion(self: &Arc<Self>, request: &SupervisedRequest) -> Option<CompletionWinner> {
        let mut rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        let Entry::Occupied(mut entry) = rows.entry(request.request_id.clone()) else {
            return None;
        };
        let RequestRow::Active(current) = entry.get() else {
            return None;
        };
        if !Arc::ptr_eq(&current.row_identity, &request.active.row_identity) {
            return None;
        }
        let completing = CompletingRequest {
            row_identity: Arc::clone(&current.row_identity),
        };
        let RequestRow::Active(active) = entry.insert(RequestRow::Completing(completing)) else {
            unreachable!("matching active row was replaced")
        };
        // Only an explicit root cancellation may change an ordinary
        // success/error completion into the cancellation terminal.
        let cancelled = active.cancel_requested.load(Ordering::SeqCst);
        Some(CompletionWinner {
            supervisor: Arc::clone(self),
            request_id: request.request_id.clone(),
            active,
            cancelled,
        })
    }
}

impl RequestReservation {
    pub(crate) fn observer(&self) -> &BytecodeExecutionObserver {
        &self.observer
    }

    /// Activates an admitted request and creates its budget/handles only now.
    pub(crate) fn activate(
        mut self,
        request: &RequestEnvelope,
        telemetry: RequestTelemetryContext,
    ) -> Option<SupervisedRequest> {
        if request.request_id != self.request_id {
            return None;
        }
        let active = ActiveRequest {
            row_identity: Arc::clone(&self.row_identity),
            cancellation: CancellationToken::new(),
            execution_budget: Arc::new(ExecutionBudget::for_runtime_request(&request.extra)),
            telemetry,
            started_at: Instant::now(),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            cancel_event_emitted: Arc::new(AtomicBool::new(false)),
            observer: self.observer.clone(),
        };
        let activated = {
            let mut rows = self
                .supervisor
                .rows
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let matches = matches!(
                rows.get(&self.request_id),
                Some(RequestRow::Reserved(reserved))
                    if Arc::ptr_eq(&reserved.row_identity, &self.row_identity)
                        && reserved.observer.correlation() == self.observer.correlation()
            );
            if matches {
                rows.insert(self.request_id.clone(), RequestRow::Active(active.clone()));
                true
            } else {
                false
            }
        };
        if !activated {
            return None;
        }
        self.armed = false;
        Some(SupervisedRequest {
            request_id: self.request_id.clone(),
            active,
        })
    }
}

impl Drop for RequestReservation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut rows = self
            .supervisor
            .rows
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let matches = matches!(
            rows.get(&self.request_id),
            Some(RequestRow::Reserved(reserved))
                if Arc::ptr_eq(&reserved.row_identity, &self.row_identity)
        );
        if matches {
            rows.remove(&self.request_id);
        }
    }
}

impl CompletionWinner {
    fn into_cleanup_permit(self, response_owned: bool) -> CleanupPermit {
        CleanupPermit {
            supervisor: self.supervisor,
            request_id: self.request_id,
            row_identity: self.active.row_identity,
            observer: self.active.observer,
            response_owned,
        }
    }
}

impl CleanupPermit {
    pub(crate) fn response_owned(&self) -> bool {
        self.response_owned
    }

    /// Consumes the unique finalizer authority after request-local pins drop.
    ///
    /// The completing row is first replaced by an exact cleanup guard. Both
    /// transitions happen under the supervisor lock, while the observer call
    /// happens without it. A stale or dropped permit deliberately leaves the
    /// completing row occupied rather than releasing a possibly newer row.
    pub(crate) fn observe_cleanup(self) {
        let Some(guard) = self.begin_cleanup() else {
            return;
        };
        guard.observe_cleanup();
    }

    fn begin_cleanup(self) -> Option<CleanupGuard> {
        let CleanupPermit {
            supervisor,
            request_id,
            row_identity,
            observer,
            response_owned: _,
        } = self;
        let guard_identity = Arc::new(());
        {
            let mut rows = supervisor
                .rows
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let matches = matches!(
                rows.get(&request_id),
                Some(RequestRow::Completing(completing))
                    if Arc::ptr_eq(&completing.row_identity, &row_identity)
            );
            if !matches {
                return None;
            }
            rows.insert(
                request_id.clone(),
                RequestRow::Cleanup(CleanupRequest {
                    guard_identity: Arc::clone(&guard_identity),
                }),
            );
        }
        Some(CleanupGuard {
            supervisor,
            request_id,
            guard_identity,
            observer,
        })
    }
}

impl CleanupGuard {
    fn observe_cleanup(self) {
        self.observer
            .observe(BytecodeExecutionEvent::RequestCleanupComplete(
                RequestCleanupComplete {},
            ));
        self.finish();
    }

    fn finish(self) {
        let mut rows = self
            .supervisor
            .rows
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let matches = matches!(
            rows.get(&self.request_id),
            Some(RequestRow::Cleanup(cleanup))
                if Arc::ptr_eq(&cleanup.guard_identity, &self.guard_identity)
        );
        if matches {
            rows.remove(&self.request_id);
        }
    }
}

fn finish_cancelled_request(winner: CompletionWinner, trace: CompletionTrace) -> CleanupPermit {
    winner.active.cancel_requested.store(true, Ordering::SeqCst);
    winner.active.cancellation.cancel();
    winner.active.execution_budget.record_cancelled();
    winner.active.execution_budget.finish(Instant::now());
    observe_terminal(&winner.active, BytecodeRequestTerminal::Cancelled);
    let duration_ms = elapsed_ms(winner.active.started_at);
    if !winner
        .active
        .cancel_event_emitted
        .swap(true, Ordering::SeqCst)
    {
        winner.active.telemetry.emit_trace(
            "request.cancel",
            trace.include_duration.then_some(duration_ms),
            None,
            budget_attrs(&winner.active, duration_ms, trace),
        );
        emit_request_duration_metric(&winner.active, duration_ms, "cancel");
    }
    winner.into_cleanup_permit(false)
}

fn observe_terminal(active: &ActiveRequest, terminal: BytecodeRequestTerminal) {
    active
        .observer
        .observe(BytecodeExecutionEvent::RequestTerminalClaimed(
            RequestTerminalClaimed { terminal },
        ));
}

fn emit_request_duration_metric(active: &ActiveRequest, duration_ms: f64, outcome: &str) {
    let mut attrs = Map::new();
    attrs.insert("durationMs".to_string(), Value::from(duration_ms));
    attrs.insert("outcome".to_string(), Value::String(outcome.to_string()));
    active
        .telemetry
        .emit_duration_metric("request.duration", Some(attrs));
}

impl SupervisedRequest {
    pub(crate) fn cancelled(&self) -> Arc<AtomicBool> {
        self.active.cancellation.cancel_flag()
    }

    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.active.cancellation.clone()
    }

    pub(crate) fn execution_budget(&self) -> Arc<ExecutionBudget> {
        self.active.execution_budget.clone()
    }
}

fn budget_attrs(
    active: &ActiveRequest,
    duration_ms: f64,
    trace: CompletionTrace,
) -> Option<Map<String, Value>> {
    trace
        .include_budget_attrs
        .then(|| execution_budget_trace_attrs(&active.execution_budget, duration_ms))
}

fn elapsed_ms(started_at: Instant) -> f64 {
    started_at.elapsed().as_secs_f64() * 1000.0
}

fn fixed_service_failure_telemetry_map(error: &OpaqueServiceError) -> Map<String, Value> {
    let cause_kind = match error.envelope() {
        ServiceErrorEnvelope::PublicTypedError { .. } => "publicTypedError",
        ServiceErrorEnvelope::InternalError { .. } => "internalError",
        ServiceErrorEnvelope::PlatformError { .. } => "platformError",
    };
    Map::from_iter([
        (
            "kind".to_string(),
            Value::String("fixedService".to_string()),
        ),
        (
            "causeKind".to_string(),
            Value::String(cause_kind.to_string()),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_runtime_model::bytecode_execution_observation::{
        BytecodeExecutionCorrelation, BytecodeExecutionEventSink, BytecodeExecutionObservation,
    };
    use std::sync::{
        atomic::AtomicUsize,
        mpsc::{channel, Receiver},
    };

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<BytecodeExecutionObservation>>);

    impl BytecodeExecutionEventSink for RecordingSink {
        fn observe(&self, observation: BytecodeExecutionObservation) {
            self.0.lock().unwrap().push(observation);
        }
    }

    struct LifecycleBlockingSink {
        records: Mutex<Vec<BytecodeExecutionObservation>>,
        callbacks: AtomicUsize,
        overlap: AtomicBool,
        block_terminal: AtomicBool,
        terminal_entered: tokio::sync::mpsc::UnboundedSender<()>,
        terminal_release: Mutex<Receiver<()>>,
        block_cleanup: AtomicBool,
        cleanup_entered: tokio::sync::mpsc::UnboundedSender<()>,
        cleanup_release: Mutex<Receiver<()>>,
    }

    impl BytecodeExecutionEventSink for LifecycleBlockingSink {
        fn observe(&self, observation: BytecodeExecutionObservation) {
            if self.callbacks.fetch_add(1, Ordering::SeqCst) != 0 {
                self.overlap.store(true, Ordering::SeqCst);
            }
            self.records.lock().unwrap().push(observation.clone());
            match observation.event {
                BytecodeExecutionEvent::RequestTerminalClaimed(_)
                    if self.block_terminal.swap(false, Ordering::SeqCst) =>
                {
                    let _ = self.terminal_entered.send(());
                    let _ = self.terminal_release.lock().unwrap().recv();
                }
                BytecodeExecutionEvent::RequestCleanupComplete(_)
                    if self.block_cleanup.swap(false, Ordering::SeqCst) =>
                {
                    let _ = self.cleanup_entered.send(());
                    let _ = self.cleanup_release.lock().unwrap().recv();
                }
                _ => {}
            }
            self.callbacks.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn request(request_id: &str) -> RequestEnvelope {
        RequestEnvelope {
            request_id: request_id.to_string(),
            mode: "unary".to_string(),
            target: "run".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: None,
            build_id: "build".to_string(),
            service_protocol_identity: "protocol".to_string(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: None,
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: Default::default(),
            payload_bytes: Vec::new(),
            extra: Map::new(),
        }
    }

    fn observer<S>(sink: Arc<S>, request_id: &str) -> BytecodeExecutionObserver
    where
        S: BytecodeExecutionEventSink,
    {
        BytecodeExecutionObserver::new(
            sink,
            BytecodeExecutionCorrelation {
                router_session_id: "session".to_string(),
                request_id: request_id.to_string(),
            },
        )
    }

    fn telemetry() -> RequestTelemetryContext {
        use super::super::telemetry::{TelemetryConfig, TelemetryProducer};
        RequestTelemetryContext::new(TelemetryProducer::new(TelemetryConfig::for_test(
            "request-supervisor-test",
        )))
    }

    #[tokio::test]
    async fn reservation_is_insert_if_vacant_and_drop_removes_only_reserved_row() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let reservation = supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .expect("first reservation");
        assert!(supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .is_none());
        assert_eq!(supervisor.active_count().await, 0);
        drop(reservation);
        assert!(supervisor
            .reserve("request".to_string(), observer(sink, "request"))
            .is_some());
    }

    #[tokio::test]
    async fn complete_error_mints_failed_once_and_returns_cleanup_only_to_winner() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let reservation = supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .expect("reservation");
        let supervised = reservation
            .activate(&request("request"), telemetry())
            .expect("activation");
        let error = ResponseError {
            code: "InternalError".to_string(),
            message: "json-rpc failure".to_string(),
            status: None,
            details: None,
        };

        let permit = supervisor
            .complete_error(
                &supervised,
                "request.error",
                &error,
                CompletionTrace::RUNTIME,
            )
            .await
            .expect("winner cleanup permit");
        assert!(permit.response_owned());
        assert!(supervisor
            .complete_success(&supervised, CompletionTrace::RUNTIME)
            .await
            .is_none());
        let records = sink.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert!(matches!(
            records[0].event,
            BytecodeExecutionEvent::RequestTerminalClaimed(RequestTerminalClaimed {
                terminal: BytecodeRequestTerminal::Failed
            })
        ));
        drop(records);
        drop(permit);
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        assert!(supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .is_none());
        assert!(
            !supervisor
                .cancel(&RequestCancel {
                    request_id: "request".to_string(),
                    reason: None,
                })
                .await
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn request_id_stays_guarded_through_terminal_and_cleanup_observers() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let (terminal_entered_tx, mut terminal_entered_rx) = tokio::sync::mpsc::unbounded_channel();
        let (terminal_release_tx, terminal_release_rx) = channel();
        let (cleanup_entered_tx, mut cleanup_entered_rx) = tokio::sync::mpsc::unbounded_channel();
        let (cleanup_release_tx, cleanup_release_rx) = channel();
        let sink = Arc::new(LifecycleBlockingSink {
            records: Mutex::new(Vec::new()),
            callbacks: AtomicUsize::new(0),
            overlap: AtomicBool::new(false),
            block_terminal: AtomicBool::new(true),
            terminal_entered: terminal_entered_tx,
            terminal_release: Mutex::new(terminal_release_rx),
            block_cleanup: AtomicBool::new(true),
            cleanup_entered: cleanup_entered_tx,
            cleanup_release: Mutex::new(cleanup_release_rx),
        });
        let reservation = supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .expect("first reservation");
        let supervised = reservation
            .activate(&request("request"), telemetry())
            .expect("first activation");

        let completing_supervisor = Arc::clone(&supervisor);
        let completing_request = supervised.clone();
        let completing = tokio::spawn(async move {
            completing_supervisor
                .complete_success(&completing_request, CompletionTrace::RUNTIME)
                .await
                .expect("winner cleanup permit")
        });
        terminal_entered_rx
            .recv()
            .await
            .expect("terminal observer entered");

        assert!(supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .is_none());
        assert!(
            !supervisor
                .cancel(&RequestCancel {
                    request_id: "request".to_string(),
                    reason: Some("late cancel".to_string()),
                })
                .await
        );
        assert_eq!(supervisor.active_count().await, 0);

        terminal_release_tx
            .send(())
            .expect("release terminal observer");
        let permit = completing.await.expect("completion task");
        assert!(permit.response_owned());
        assert!(supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .is_none());
        assert!(
            !supervisor
                .cancel(&RequestCancel {
                    request_id: "request".to_string(),
                    reason: None,
                })
                .await
        );

        let cleaning = tokio::task::spawn_blocking(move || permit.observe_cleanup());
        cleanup_entered_rx
            .recv()
            .await
            .expect("cleanup observer entered");
        assert!(supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .is_none());
        assert!(
            !supervisor
                .cancel(&RequestCancel {
                    request_id: "request".to_string(),
                    reason: None,
                })
                .await
        );

        cleanup_release_tx
            .send(())
            .expect("release cleanup observer");
        cleaning.await.expect("cleanup task");

        let next_reservation = supervisor
            .reserve("request".to_string(), observer(sink.clone(), "request"))
            .expect("reuse only after cleanup returned");
        let next_supervised = next_reservation
            .activate(&request("request"), telemetry())
            .expect("next activation");
        let next_permit = supervisor
            .complete_success(&next_supervised, CompletionTrace::RUNTIME)
            .await
            .expect("next completion winner");
        drop(next_supervised);
        next_permit.observe_cleanup();

        let records = sink.records.lock().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.ordinal)
                .collect::<Vec<_>>(),
            [0, 1, 0, 1]
        );
        assert!(matches!(
            records[0].event,
            BytecodeExecutionEvent::RequestTerminalClaimed(_)
        ));
        assert!(matches!(
            records[1].event,
            BytecodeExecutionEvent::RequestCleanupComplete(_)
        ));
        assert!(records.iter().all(|record| {
            record.correlation.router_session_id == "session"
                && record.correlation.request_id == "request"
        }));
        assert!(!sink.overlap.load(Ordering::SeqCst));
        assert_eq!(sink.callbacks.load(Ordering::SeqCst), 0);
    }
}
