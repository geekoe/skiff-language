use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use serde_json::{Map, Value};
use skiff_runtime_model::service_error::{
    ErrorCorrelation, OpaqueServiceError, ServiceErrorEnvelope,
};
use skiff_runtime_request::{
    cancellation::CancellationToken, execution_budget::ExecutionBudget,
    execution_budget_trace_attrs, response_error_to_telemetry_map, RequestCancel, RequestEnvelope,
    ResponseError,
};
use skiff_runtime_transport::protocol::{
    BytecodeRequestStartFrameHeader, BytecodeTaskRequestStartFrameHeader,
};
use tokio::sync::Mutex;

use crate::telemetry::RequestTelemetryContext;

#[derive(Clone)]
struct ActiveRequest {
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    telemetry: RequestTelemetryContext,
    started_at: Instant,
    cancel_requested: Arc<AtomicBool>,
    cancel_event_emitted: Arc<AtomicBool>,
}

#[derive(Clone)]
pub(crate) struct SupervisedRequest {
    request_id: String,
    active: ActiveRequest,
}

#[derive(Clone, Copy)]
pub(crate) struct CompletionTrace {
    include_duration: bool,
    include_budget_attrs: bool,
}

enum CompletionClaim {
    Missing,
    Cancelled,
    Ordinary,
}

impl CompletionTrace {
    pub(crate) const RUNTIME: Self = Self {
        include_duration: true,
        include_budget_attrs: true,
    };
}

#[derive(Default)]
pub(crate) struct RequestSupervisor {
    active: Mutex<HashMap<String, ActiveRequest>>,
}

impl RequestSupervisor {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn begin(
        &self,
        request: &RequestEnvelope,
        telemetry: RequestTelemetryContext,
    ) -> SupervisedRequest {
        self.begin_with_budget(
            request.request_id.clone(),
            Arc::new(ExecutionBudget::for_runtime_request(&request.extra)),
            telemetry,
        )
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn begin_http_gateway(
        &self,
        header: &BytecodeRequestStartFrameHeader,
        telemetry: RequestTelemetryContext,
    ) -> SupervisedRequest {
        let mut extra = Map::new();
        if let Some(deadline) = &header.deadline {
            extra.insert(
                "deadline".to_string(),
                serde_json::to_value(deadline)
                    .expect("typed HTTP gateway deadline remains serializable"),
            );
        }
        self.begin_with_budget(
            header.request_id.clone(),
            Arc::new(ExecutionBudget::for_runtime_request(&extra)),
            telemetry,
        )
        .await
    }

    pub(crate) async fn begin_task(
        &self,
        header: &BytecodeTaskRequestStartFrameHeader,
        telemetry: RequestTelemetryContext,
    ) -> Option<SupervisedRequest> {
        let mut extra = Map::new();
        if let Some(deadline) = &header.deadline {
            extra.insert(
                "deadline".to_string(),
                serde_json::to_value(deadline).expect("typed task deadline remains serializable"),
            );
        }
        let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(&extra));
        let cancellation = CancellationToken::new();
        let active = ActiveRequest {
            cancellation,
            execution_budget,
            telemetry,
            started_at: Instant::now(),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            cancel_event_emitted: Arc::new(AtomicBool::new(false)),
        };
        let mut requests = self.active.lock().await;
        if requests.contains_key(&header.request_id) {
            return None;
        }
        requests.insert(header.request_id.clone(), active.clone());
        Some(SupervisedRequest {
            request_id: header.request_id.clone(),
            active,
        })
    }

    async fn begin_with_budget(
        &self,
        request_id: String,
        execution_budget: Arc<ExecutionBudget>,
        telemetry: RequestTelemetryContext,
    ) -> SupervisedRequest {
        let cancellation = CancellationToken::new();
        let active = ActiveRequest {
            cancellation,
            execution_budget,
            telemetry,
            started_at: Instant::now(),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            cancel_event_emitted: Arc::new(AtomicBool::new(false)),
        };

        self.active
            .lock()
            .await
            .insert(request_id.clone(), active.clone());

        SupervisedRequest { request_id, active }
    }

    pub(crate) async fn complete_success(
        &self,
        request: &SupervisedRequest,
        trace: CompletionTrace,
    ) -> bool {
        match self.claim_completion(request).await {
            CompletionClaim::Missing => return false,
            CompletionClaim::Cancelled => {
                finish_cancelled_request(request, trace);
                return false;
            }
            CompletionClaim::Ordinary => {}
        }
        request.active.execution_budget.finish(Instant::now());
        let duration_ms = request.duration_ms();
        emit_request_duration_metric(&request.active, duration_ms, "ok");
        true
    }

    pub(crate) async fn complete_error(
        &self,
        request: &SupervisedRequest,
        event_name: &'static str,
        error: &ResponseError,
        trace: CompletionTrace,
    ) -> bool {
        match self.claim_completion(request).await {
            CompletionClaim::Missing => return false,
            CompletionClaim::Cancelled => {
                finish_cancelled_request(request, trace);
                return false;
            }
            CompletionClaim::Ordinary => {}
        }
        request.active.execution_budget.finish(Instant::now());
        if event_name == "request.cancel"
            && request
                .active
                .cancel_event_emitted
                .swap(true, Ordering::SeqCst)
        {
            return false;
        }

        let duration_ms = request.duration_ms();
        request.active.telemetry.emit_trace(
            event_name,
            trace.include_duration.then_some(duration_ms),
            Some(response_error_to_telemetry_map(error)),
            request.budget_attrs(duration_ms, trace),
        );
        emit_request_duration_metric(&request.active, duration_ms, "error");
        true
    }

    pub(crate) async fn complete_fixed_service_failure(
        &self,
        request: &SupervisedRequest,
        event_name: &'static str,
        error: &OpaqueServiceError,
        trace: CompletionTrace,
    ) -> bool {
        match self.claim_completion(request).await {
            CompletionClaim::Missing => return false,
            CompletionClaim::Cancelled => {
                finish_cancelled_request(request, trace);
                return false;
            }
            CompletionClaim::Ordinary => {}
        }
        request.active.execution_budget.finish(Instant::now());

        let duration_ms = request.duration_ms();
        let correlation = ErrorCorrelation {
            trace_id: error.envelope().trace_id().to_string(),
            error_id: error.envelope().error_id().to_string(),
        };
        request.active.telemetry.emit_trace_with_error_correlation(
            event_name,
            trace.include_duration.then_some(duration_ms),
            Some(fixed_service_failure_telemetry_map(error)),
            request.budget_attrs(duration_ms, trace),
            &correlation,
        );
        emit_request_duration_metric(&request.active, duration_ms, "error");
        true
    }

    pub(crate) async fn complete_cancelled(
        &self,
        request: &SupervisedRequest,
        trace: CompletionTrace,
    ) -> bool {
        let claim = self.claim_completion(request).await;
        if matches!(claim, CompletionClaim::Missing) {
            return false;
        }
        finish_cancelled_request(request, trace);
        true
    }

    pub(crate) async fn cancel(&self, cancel: &RequestCancel) -> bool {
        let active_requests = self.active.lock().await;
        let Some(active) = active_requests.get(&cancel.request_id) else {
            return false;
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
            emit_request_duration_metric(active, duration_ms, "cancel");
        }
        true
    }

    async fn claim_completion(&self, request: &SupervisedRequest) -> CompletionClaim {
        let mut active = self.active.lock().await;
        let Some(current) = active.get(&request.request_id) else {
            return CompletionClaim::Missing;
        };
        if !Arc::ptr_eq(
            &current.cancel_event_emitted,
            &request.active.cancel_event_emitted,
        ) {
            return CompletionClaim::Missing;
        }
        // The work token is also used to stop losing child lanes after an
        // ordinary deadline has won. Only an explicit root cancellation may
        // change the observable terminal from success/error to cancellation.
        let cancelled = current.cancel_requested.load(Ordering::SeqCst);
        active.remove(&request.request_id);
        if cancelled {
            CompletionClaim::Cancelled
        } else {
            CompletionClaim::Ordinary
        }
    }

    pub(crate) async fn active_count(&self) -> usize {
        self.active.lock().await.len()
    }
}

fn finish_cancelled_request(request: &SupervisedRequest, trace: CompletionTrace) {
    request
        .active
        .cancel_requested
        .store(true, Ordering::SeqCst);
    request.active.cancellation.cancel();
    request.active.execution_budget.record_cancelled();
    request.active.execution_budget.finish(Instant::now());
    if request
        .active
        .cancel_event_emitted
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    let duration_ms = request.duration_ms();
    request.active.telemetry.emit_trace(
        "request.cancel",
        trace.include_duration.then_some(duration_ms),
        None,
        request.budget_attrs(duration_ms, trace),
    );
    emit_request_duration_metric(&request.active, duration_ms, "cancel");
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

impl SupervisedRequest {
    fn duration_ms(&self) -> f64 {
        elapsed_ms(self.active.started_at)
    }

    fn budget_attrs(&self, duration_ms: f64, trace: CompletionTrace) -> Option<Map<String, Value>> {
        trace
            .include_budget_attrs
            .then(|| execution_budget_trace_attrs(&self.active.execution_budget, duration_ms))
    }
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
