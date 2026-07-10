use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use serde_json::{Map, Value};
use skiff_runtime_request::{
    cancellation::CancellationToken, execution_budget::ExecutionBudget,
    execution_budget_trace_attrs, response_error_to_telemetry_map, RequestCancel, RequestEnvelope,
    ResponseError,
};
use tokio::sync::Mutex;

use crate::telemetry::RequestTelemetryContext;

#[derive(Clone)]
struct ActiveRequest {
    cancellation: CancellationToken,
    execution_budget: Arc<ExecutionBudget>,
    telemetry: RequestTelemetryContext,
    started_at: Instant,
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
    cancel_priority: CancelTracePriority,
}

#[derive(Clone, Copy)]
enum CancelTracePriority {
    None,
    RequestCancelOnly,
    AnyError,
}

impl CompletionTrace {
    pub(crate) const RUNTIME: Self = Self {
        include_duration: true,
        include_budget_attrs: true,
        cancel_priority: CancelTracePriority::RequestCancelOnly,
    };

    pub(crate) const SPAWN: Self = Self {
        include_duration: false,
        include_budget_attrs: false,
        cancel_priority: CancelTracePriority::AnyError,
    };

    pub(crate) const SPAWN_RENEW_ERROR: Self = Self {
        include_duration: false,
        include_budget_attrs: false,
        cancel_priority: CancelTracePriority::None,
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
        start_event: &'static str,
    ) -> SupervisedRequest {
        let cancellation = CancellationToken::new();
        let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(&request.extra));
        let active = ActiveRequest {
            cancellation,
            execution_budget,
            telemetry,
            started_at: Instant::now(),
            cancel_event_emitted: Arc::new(AtomicBool::new(false)),
        };

        active.telemetry.emit_trace(start_event, None, None, None);
        self.active
            .lock()
            .await
            .insert(request.request_id.clone(), active.clone());

        SupervisedRequest {
            request_id: request.request_id.clone(),
            active,
        }
    }

    pub(crate) async fn complete_success(
        &self,
        request: &SupervisedRequest,
        event_name: &'static str,
        trace: CompletionTrace,
    ) {
        request.active.execution_budget.finish(Instant::now());
        self.active.lock().await.remove(&request.request_id);
        let duration_ms = request.duration_ms();
        request.active.telemetry.emit_trace(
            event_name,
            trace.include_duration.then_some(duration_ms),
            None,
            request.budget_attrs(duration_ms, trace),
        );
    }

    pub(crate) async fn complete_error(
        &self,
        request: &SupervisedRequest,
        event_name: &'static str,
        error: &ResponseError,
        trace: CompletionTrace,
    ) {
        request.active.execution_budget.finish(Instant::now());
        self.active.lock().await.remove(&request.request_id);
        match trace.cancel_priority {
            CancelTracePriority::None => {}
            CancelTracePriority::RequestCancelOnly if event_name == "request.cancel" => {
                if request
                    .active
                    .cancel_event_emitted
                    .swap(true, Ordering::SeqCst)
                {
                    return;
                }
            }
            CancelTracePriority::RequestCancelOnly => {}
            CancelTracePriority::AnyError => {
                if request
                    .active
                    .cancel_event_emitted
                    .swap(true, Ordering::SeqCst)
                {
                    return;
                }
            }
        }

        let duration_ms = request.duration_ms();
        request.active.telemetry.emit_trace(
            event_name,
            trace.include_duration.then_some(duration_ms),
            Some(response_error_to_telemetry_map(error)),
            request.budget_attrs(duration_ms, trace),
        );
    }

    pub(crate) async fn cancel(&self, cancel: &RequestCancel) -> bool {
        let Some(active) = self.active.lock().await.get(&cancel.request_id).cloned() else {
            return false;
        };

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
        }
        true
    }

    pub(crate) async fn active_count(&self) -> usize {
        self.active.lock().await.len()
    }
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
