use serde_json::Value;
use skiff_runtime_request::RequestEnvelope;
use skiff_runtime_transport::protocol::TelemetryEvent;

use crate::telemetry::RequestTelemetryContext;

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

    pub(super) fn apply_to_context(&self, context: &mut RequestTelemetryContext) {
        context.trace_id = self.trace_id.clone();
        context.span_id = self.span_id.clone();
        context.parent_span_id = self.parent_span_id.clone();
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn ingress_trace_fields_apply_to_the_request_telemetry_context() {
        let mut request = RequestEnvelope {
            request_id: "request-trace".to_string(),
            mode: "unary".to_string(),
            target: "entry.run".to_string(),
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
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        };
        request.extra.insert(
            "trace".to_string(),
            serde_json::json!({
                "traceId": "router-trace",
                "spanId": "router-span",
                "parentSpanId": "router-parent",
            }),
        );
        let mut context = RequestTelemetryContext::for_test();

        RequestTraceFields::from_request(&request).apply_to_context(&mut context);

        assert_eq!(context.trace_id.as_deref(), Some("router-trace"));
        assert_eq!(context.span_id.as_deref(), Some("router-span"));
        assert_eq!(context.parent_span_id.as_deref(), Some("router-parent"));
    }
}
