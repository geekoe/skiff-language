//! Runtime-local telemetry capability for native telemetry emission.

use serde_json::{Map, Value};

use crate::{
    config_view::materialize_json,
    error::{Result, RuntimeError},
    telemetry::{telemetry_event, telemetry_timestamp_now, RequestTelemetryContext},
};
use skiff_runtime_transport::protocol::{
    TelemetryEvent, TelemetryLevel, TelemetrySource, TelemetryTopic,
};

#[derive(Clone)]
pub struct TelemetryCapabilityContext {
    request: Option<RequestTelemetryContext>,
}

impl TelemetryCapabilityContext {
    pub fn new(request: Option<RequestTelemetryContext>) -> Self {
        Self { request }
    }

    pub fn emit_native(&self, target: &str, args: &[Value]) -> Result<Value> {
        if target != "std.telemetry.emit" {
            return Err(RuntimeError::Unsupported(format!(
                "unsupported telemetry target {target}"
            )));
        }
        if matches!(
            args.first(),
            Some(Value::String(_)) | Some(Value::Null) | None
        ) {
            return self.emit_log_args(target, args);
        }
        let input = args
            .first()
            .and_then(Value::as_object)
            .ok_or_else(|| RuntimeError::Decode(format!("{target} payload must be an object")))?;
        let topic = input
            .get("topic")
            .map(decode_telemetry_topic)
            .transpose()?
            .unwrap_or(TelemetryTopic::Log);
        let source = input
            .get("source")
            .map(decode_telemetry_source)
            .transpose()?
            .unwrap_or(TelemetrySource::Runtime);
        let mut event = telemetry_event(topic, telemetry_timestamp_now(), source);
        self.apply_request_context(&mut event);
        event.level = input.get("level").map(decode_telemetry_level).transpose()?;
        event.name = optional_string(input, "name", target)?;
        event.message = optional_string(input, "message", target)?;
        event.target = optional_string(input, "target", target)?.or(event.target);
        event.attrs = optional_object(input, "attrs", target)?;
        event.error = optional_object(input, "error", target)?;
        event.duration_ms = input
            .get("durationMs")
            .or_else(|| input.get("duration_ms"))
            .map(|value| {
                value.as_f64().ok_or_else(|| {
                    RuntimeError::Decode(format!("{target} durationMs must be a number"))
                })
            })
            .transpose()?;

        self.emit(event);
        Ok(Value::Null)
    }

    fn emit_log_args(&self, target: &str, args: &[Value]) -> Result<Value> {
        let level = args
            .first()
            .ok_or_else(|| RuntimeError::Decode(format!("{target} level must be a string")))
            .and_then(decode_telemetry_level)?;
        let message = args
            .get(1)
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError::Decode(format!("{target} message must be a string")))?;
        let attrs = match args.get(2) {
            None | Some(Value::Null) => None,
            Some(Value::Object(_)) => Some(
                materialize_json(args.get(2).cloned().unwrap_or(Value::Null))?
                    .as_object()
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::Decode(format!("{target} attrs must be an object"))
                    })?,
            ),
            Some(_) => {
                return Err(RuntimeError::Decode(format!(
                    "{target} attrs must be an object or null"
                )));
            }
        };

        let mut event = telemetry_event(
            TelemetryTopic::Log,
            telemetry_timestamp_now(),
            TelemetrySource::Runtime,
        );
        self.apply_request_context(&mut event);
        event.level = Some(level);
        event.message = Some(message.to_string());
        event.attrs = attrs;

        self.emit(event);
        Ok(Value::Null)
    }

    fn apply_request_context(&self, event: &mut TelemetryEvent) {
        let Some(context) = self.request.as_ref() else {
            return;
        };
        event.service_id = context.service_id.clone();
        event.revision_id = context.revision_id.clone();
        event.build_id = context.build_id.clone();
        event.activation_identity = context.activation_identity.clone();
        event.runtime_id = context.runtime_id.clone();
        event.request_id = context.request_id.clone();
        event.trace_id = context.trace_id.clone();
        event.span_id = context.span_id.clone();
        event.parent_span_id = context.parent_span_id.clone();
        event.target = context.target.clone();
    }

    fn emit(&self, event: TelemetryEvent) {
        if let Some(context) = self.request.as_ref() {
            context.emit(event);
        }
    }
}

fn decode_telemetry_topic(value: &Value) -> Result<TelemetryTopic> {
    match value.as_str() {
        Some("log") => Ok(TelemetryTopic::Log),
        Some("trace") => Ok(TelemetryTopic::Trace),
        Some("metric") => Ok(TelemetryTopic::Metric),
        Some("health") => Ok(TelemetryTopic::Health),
        Some("debug") => Ok(TelemetryTopic::Debug),
        _ => Err(RuntimeError::Decode(
            "std.telemetry.emit topic must be one of log, trace, metric, health, debug".to_string(),
        )),
    }
}

fn decode_telemetry_source(value: &Value) -> Result<TelemetrySource> {
    match value.as_str() {
        Some("gateway") => Ok(TelemetrySource::Gateway),
        Some("router") => Ok(TelemetrySource::Router),
        Some("runtime") => Ok(TelemetrySource::Runtime),
        Some("provider") => Ok(TelemetrySource::Provider),
        Some("test") => Ok(TelemetrySource::Test),
        _ => Err(RuntimeError::Decode(
            "std.telemetry.emit source must be one of gateway, router, runtime, provider, test"
                .to_string(),
        )),
    }
}

fn decode_telemetry_level(value: &Value) -> Result<TelemetryLevel> {
    match value.as_str() {
        Some("debug") => Ok(TelemetryLevel::Debug),
        Some("info") => Ok(TelemetryLevel::Info),
        Some("warn") => Ok(TelemetryLevel::Warn),
        Some("error") => Ok(TelemetryLevel::Error),
        _ => Err(RuntimeError::Decode(
            "std.telemetry.emit level must be one of debug, info, warn, error".to_string(),
        )),
    }
}

fn optional_string(input: &Map<String, Value>, key: &str, target: &str) -> Result<Option<String>> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(RuntimeError::Decode(format!(
            "{target} {key} must be a string"
        ))),
    }
}

fn optional_object(
    input: &Map<String, Value>,
    key: &str,
    target: &str,
) -> Result<Option<Map<String, Value>>> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(_)) => materialize_json(input.get(key).cloned().unwrap_or(Value::Null))?
            .as_object()
            .cloned()
            .map(Some)
            .ok_or_else(|| RuntimeError::Decode(format!("{target} {key} must be an object"))),
        Some(_) => Err(RuntimeError::Decode(format!(
            "{target} {key} must be an object"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::{json, Value};
    use skiff_runtime_transport::protocol::{
        TelemetryEvent, TelemetryLevel, TelemetrySource, TelemetryTopic,
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
        assert_eq!(events[0].topic, TelemetryTopic::Log);
        assert_eq!(events[0].source, TelemetrySource::Runtime);
        assert_eq!(events[0].level, Some(TelemetryLevel::Warn));
        assert_eq!(events[0].message.as_deref(), Some("legacy message"));
        assert_eq!(events[0].attrs.as_ref().unwrap()["kind"], "legacy");
        assert_eq!(events[0].service_id.as_deref(), Some("svc"));
        assert_eq!(events[0].request_id.as_deref(), Some("req"));
        assert_eq!(events[0].target.as_deref(), Some("svc.main.run"));
    }

    #[test]
    fn telemetry_native_emit_accepts_object_payload() {
        let (context, events) = telemetry_context();

        let value = context
            .emit_native(
                "std.telemetry.emit",
                &[json!({
                    "topic": "trace",
                    "source": "test",
                    "level": "info",
                    "name": "native.emit",
                    "message": "object message",
                    "target": "custom.target",
                    "attrs": {"shape": "object"},
                    "error": {"code": "E_TEST"},
                    "duration_ms": 12.5
                })],
            )
            .expect("object payload should decode");

        assert_eq!(value, Value::Null);
        let events = events
            .lock()
            .expect("telemetry events lock should not be poisoned");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, TelemetryTopic::Trace);
        assert_eq!(events[0].source, TelemetrySource::Test);
        assert_eq!(events[0].level, Some(TelemetryLevel::Info));
        assert_eq!(events[0].name.as_deref(), Some("native.emit"));
        assert_eq!(events[0].message.as_deref(), Some("object message"));
        assert_eq!(events[0].target.as_deref(), Some("custom.target"));
        assert_eq!(events[0].attrs.as_ref().unwrap()["shape"], "object");
        assert_eq!(events[0].error.as_ref().unwrap()["code"], "E_TEST");
        assert_eq!(events[0].duration_ms, Some(12.5));
        assert_eq!(events[0].service_id.as_deref(), Some("svc"));
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
}
