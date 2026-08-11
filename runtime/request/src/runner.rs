use serde_json::Value;
use crate::{execution_budget::ExecutionStats, ExecutionBudget, ResponseError};

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
