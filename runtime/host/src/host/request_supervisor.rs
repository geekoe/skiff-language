use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use serde_json::{Map, Value};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionEvent, BytecodeExecutionObserver, BytecodeRequestTerminal,
    RequestCleanupComplete, RequestTerminalClaimed, VmBudgetAccounted,
};
use skiff_runtime_model::service_error::{
    ErrorCorrelation, OpaqueServiceError, ServiceErrorEnvelope,
};
#[cfg(test)]
use skiff_runtime_request::execution_budget::RequestPendingSink;
use skiff_runtime_request::{
    cancellation::CancellationToken,
    execution_budget::{
        AdmittedRequestDeadline, CompletionCandidate, ExecutionBudget, ExecutionSettlement,
        ExecutionWinner,
    },
    execution_budget_trace_attrs, response_error_to_telemetry_map, RequestCancel, RequestEnvelope,
    RequestExecutionOwnerInventorySnapshot, ResponseError,
};

use crate::telemetry::RequestTelemetryContext;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RouterSessionEpoch(String);

impl RouterSessionEpoch {
    pub(crate) fn from_connection_id(value: String) -> Result<Self, RequestIdentityError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(RequestIdentityError::InvalidRouterSessionEpoch);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RequestId(String);

impl RequestId {
    pub(crate) fn parse(value: String) -> Result<Self, RequestIdentityError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(RequestIdentityError::InvalidRequestId);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestIdentityError {
    InvalidRouterSessionEpoch,
    InvalidRequestId,
}

impl fmt::Display for RequestIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRouterSessionEpoch => "invalid router session epoch",
            Self::InvalidRequestId => "invalid bytecode request id",
        })
    }
}

impl std::error::Error for RequestIdentityError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RequestExecutionKey {
    router_session: RouterSessionEpoch,
    request_id: RequestId,
}

impl RequestExecutionKey {
    pub(crate) const fn new(router_session: RouterSessionEpoch, request_id: RequestId) -> Self {
        Self {
            router_session,
            request_id,
        }
    }

    pub(crate) fn router_session(&self) -> &RouterSessionEpoch {
        &self.router_session
    }

    pub(crate) fn request_id(&self) -> &RequestId {
        &self.request_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReservationRevocation {
    Cancel,
    SessionStop,
}

#[derive(Clone)]
struct ReservedRequest {
    row_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
}

struct RevokedRequest {
    row_identity: Arc<()>,
    revocation: ReservationRevocation,
}

#[derive(Clone)]
struct ActiveRequest {
    row_identity: Arc<()>,
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    telemetry: RequestTelemetryContext,
    started_at: Instant,
    cancel_event_emitted: Arc<AtomicBool>,
    cancel_received: Arc<AtomicBool>,
    observer: BytecodeExecutionObserver,
}

struct CompletingRequest {
    row_identity: Arc<()>,
    settlement: Arc<ExecutionSettlement>,
    owner_inventory: RequestExecutionOwnerInventorySnapshot,
}

struct CleanupRequest {
    guard_identity: Arc<()>,
}

enum RequestRow {
    Reserved(ReservedRequest),
    Revoked(RevokedRequest),
    Active(ActiveRequest),
    Completing(CompletingRequest),
    Cleanup(CleanupRequest),
}

#[derive(Default)]
struct SupervisorState {
    rows: HashMap<RequestExecutionKey, RequestRow>,
    open_sessions: HashSet<RouterSessionEpoch>,
}

#[derive(Clone)]
pub(crate) struct SupervisedRequest {
    key: RequestExecutionKey,
    active: ActiveRequest,
}

/// RAII ownership of one exact reserved row during fallible admission.
pub(crate) struct RequestReservation {
    supervisor: Arc<RequestSupervisor>,
    key: RequestExecutionKey,
    row_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
    admitted_deadline: Option<AdmittedRequestDeadline>,
    armed: bool,
}

pub(crate) enum ActivationOutcome {
    Activated(SupervisedRequest),
    RevokedByCancel,
    RevokedBySessionStop,
    Invalid,
}

struct CompletionWinner {
    supervisor: Arc<RequestSupervisor>,
    key: RequestExecutionKey,
    active: ActiveRequest,
    settlement: Arc<ExecutionSettlement>,
    owner_inventory: RequestExecutionOwnerInventorySnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CompletionResponseAction {
    Candidate,
    DeadlineExceeded {
        instruction_count: u64,
        limit: u64,
        elapsed_ms: f64,
    },
    InstructionLimitExceeded {
        instruction_count: u64,
        limit: u64,
    },
    AccountingFailure,
    StopWithoutResponse,
}

/// Uncloneable authority for the request-task finalizer to mint cleanup.
pub(crate) struct CleanupPermit {
    supervisor: Arc<RequestSupervisor>,
    key: RequestExecutionKey,
    row_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
    settlement: Arc<ExecutionSettlement>,
    response_action: CompletionResponseAction,
    owner_inventory: RequestExecutionOwnerInventorySnapshot,
}

struct CleanupGuard {
    supervisor: Arc<RequestSupervisor>,
    key: RequestExecutionKey,
    guard_identity: Arc<()>,
    observer: BytecodeExecutionObserver,
    _settlement: Arc<ExecutionSettlement>,
    owner_inventory: RequestExecutionOwnerInventorySnapshot,
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
    state: Mutex<SupervisorState>,
}

impl RequestSupervisor {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn start_session(&self, router_session: RouterSessionEpoch) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .open_sessions
            .insert(router_session)
    }

    pub(crate) fn reserve(
        self: &Arc<Self>,
        key: RequestExecutionKey,
        observer: BytecodeExecutionObserver,
        admitted_deadline: Option<AdmittedRequestDeadline>,
    ) -> Option<RequestReservation> {
        if observer.correlation().router_session_id != key.router_session.as_str()
            || observer.correlation().request_id != key.request_id.as_str()
        {
            return None;
        }
        let row_identity = Arc::new(());
        let reserved = ReservedRequest {
            row_identity: Arc::clone(&row_identity),
            observer: observer.clone(),
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.open_sessions.contains(&key.router_session) {
            return None;
        }
        match state.rows.entry(key.clone()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                entry.insert(RequestRow::Reserved(reserved));
                Some(RequestReservation {
                    supervisor: Arc::clone(self),
                    key,
                    row_identity,
                    observer,
                    admitted_deadline,
                    armed: true,
                })
            }
        }
    }

    pub(crate) async fn complete_success(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        let winner =
            self.claim_completion(request, owner_inventory, CompletionCandidate::Success)?;
        finish_completion(winner, trace, None, None)
    }

    pub(crate) async fn complete_error(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        event_name: &'static str,
        error: &ResponseError,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        let winner =
            self.claim_completion(request, owner_inventory, CompletionCandidate::Failure)?;
        finish_completion(
            winner,
            trace,
            Some(event_name),
            Some(response_error_to_telemetry_map(error)),
        )
    }

    pub(crate) async fn complete_fixed_service_failure(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        event_name: &'static str,
        error: &OpaqueServiceError,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        let winner =
            self.claim_completion(request, owner_inventory, CompletionCandidate::Failure)?;
        let correlation = ErrorCorrelation {
            trace_id: error.envelope().trace_id().to_string(),
            error_id: error.envelope().error_id().to_string(),
        };
        let duration_ms = elapsed_ms(winner.active.started_at);
        winner.active.telemetry.emit_trace_with_error_correlation(
            event_name,
            trace.include_duration.then_some(duration_ms),
            Some(fixed_service_failure_telemetry_map(error)),
            budget_attrs(&winner.active, duration_ms, trace),
            &correlation,
        );
        finish_completion(winner, trace, None, None)
    }

    pub(crate) async fn complete_cancelled(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        trace: CompletionTrace,
    ) -> Option<CleanupPermit> {
        let winner =
            self.claim_completion(request, owner_inventory, CompletionCandidate::Failure)?;
        finish_completion(winner, trace, None, None)
    }

    pub(crate) async fn cancel(
        &self,
        router_session: &RouterSessionEpoch,
        cancel: &RequestCancel,
    ) -> bool {
        let Ok(request_id) = RequestId::parse(cancel.request_id.clone()) else {
            return false;
        };
        let key = RequestExecutionKey::new(router_session.clone(), request_id);
        let mut active_to_wake = None;
        let handled = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(row) = state.rows.get_mut(&key) else {
                return false;
            };
            match row {
                RequestRow::Reserved(reserved) => {
                    *row = RequestRow::Revoked(RevokedRequest {
                        row_identity: Arc::clone(&reserved.row_identity),
                        revocation: ReservationRevocation::Cancel,
                    });
                    true
                }
                RequestRow::Revoked(revoked) => revoked.revocation == ReservationRevocation::Cancel,
                RequestRow::Active(active) => {
                    let settlement = active.execution_budget.request_cancel().into_settlement();
                    active.cancel_received.store(true, Ordering::SeqCst);
                    active_to_wake = Some((
                        active.clone(),
                        settlement.winner() == ExecutionWinner::Cancelled,
                    ));
                    true
                }
                RequestRow::Completing(_) | RequestRow::Cleanup(_) => false,
            }
        };

        if let Some((active, cancellation_won)) = active_to_wake {
            active.cancellation.cancel();
            let duration_ms = elapsed_ms(active.started_at);
            if cancellation_won && !active.cancel_event_emitted.swap(true, Ordering::SeqCst) {
                let mut attrs = execution_budget_trace_attrs(&active.execution_budget, duration_ms);
                if let Some(reason) = cancel.reason.as_deref() {
                    attrs.insert("reason".to_string(), Value::String(reason.to_string()));
                }
                active
                    .telemetry
                    .emit_trace("request.cancel", Some(duration_ms), None, Some(attrs));
                emit_request_duration_metric(&active, duration_ms, "cancel");
            }
        }
        handled
    }

    pub(crate) fn stop_session(&self, router_session: &RouterSessionEpoch) {
        let active_to_wake = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.open_sessions.remove(router_session);
            let matching_keys = state
                .rows
                .keys()
                .filter(|key| key.router_session() == router_session)
                .cloned()
                .collect::<Vec<_>>();
            let mut active_to_wake = Vec::new();
            for key in matching_keys {
                let Some(row) = state.rows.get_mut(&key) else {
                    continue;
                };
                match row {
                    RequestRow::Reserved(reserved) => {
                        *row = RequestRow::Revoked(RevokedRequest {
                            row_identity: Arc::clone(&reserved.row_identity),
                            revocation: ReservationRevocation::SessionStop,
                        });
                    }
                    RequestRow::Revoked(_) => {}
                    RequestRow::Active(active) => {
                        let _ = active.execution_budget.request_internal_stop();
                        active_to_wake.push(active.clone());
                    }
                    RequestRow::Completing(_) | RequestRow::Cleanup(_) => {}
                }
            }
            active_to_wake
        };
        for active in active_to_wake {
            active.cancellation.cancel();
        }
    }

    pub(crate) async fn active_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .rows
            .values()
            .filter(|row| matches!(row, RequestRow::Active(_)))
            .count()
    }

    fn claim_completion(
        self: &Arc<Self>,
        request: &SupervisedRequest,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        candidate: CompletionCandidate,
    ) -> Option<CompletionWinner> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Entry::Occupied(mut entry) = state.rows.entry(request.key.clone()) else {
            return None;
        };
        let RequestRow::Active(current) = entry.get() else {
            return None;
        };
        if !Arc::ptr_eq(&current.row_identity, &request.active.row_identity) {
            return None;
        }
        let settlement = current.execution_budget.settle(candidate).into_settlement();
        let completing = CompletingRequest {
            row_identity: Arc::clone(&current.row_identity),
            settlement: Arc::clone(&settlement),
            owner_inventory,
        };
        let RequestRow::Active(active) = entry.insert(RequestRow::Completing(completing)) else {
            unreachable!("matching active row was replaced")
        };
        Some(CompletionWinner {
            supervisor: Arc::clone(self),
            key: request.key.clone(),
            active,
            settlement,
            owner_inventory,
        })
    }
}

impl RequestReservation {
    pub(crate) fn observer(&self) -> &BytecodeExecutionObserver {
        &self.observer
    }

    pub(crate) fn key(&self) -> &RequestExecutionKey {
        &self.key
    }

    pub(crate) fn activate(
        mut self,
        exact_key: &RequestExecutionKey,
        request: &RequestEnvelope,
        telemetry: RequestTelemetryContext,
    ) -> ActivationOutcome {
        if exact_key != &self.key || request.request_id != self.key.request_id.as_str() {
            return ActivationOutcome::Invalid;
        }
        let outcome = {
            let mut state = self
                .supervisor
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(row) = state.rows.get(&self.key) else {
                return ActivationOutcome::Invalid;
            };
            match row {
                RequestRow::Reserved(reserved)
                    if Arc::ptr_eq(&reserved.row_identity, &self.row_identity)
                        && reserved.observer.correlation() == self.observer.correlation() =>
                {
                    let active = ActiveRequest {
                        row_identity: Arc::clone(&self.row_identity),
                        cancellation: CancellationToken::new(),
                        execution_budget: Arc::new(ExecutionBudget::for_runtime_request(
                            self.admitted_deadline,
                        )),
                        telemetry,
                        started_at: Instant::now(),
                        cancel_event_emitted: Arc::new(AtomicBool::new(false)),
                        cancel_received: Arc::new(AtomicBool::new(false)),
                        observer: self.observer.clone(),
                    };
                    state
                        .rows
                        .insert(self.key.clone(), RequestRow::Active(active.clone()));
                    ActivationOutcome::Activated(SupervisedRequest {
                        key: self.key.clone(),
                        active,
                    })
                }
                RequestRow::Revoked(revoked)
                    if Arc::ptr_eq(&revoked.row_identity, &self.row_identity) =>
                {
                    let revocation = revoked.revocation;
                    state.rows.remove(&self.key);
                    match revocation {
                        ReservationRevocation::Cancel => ActivationOutcome::RevokedByCancel,
                        ReservationRevocation::SessionStop => {
                            ActivationOutcome::RevokedBySessionStop
                        }
                    }
                }
                RequestRow::Reserved(_)
                | RequestRow::Revoked(_)
                | RequestRow::Active(_)
                | RequestRow::Completing(_)
                | RequestRow::Cleanup(_) => ActivationOutcome::Invalid,
            }
        };
        self.armed = false;
        outcome
    }
}

impl Drop for RequestReservation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = self
            .supervisor
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let matches = match state.rows.get(&self.key) {
            Some(RequestRow::Reserved(reserved)) => {
                Arc::ptr_eq(&reserved.row_identity, &self.row_identity)
            }
            Some(RequestRow::Revoked(revoked)) => {
                Arc::ptr_eq(&revoked.row_identity, &self.row_identity)
            }
            Some(RequestRow::Active(_) | RequestRow::Completing(_) | RequestRow::Cleanup(_))
            | None => false,
        };
        if matches {
            state.rows.remove(&self.key);
        }
    }
}

impl CleanupPermit {
    pub(crate) const fn response_action(&self) -> CompletionResponseAction {
        self.response_action
    }

    pub(crate) const fn response_owned(&self) -> bool {
        !matches!(
            self.response_action,
            CompletionResponseAction::StopWithoutResponse
        )
    }

    pub(crate) fn response_override(&self) -> Option<ResponseError> {
        match self.response_action {
            CompletionResponseAction::Candidate | CompletionResponseAction::StopWithoutResponse => {
                None
            }
            CompletionResponseAction::DeadlineExceeded {
                instruction_count,
                limit,
                elapsed_ms,
            } => Some(ResponseError {
                code: "TimeoutError".to_string(),
                message: "execution deadline exceeded".to_string(),
                status: None,
                details: Some(serde_json::json!({
                    "reason": "deadlineExceeded",
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                })),
            }),
            CompletionResponseAction::InstructionLimitExceeded {
                instruction_count,
                limit,
            } => Some(ResponseError {
                code: "std.error.InstructionLimitExceededError".to_string(),
                message: "execution instruction limit exceeded".to_string(),
                status: None,
                details: Some(serde_json::json!({
                    "instructionCount": instruction_count,
                    "limit": limit,
                })),
            }),
            CompletionResponseAction::AccountingFailure => Some(ResponseError {
                code: "InternalError".to_string(),
                message: "bytecode execution failed".to_string(),
                status: None,
                details: None,
            }),
        }
    }

    pub(crate) fn settlement(&self) -> &Arc<ExecutionSettlement> {
        &self.settlement
    }

    pub(crate) fn observe_cleanup(self) {
        let Some(guard) = self.begin_cleanup() else {
            return;
        };
        guard.observe_cleanup();
    }

    fn begin_cleanup(self) -> Option<CleanupGuard> {
        let Self {
            supervisor,
            key,
            row_identity,
            observer,
            settlement,
            response_action: _,
            owner_inventory,
        } = self;
        let guard_identity = Arc::new(());
        {
            let mut state = supervisor
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let matches = matches!(
                state.rows.get(&key),
                Some(RequestRow::Completing(completing))
                    if Arc::ptr_eq(&completing.row_identity, &row_identity)
                        && Arc::ptr_eq(&completing.settlement, &settlement)
                        && completing.owner_inventory == owner_inventory
            );
            if !matches {
                return None;
            }
            state.rows.insert(
                key.clone(),
                RequestRow::Cleanup(CleanupRequest {
                    guard_identity: Arc::clone(&guard_identity),
                }),
            );
        }
        Some(CleanupGuard {
            supervisor,
            key,
            guard_identity,
            observer,
            _settlement: settlement,
            owner_inventory,
        })
    }
}

impl CleanupGuard {
    fn observe_cleanup(self) {
        self.observer
            .observe(BytecodeExecutionEvent::RequestCleanupComplete(
                RequestCleanupComplete {
                    owner_inventory: self.owner_inventory,
                },
            ));
        self.finish();
    }

    fn finish(self) {
        let mut state = self
            .supervisor
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let matches = matches!(
            state.rows.get(&self.key),
            Some(RequestRow::Cleanup(cleanup))
                if Arc::ptr_eq(&cleanup.guard_identity, &self.guard_identity)
        );
        if matches {
            state.rows.remove(&self.key);
        }
    }
}

impl SupervisedRequest {
    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.active.cancellation.clone()
    }

    pub(crate) fn execution_budget(&self) -> Arc<ExecutionBudget> {
        self.active.execution_budget.clone()
    }
}

fn finish_completion(
    winner: CompletionWinner,
    trace: CompletionTrace,
    event_name: Option<&'static str>,
    error_attrs: Option<Map<String, Value>>,
) -> Option<CleanupPermit> {
    let terminal = terminal_for_winner(winner.settlement.winner());
    winner
        .active
        .observer
        .observe(BytecodeExecutionEvent::VmBudgetAccounted(
            VmBudgetAccounted {
                raw_executed_count: winner.settlement.raw_executed_count(),
                charged_instruction_count: winner.settlement.raw_executed_count(),
                hard_limit: winner.settlement.hard_raw_limit(),
                poll_count: winner.settlement.poll_count(),
            },
        ));
    observe_terminal(&winner.active, terminal);
    let duration_ms = elapsed_ms(winner.active.started_at);
    let response_action = if winner.active.cancel_received.load(Ordering::SeqCst) {
        CompletionResponseAction::StopWithoutResponse
    } else {
        response_action(&winner.settlement)
    };

    match winner.settlement.winner() {
        ExecutionWinner::Succeeded => {
            emit_request_duration_metric(&winner.active, duration_ms, "ok");
        }
        ExecutionWinner::Cancelled | ExecutionWinner::InternalStop => {
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
        }
        ExecutionWinner::Failed
        | ExecutionWinner::DeadlineExceeded
        | ExecutionWinner::InstructionLimitExceeded
        | ExecutionWinner::AccountingFailure => {
            if let Some(event_name) = event_name {
                winner.active.telemetry.emit_trace(
                    event_name,
                    trace.include_duration.then_some(duration_ms),
                    error_attrs,
                    budget_attrs(&winner.active, duration_ms, trace),
                );
            }
            emit_request_duration_metric(&winner.active, duration_ms, "error");
        }
    }
    Some(winner.into_cleanup_permit(response_action))
}

impl CompletionWinner {
    fn into_cleanup_permit(self, response_action: CompletionResponseAction) -> CleanupPermit {
        CleanupPermit {
            supervisor: self.supervisor,
            key: self.key,
            row_identity: self.active.row_identity,
            observer: self.active.observer,
            settlement: self.settlement,
            response_action,
            owner_inventory: self.owner_inventory,
        }
    }
}

fn terminal_for_winner(winner: ExecutionWinner) -> BytecodeRequestTerminal {
    match winner {
        ExecutionWinner::Succeeded => BytecodeRequestTerminal::Succeeded,
        ExecutionWinner::Cancelled | ExecutionWinner::InternalStop => {
            BytecodeRequestTerminal::Cancelled
        }
        ExecutionWinner::Failed
        | ExecutionWinner::DeadlineExceeded
        | ExecutionWinner::InstructionLimitExceeded
        | ExecutionWinner::AccountingFailure => BytecodeRequestTerminal::Failed,
    }
}

fn response_action(settlement: &ExecutionSettlement) -> CompletionResponseAction {
    match settlement.winner() {
        ExecutionWinner::Succeeded | ExecutionWinner::Failed => CompletionResponseAction::Candidate,
        ExecutionWinner::Cancelled | ExecutionWinner::InternalStop => {
            CompletionResponseAction::StopWithoutResponse
        }
        ExecutionWinner::DeadlineExceeded => CompletionResponseAction::DeadlineExceeded {
            instruction_count: settlement.raw_executed_count(),
            limit: settlement.hard_raw_limit(),
            elapsed_ms: settlement.elapsed().as_secs_f64() * 1000.0,
        },
        ExecutionWinner::InstructionLimitExceeded => {
            CompletionResponseAction::InstructionLimitExceeded {
                instruction_count: settlement.raw_executed_count(),
                limit: settlement.hard_raw_limit(),
            }
        }
        ExecutionWinner::AccountingFailure => CompletionResponseAction::AccountingFailure,
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
    use skiff_runtime_request::FrozenOwnerDomain;
    use std::sync::{
        atomic::AtomicUsize,
        mpsc::{channel, Receiver},
    };
    use tokio::sync::mpsc::UnboundedSender;

    fn zero_snapshot() -> RequestExecutionOwnerInventorySnapshot {
        RequestExecutionOwnerInventorySnapshot {
            pending: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            resource: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            child: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            child_heap: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            boundary: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            actor: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
        }
    }

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<BytecodeExecutionObservation>>);

    impl BytecodeExecutionEventSink for RecordingSink {
        fn observe(&self, observation: BytecodeExecutionObservation) {
            self.0.lock().unwrap().push(observation);
        }
    }

    /// Sink that blocks the inline drainer on the terminal and cleanup
    /// callbacks while proving callbacks never overlap. `VmBudgetAccounted`
    /// passes through unblocked.
    struct LifecycleBlockingSink {
        records: Mutex<Vec<BytecodeExecutionObservation>>,
        callbacks: AtomicUsize,
        overlap: AtomicBool,
        block_terminal: AtomicBool,
        terminal_entered: UnboundedSender<()>,
        terminal_release: Mutex<Receiver<()>>,
        block_cleanup: AtomicBool,
        cleanup_entered: UnboundedSender<()>,
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

    fn epoch(id: &str) -> RouterSessionEpoch {
        RouterSessionEpoch::from_connection_id(id.to_string()).unwrap()
    }

    fn key(session: &str, request: &str) -> RequestExecutionKey {
        RequestExecutionKey::new(
            epoch(session),
            RequestId::parse(request.to_string()).unwrap(),
        )
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

    fn observer<S>(sink: Arc<S>, key: &RequestExecutionKey) -> BytecodeExecutionObserver
    where
        S: BytecodeExecutionEventSink,
    {
        BytecodeExecutionObserver::new(
            sink,
            BytecodeExecutionCorrelation {
                router_session_id: key.router_session().as_str().to_string(),
                request_id: key.request_id().as_str().to_string(),
            },
        )
    }

    fn telemetry() -> RequestTelemetryContext {
        use super::super::telemetry::{TelemetryConfig, TelemetryProducer};
        RequestTelemetryContext::new(TelemetryProducer::new(TelemetryConfig::for_test(
            "request-supervisor-test",
        )))
    }

    fn activate(reservation: RequestReservation, key: &RequestExecutionKey) -> SupervisedRequest {
        match reservation.activate(key, &request(key.request_id().as_str()), telemetry()) {
            ActivationOutcome::Activated(request) => request,
            _ => panic!("expected activation"),
        }
    }

    #[tokio::test]
    async fn equal_request_ids_are_independent_across_session_epochs() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let a = key("session-a", "same");
        let b = key("session-b", "same");
        assert!(supervisor.start_session(a.router_session().clone()));
        assert!(supervisor.start_session(b.router_session().clone()));
        let a_request = activate(
            supervisor
                .reserve(a.clone(), observer(sink.clone(), &a), None)
                .unwrap(),
            &a,
        );
        let b_request = activate(
            supervisor
                .reserve(b.clone(), observer(sink.clone(), &b), None)
                .unwrap(),
            &b,
        );

        assert!(
            supervisor
                .cancel(
                    b.router_session(),
                    &RequestCancel {
                        request_id: "same".to_string(),
                        reason: None,
                    },
                )
                .await
        );
        assert!(a_request.execution_budget().settlement().is_none());
        assert_eq!(
            b_request.execution_budget().settlement().unwrap().winner(),
            ExecutionWinner::Cancelled
        );
    }

    #[tokio::test]
    async fn cancel_before_activation_is_stop_without_budget_terminal_or_cleanup() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let key = key("session", "request");
        assert!(supervisor.start_session(key.router_session().clone()));
        let reservation = supervisor
            .reserve(key.clone(), observer(sink.clone(), &key), None)
            .unwrap();

        assert!(
            supervisor
                .cancel(
                    key.router_session(),
                    &RequestCancel {
                        request_id: "request".to_string(),
                        reason: None,
                    },
                )
                .await
        );
        assert!(matches!(
            reservation.activate(&key, &request("request"), telemetry()),
            ActivationOutcome::RevokedByCancel
        ));
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn session_stop_revokes_reserved_and_stops_only_its_active_rows() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let reserved_key = key("session-a", "reserved");
        let active_key = key("session-a", "active");
        let other_key = key("session-b", "active");
        assert!(supervisor.start_session(active_key.router_session().clone()));
        assert!(supervisor.start_session(other_key.router_session().clone()));
        let reserved = supervisor
            .reserve(
                reserved_key.clone(),
                observer(sink.clone(), &reserved_key),
                None,
            )
            .unwrap();
        let active = activate(
            supervisor
                .reserve(
                    active_key.clone(),
                    observer(sink.clone(), &active_key),
                    None,
                )
                .unwrap(),
            &active_key,
        );
        let other = activate(
            supervisor
                .reserve(other_key.clone(), observer(sink.clone(), &other_key), None)
                .unwrap(),
            &other_key,
        );

        supervisor.stop_session(active_key.router_session());
        assert!(matches!(
            reserved.activate(&reserved_key, &request("reserved"), telemetry()),
            ActivationOutcome::RevokedBySessionStop
        ));
        assert_eq!(
            active.execution_budget().settlement().unwrap().winner(),
            ExecutionWinner::InternalStop
        );
        assert!(other.execution_budget().settlement().is_none());
    }

    #[tokio::test]
    async fn one_frozen_winner_mints_one_terminal_and_one_cleanup_permit() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let key = key("session", "request");
        assert!(supervisor.start_session(key.router_session().clone()));
        let supervised = activate(
            supervisor
                .reserve(key.clone(), observer(sink.clone(), &key), None)
                .unwrap(),
            &key,
        );

        let snapshot = RequestExecutionOwnerInventorySnapshot {
            pending: FrozenOwnerDomain {
                current: 1,
                ever_created: true,
            },
            resource: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            child: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            child_heap: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            boundary: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
            actor: FrozenOwnerDomain {
                current: 0,
                ever_created: false,
            },
        };
        let permit = supervisor
            .complete_success(&supervised, snapshot, CompletionTrace::RUNTIME)
            .await
            .unwrap();
        assert_eq!(
            permit.response_action(),
            CompletionResponseAction::Candidate
        );
        let settlement = permit.settlement();
        let settled_raw = settlement.raw_executed_count();
        let settled_hard_limit = settlement.hard_raw_limit();
        let settled_poll_count = settlement.poll_count();
        assert!(supervisor
            .complete_success(&supervised, zero_snapshot(), CompletionTrace::RUNTIME)
            .await
            .is_none());
        permit.observe_cleanup();

        let records = sink.0.lock().unwrap();
        assert_eq!(records.len(), 3);
        assert!(matches!(
            &records[0].event,
            BytecodeExecutionEvent::VmBudgetAccounted(VmBudgetAccounted {
                raw_executed_count,
                charged_instruction_count,
                hard_limit,
                poll_count,
            }) if raw_executed_count == &settled_raw
                && charged_instruction_count == &settled_raw
                && hard_limit == &settled_hard_limit
                && poll_count == &settled_poll_count
        ));
        assert!(matches!(
            records[1].event,
            BytecodeExecutionEvent::RequestTerminalClaimed(RequestTerminalClaimed {
                terminal: BytecodeRequestTerminal::Succeeded
            })
        ));
        assert!(matches!(
            records[2].event,
            BytecodeExecutionEvent::RequestCleanupComplete(RequestCleanupComplete {
                owner_inventory
            }) if owner_inventory == snapshot
        ));
    }

    #[tokio::test]
    async fn cancelled_winner_is_stop_without_response() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let key = key("session", "request");
        assert!(supervisor.start_session(key.router_session().clone()));
        let supervised = activate(
            supervisor
                .reserve(key.clone(), observer(sink, &key), None)
                .unwrap(),
            &key,
        );
        supervisor
            .cancel(
                key.router_session(),
                &RequestCancel {
                    request_id: "request".to_string(),
                    reason: None,
                },
            )
            .await;

        let permit = supervisor
            .complete_cancelled(&supervised, zero_snapshot(), CompletionTrace::RUNTIME)
            .await
            .unwrap();
        assert_eq!(
            permit.response_action(),
            CompletionResponseAction::StopWithoutResponse
        );
        assert!(!permit.response_owned());
    }

    #[tokio::test]
    async fn due_deadline_overrides_a_success_candidate_with_frozen_response_facts() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let key = key("session", "deadline");
        assert!(supervisor.start_session(key.router_session().clone()));
        let deadline = AdmittedRequestDeadline::new(
            Instant::now()
                .checked_sub(std::time::Duration::from_millis(1))
                .unwrap(),
        );
        let supervised = activate(
            supervisor
                .reserve(key.clone(), observer(sink, &key), Some(deadline))
                .unwrap(),
            &key,
        );

        let permit = supervisor
            .complete_success(&supervised, zero_snapshot(), CompletionTrace::RUNTIME)
            .await
            .unwrap();
        assert!(matches!(
            permit.response_action(),
            CompletionResponseAction::DeadlineExceeded {
                instruction_count: 0,
                limit: skiff_runtime_request::execution_budget::DEFAULT_INSTRUCTION_LIMIT,
                ..
            }
        ));
        let response = permit.response_override().unwrap();
        assert_eq!(response.code, "TimeoutError");
        assert_eq!(response.message, "execution deadline exceeded");
    }

    #[tokio::test]
    async fn cancel_received_suppresses_a_later_deadline_response() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let key = key("session", "cancel-deadline-race");
        assert!(supervisor.start_session(key.router_session().clone()));
        let deadline = AdmittedRequestDeadline::new(
            Instant::now()
                .checked_sub(std::time::Duration::from_millis(1))
                .unwrap(),
        );
        let supervised = activate(
            supervisor
                .reserve(key.clone(), observer(sink, &key), Some(deadline))
                .unwrap(),
            &key,
        );

        assert!(
            supervisor
                .cancel(
                    key.router_session(),
                    &RequestCancel {
                        request_id: "cancel-deadline-race".to_string(),
                        reason: None,
                    },
                )
                .await
        );

        let permit = supervisor
            .complete_success(&supervised, zero_snapshot(), CompletionTrace::RUNTIME)
            .await
            .unwrap();
        assert_eq!(
            permit.response_action(),
            CompletionResponseAction::StopWithoutResponse
        );
        assert!(!permit.response_owned());
        assert!(permit.response_override().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn request_id_stays_guarded_through_terminal_and_cleanup_observers() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let key = key("session", "request");
        assert!(supervisor.start_session(key.router_session().clone()));
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
            .reserve(key.clone(), observer(sink.clone(), &key), None)
            .expect("first reservation");
        let supervised = activate(reservation, &key);

        let completing_supervisor = Arc::clone(&supervisor);
        let completing_request = supervised.clone();
        let completing = tokio::spawn(async move {
            completing_supervisor
                .complete_success(
                    &completing_request,
                    zero_snapshot(),
                    CompletionTrace::RUNTIME,
                )
                .await
                .expect("winner cleanup permit")
        });
        terminal_entered_rx
            .recv()
            .await
            .expect("terminal observer entered");

        assert!(supervisor
            .reserve(key.clone(), observer(sink.clone(), &key), None)
            .is_none());
        assert!(
            !supervisor
                .cancel(
                    key.router_session(),
                    &RequestCancel {
                        request_id: "request".to_string(),
                        reason: Some("late cancel".to_string()),
                    },
                )
                .await
        );
        assert_eq!(supervisor.active_count().await, 0);

        terminal_release_tx
            .send(())
            .expect("release terminal observer");
        let permit = completing.await.expect("completion task");
        assert!(permit.response_owned());
        assert!(supervisor
            .reserve(key.clone(), observer(sink.clone(), &key), None)
            .is_none());
        assert!(
            !supervisor
                .cancel(
                    key.router_session(),
                    &RequestCancel {
                        request_id: "request".to_string(),
                        reason: None,
                    },
                )
                .await
        );

        let cleaning = tokio::task::spawn_blocking(move || permit.observe_cleanup());
        cleanup_entered_rx
            .recv()
            .await
            .expect("cleanup observer entered");
        assert!(supervisor
            .reserve(key.clone(), observer(sink.clone(), &key), None)
            .is_none());
        assert!(
            !supervisor
                .cancel(
                    key.router_session(),
                    &RequestCancel {
                        request_id: "request".to_string(),
                        reason: None,
                    },
                )
                .await
        );

        cleanup_release_tx
            .send(())
            .expect("release cleanup observer");
        cleaning.await.expect("cleanup task");

        let next_reservation = supervisor
            .reserve(key.clone(), observer(sink.clone(), &key), None)
            .expect("reuse only after cleanup returned");
        let next_supervised = activate(next_reservation, &key);
        let next_permit = supervisor
            .complete_success(&next_supervised, zero_snapshot(), CompletionTrace::RUNTIME)
            .await
            .expect("next completion winner");
        next_permit.observe_cleanup();

        let records = sink.records.lock().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| record.ordinal)
                .collect::<Vec<_>>(),
            [0, 1, 2, 0, 1, 2]
        );
        assert!(matches!(
            &records[0].event,
            BytecodeExecutionEvent::VmBudgetAccounted(_)
        ));
        assert!(matches!(
            &records[1].event,
            BytecodeExecutionEvent::RequestTerminalClaimed(_)
        ));
        assert!(matches!(
            &records[2].event,
            BytecodeExecutionEvent::RequestCleanupComplete(_)
        ));
        assert!(records.iter().all(|record| {
            record.correlation.router_session_id == "session"
                && record.correlation.request_id == "request"
        }));
        assert!(!sink.overlap.load(Ordering::SeqCst));
        assert_eq!(sink.callbacks.load(Ordering::SeqCst), 0);
    }

    #[derive(Default)]
    struct RecordingPendingSink(Mutex<Vec<ExecutionWinner>>);

    impl RequestPendingSink for RecordingPendingSink {
        fn on_terminal(&self, winner: ExecutionWinner) {
            self.0.lock().unwrap().push(winner);
        }
    }

    #[tokio::test]
    async fn session_stop_terminates_registered_pending_sinks_exactly_once() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let sink = Arc::new(RecordingSink::default());
        let key = key("session", "parked");
        assert!(supervisor.start_session(key.router_session().clone()));
        let supervised = activate(
            supervisor
                .reserve(key.clone(), observer(sink.clone(), &key), None)
                .unwrap(),
            &key,
        );
        let pending_sink = Arc::new(RecordingPendingSink::default());
        assert_eq!(
            supervised
                .execution_budget()
                .register_pending_sink(pending_sink.clone()),
            None
        );

        supervisor.stop_session(key.router_session());
        supervisor.stop_session(key.router_session());

        assert_eq!(
            *pending_sink.0.lock().unwrap(),
            [ExecutionWinner::InternalStop]
        );
        assert_eq!(
            supervised.execution_budget().settlement().unwrap().winner(),
            ExecutionWinner::InternalStop
        );
        // Pending termination must not mint request terminal/cleanup
        // observations; only the request finalizer does that, exactly once.
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancel_terminates_registered_pending_sinks_with_one_winner() {
        let supervisor = Arc::new(RequestSupervisor::new());
        let key = key("session", "cancelled");
        assert!(supervisor.start_session(key.router_session().clone()));
        let supervised = activate(
            supervisor
                .reserve(
                    key.clone(),
                    observer(Arc::new(RecordingSink::default()), &key),
                    None,
                )
                .unwrap(),
            &key,
        );
        let pending_sink = Arc::new(RecordingPendingSink::default());
        assert_eq!(
            supervised
                .execution_budget()
                .register_pending_sink(pending_sink.clone()),
            None
        );

        assert!(
            supervisor
                .cancel(
                    key.router_session(),
                    &RequestCancel {
                        request_id: "cancelled".to_string(),
                        reason: None,
                    },
                )
                .await
        );
        assert_eq!(
            *pending_sink.0.lock().unwrap(),
            [ExecutionWinner::Cancelled]
        );
        assert_eq!(
            supervised.execution_budget().settlement().unwrap().winner(),
            ExecutionWinner::Cancelled
        );
    }
}
