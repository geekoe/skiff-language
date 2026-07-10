use std::sync::{atomic::AtomicBool, Arc};

use serde_json::Value;
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_model::request_heap::RequestHeapLimits;

use crate::{
    execution_budget::ExecutionStats, BoundaryResponse, ExecutionBudget, RequestEnvelope,
    RequestError, RequestEvalAdapter, RequestOperationContext, ResponseError, ResponseEventSink,
};

pub type RuntimeResponse = BoundaryResponse;
pub type RequestExecutionResult = std::result::Result<BoundaryResponse, RequestExecutionError>;

pub struct RequestExecutionError {
    error: RequestError,
    attach_request_diagnostic: bool,
}

impl RequestExecutionError {
    fn with_request_diagnostic(error: RequestError) -> Self {
        Self {
            error,
            attach_request_diagnostic: true,
        }
    }

    pub fn into_error(self) -> RequestError {
        self.error
    }

    pub fn attach_request_diagnostic(&self) -> bool {
        self.attach_request_diagnostic
    }
}

impl From<RequestError> for RequestExecutionError {
    fn from(error: RequestError) -> Self {
        Self {
            error,
            attach_request_diagnostic: false,
        }
    }
}

pub struct RequestExecutionInput {
    pub operation_context: RequestOperationContext,
    pub request: RequestEnvelope,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: RequestExecutionHandles,
}

#[derive(Clone)]
pub struct RequestExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub streaming_available: bool,
    pub response_events: Arc<dyn ResponseEventSink>,
    pub eval_adapter: Arc<dyn RequestEvalAdapter>,
}

pub async fn execute_runtime_request(input: RequestExecutionInput) -> RequestExecutionResult {
    let RequestExecutionInput {
        operation_context,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    let RequestOperationContext {
        metadata,
        eval_program,
        operation,
        addr,
    } = operation_context;
    let execution = super::ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    super::ingress::IngressDispatcher::validate_request(&request)?;

    super::ingress::IngressDispatcher::new(super::ingress::IngressDispatchInput {
        operation: &operation,
        addr: &addr,
        metadata: &metadata,
        request: &request,
        execution,
        cancellation,
        cancelled: cancelled.as_ref(),
        execution_budget: execution_budget.clone(),
        handles: &handles,
        eval_program: eval_program.clone(),
    })
    .dispatch()
    .await
    .map_err(RequestExecutionError::with_request_diagnostic)
}

pub fn response_error_to_telemetry_map(error: &ResponseError) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert("code".to_string(), Value::String(error.code.clone()));
    map.insert("message".to_string(), Value::String(error.message.clone()));
    if let Some(details) = error.details.clone() {
        map.insert("details".to_string(), details);
    }
    map
}

pub fn execution_budget_trace_attrs(
    execution_budget: &ExecutionBudget,
    duration_ms: f64,
) -> serde_json::Map<String, Value> {
    execution_stats_trace_attrs(execution_budget.stats_snapshot(), duration_ms)
}

fn execution_stats_trace_attrs(
    stats: ExecutionStats,
    duration_ms: f64,
) -> serde_json::Map<String, Value> {
    let mut attrs = serde_json::Map::new();
    attrs.insert(
        "instructionCount".to_string(),
        Value::Number(stats.instruction_count.into()),
    );
    attrs.insert(
        "budgetLimit".to_string(),
        stats.budget_limit.map_or(Value::Null, |limit| {
            Value::Number(serde_json::Number::from(limit))
        }),
    );
    attrs.insert(
        "budgetExceeded".to_string(),
        Value::Bool(stats.budget_exceeded),
    );
    attrs.insert("elapsedMs".to_string(), json_number(duration_ms));
    attrs.insert("budgetElapsedMs".to_string(), json_number(stats.elapsed_ms));
    attrs.insert(
        "budgetPollCount".to_string(),
        Value::Number(stats.poll_count.into()),
    );
    if let Some(reason) = stats.budget_reason {
        attrs.insert(
            "budgetReason".to_string(),
            Value::String(reason.as_str().to_string()),
        );
    }
    attrs
}

fn json_number(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
}
