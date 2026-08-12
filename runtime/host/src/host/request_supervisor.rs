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
    RequestTerminalClaimed,
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

enum RequestRow {
    Reserved(ReservedRequest),
    Active(ActiveRequest),
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

/// Uncloneable proof that this lane won removal of the matching active row.
struct CompletionWinner {
    active: ActiveRequest,
    cancelled: bool,
}

/// Uncloneable authority for the request-task finalizer to mint cleanup.
pub(crate) struct CleanupPermit {
    observer: BytecodeExecutionObserver,
    response_owned: bool,
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
        &self,
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
        Some(CleanupPermit {
            observer: winner.active.observer,
            response_owned: true,
        })
    }

    pub(crate) async fn complete_error(
        &self,
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
        Some(CleanupPermit {
            observer: winner.active.observer,
            response_owned: true,
        })
    }

    pub(crate) async fn complete_fixed_service_failure(
        &self,
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
        Some(CleanupPermit {
            observer: winner.active.observer,
            response_owned: true,
        })
    }

    pub(crate) async fn complete_cancelled(
        &self,
        request: &SupervisedRequest,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        self.claim_completion(request)
            .map(|winner| finish_cancelled_request(winner, trace))
    }

    pub(crate) async fn cancel(&self, cancel: &RequestCancel) -> bool {
        let active = {
            let rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
            let Some(RequestRow::Active(active)) = rows.get(&cancel.request_id) else {
                return false;
            };
            active.clone()
        };

        active.cancel_requested.store(true, Ordering::SeqCst);
        active.cancellation.cancel();
        active.execution_budget.record_cancelled();
        let duration_ms = elapsed_ms(active.started_at);
        if !active.cancel_event_emitted.swap(true, Ordering::SeqCst) {
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

    fn claim_completion(&self, request: &SupervisedRequest) -> Option<CompletionWinner> {
        let mut rows = self.rows.lock().unwrap_or_else(|error| error.into_inner());
        let matches = matches!(
            rows.get(&request.request_id),
            Some(RequestRow::Active(active))
                if Arc::ptr_eq(&active.row_identity, &request.active.row_identity)
        );
        if !matches {
            return None;
        }
        let RequestRow::Active(active) = rows
            .remove(&request.request_id)
            .expect("matching active row exists")
        else {
            unreachable!("matching row was checked as active")
        };
        // Only an explicit root cancellation may change an ordinary
        // success/error completion into the cancellation terminal.
        let cancelled = active.cancel_requested.load(Ordering::SeqCst);
        Some(CompletionWinner { active, cancelled })
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

impl CleanupPermit {
    pub(crate) fn response_owned(&self) -> bool {
        self.response_owned
    }

    pub(crate) fn into_observer(self) -> BytecodeExecutionObserver {
        self.observer
    }
}

fn finish_cancelled_request(
    winner: CompletionWinner,
    trace: CompletionTrace,
) -> CleanupPermit {
    winner
        .active
        .cancel_requested
        .store(true, Ordering::SeqCst);
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
    CleanupPermit {
        observer: winner.active.observer,
        response_owned: false,
    }
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

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<BytecodeExecutionObservation>>);

    impl BytecodeExecutionEventSink for RecordingSink {
        fn observe(&self, observation: BytecodeExecutionObservation) {
            self.0.lock().unwrap().push(observation);
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

    fn observer(
        sink: Arc<RecordingSink>,
        request_id: &str,
    ) -> BytecodeExecutionObserver {
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
    }
}
