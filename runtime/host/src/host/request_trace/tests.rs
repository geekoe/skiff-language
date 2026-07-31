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
