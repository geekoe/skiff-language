use serde_json::Value;
use skiff_runtime_request::RequestEnvelope;
use skiff_runtime_transport::protocol::TelemetryEvent;

#[derive(Clone, Default)]
pub(super) struct RequestTraceFields {
    pub(super) trace_id: Option<String>,
    pub(super) span_id: Option<String>,
    pub(super) parent_span_id: Option<String>,
}

impl RequestTraceFields {
    pub(super) fn from_request(request: &RequestEnvelope) -> Self {
        Self {
            trace_id: request_trace_field(request, "traceId"),
            span_id: request_trace_field(request, "spanId"),
            parent_span_id: request_trace_field(request, "parentSpanId"),
        }
    }

    pub(super) fn apply_to_event(&self, event: &mut TelemetryEvent) {
        event.trace_id = self.trace_id.clone();
        event.span_id = self.span_id.clone();
        event.parent_span_id = self.parent_span_id.clone();
    }
}

fn request_trace_field(request: &RequestEnvelope, field: &str) -> Option<String> {
    request
        .extra
        .get("trace")
        .and_then(Value::as_object)
        .and_then(|trace| trace.get(field))
        .and_then(Value::as_str)
        .map(str::to_string)
}
