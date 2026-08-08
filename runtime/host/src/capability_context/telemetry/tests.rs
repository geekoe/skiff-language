use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use skiff_runtime_transport::protocol::{
    TelemetryEvent, TelemetryLevel, TelemetrySource,
};

use super::TelemetryCapabilityContext;
use crate::telemetry::{RequestTelemetryContext, TelemetryEmitter};

#[derive(Debug, Clone)]
struct CapturingTelemetryEmitter {
    events: Arc<Mutex<Vec<TelemetryEvent>>>,
}

impl TelemetryEmitter for CapturingTelemetryEmitter {
    fn emit(&self, event: TelemetryEvent) -> bool {
        self.events
            .lock()
            .expect("telemetry events lock should not be poisoned")
            .push(event);
        true
    }
}

fn telemetry_context() -> (TelemetryCapabilityContext, Arc<Mutex<Vec<TelemetryEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut request = RequestTelemetryContext::new(CapturingTelemetryEmitter {
        events: events.clone(),
    });
    request.service_id = Some("svc".to_string());
    request.build_id = Some("build".to_string());
    request.request_id = Some("req".to_string());
    request.trace_id = Some("trace".to_string());
    request.target = Some("svc.main.run".to_string());
    (TelemetryCapabilityContext::new(Some(request)), events)
}

#[test]
fn telemetry_native_emit_accepts_legacy_log_args() {
    let (context, events) = telemetry_context();

    let value = context
        .emit_native(
            "std.telemetry.emit",
            &[
                Value::String("warn".to_string()),
                Value::String("legacy message".to_string()),
                json!({"kind": "legacy"}),
            ],
        )
        .expect("legacy log args should decode");

    assert_eq!(value, Value::Null);
    let events = events
        .lock()
        .expect("telemetry events lock should not be poisoned");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, TelemetrySource::Runtime);
    assert_eq!(events[0].level, Some(TelemetryLevel::Warn));
    assert_eq!(events[0].message.as_deref(), Some("legacy message"));
    assert_eq!(events[0].attrs.as_ref().unwrap()["kind"], "legacy");
    assert_eq!(events[0].service_id.as_deref(), Some("svc"));
    assert_eq!(events[0].request_id.as_deref(), Some("req"));
    assert_eq!(events[0].target.as_deref(), Some("svc.main.run"));
}

#[test]
fn telemetry_native_emit_without_request_context_returns_null() {
    let context = TelemetryCapabilityContext::new(None);

    let value = context
        .emit_native(
            "std.telemetry.emit",
            &[
                Value::String("info".to_string()),
                Value::String("no context".to_string()),
            ],
        )
        .expect("no request telemetry context should be a no-op");

    assert_eq!(value, Value::Null);
}

#[test]
fn duration_metric_event_is_metric_shaped_with_context_fields() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut request = RequestTelemetryContext::new(CapturingTelemetryEmitter {
        events: events.clone(),
    });
    request.service_id = Some("svc".to_string());
    request.build_id = Some("build".to_string());
    request.request_id = Some("req".to_string());
    request.trace_id = Some("trace".to_string());
    request.target = Some("svc.main.run".to_string());
    request.span_id = Some("span-1".to_string());

    request.emit_duration_metric(
        "request.duration",
        Some(Map::from_iter([
            ("durationMs".to_string(), Value::from(12.5)),
            ("outcome".to_string(), Value::String("ok".to_string())),
        ])),
    );

    let events = events
        .lock()
        .expect("telemetry events lock should not be poisoned");
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.name.as_deref(), Some("request.duration"));
    assert_eq!(event.source, TelemetrySource::Runtime);
    assert_eq!(event.service_id.as_deref(), Some("svc"));
    assert_eq!(event.build_id.as_deref(), Some("build"));
    assert_eq!(event.request_id.as_deref(), Some("req"));
    assert_eq!(event.trace_id.as_deref(), Some("trace"));
    assert_eq!(event.target.as_deref(), Some("svc.main.run"));
    assert_eq!(event.duration_ms, None);
    assert!(event.span_id.is_none(), "metric events must not carry a span id");
    let attrs = event.attrs.as_ref().expect("metric attrs must be present");
    assert_eq!(attrs["durationMs"], 12.5);
    assert_eq!(attrs["outcome"], "ok");
}
