use std::sync::Arc;

use serde_json::{Map, Value};

use skiff_runtime_transport::protocol::{TelemetryEvent, TelemetrySource, TelemetryTopic};

pub trait TelemetryEmitter: std::fmt::Debug + Send + Sync {
    fn emit(&self, event: TelemetryEvent) -> bool;
}

#[derive(Debug, Default)]
struct NoopTelemetryEmitter;

impl TelemetryEmitter for NoopTelemetryEmitter {
    fn emit(&self, _event: TelemetryEvent) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct RequestTelemetryContext {
    emitter: Arc<dyn TelemetryEmitter>,
    pub service_id: Option<String>,
    pub revision_id: Option<String>,
    pub build_id: Option<String>,
    pub activation_identity: Option<String>,
    pub runtime_id: Option<String>,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub target: Option<String>,
}

impl RequestTelemetryContext {
    pub fn new(emitter: impl TelemetryEmitter + 'static) -> Self {
        Self {
            emitter: Arc::new(emitter),
            service_id: None,
            revision_id: None,
            build_id: None,
            activation_identity: None,
            runtime_id: None,
            request_id: None,
            trace_id: None,
            span_id: None,
            parent_span_id: None,
            target: None,
        }
    }

    pub fn for_test() -> Self {
        Self::new(NoopTelemetryEmitter)
    }

    pub fn emit(&self, event: TelemetryEvent) -> bool {
        self.emitter.emit(event)
    }

    pub fn emit_trace(
        &self,
        name: impl Into<String>,
        duration_ms: Option<f64>,
        error: Option<Map<String, Value>>,
        attrs: Option<Map<String, Value>>,
    ) {
        let mut event = telemetry_event(
            TelemetryTopic::Trace,
            telemetry_timestamp_now(),
            TelemetrySource::Runtime,
        );
        event.service_id = self.service_id.clone();
        event.revision_id = self.revision_id.clone();
        event.build_id = self.build_id.clone();
        event.activation_identity = self.activation_identity.clone();
        event.runtime_id = self.runtime_id.clone();
        event.request_id = self.request_id.clone();
        event.trace_id = self.trace_id.clone();
        event.span_id = self.span_id.clone();
        event.parent_span_id = self.parent_span_id.clone();
        event.target = self.target.clone();
        event.name = Some(name.into());
        event.duration_ms = duration_ms;
        event.error = error;
        event.attrs = attrs;
        self.emit(event);
    }
}

pub fn telemetry_event(
    topic: TelemetryTopic,
    ts: impl Into<String>,
    source: TelemetrySource,
) -> TelemetryEvent {
    TelemetryEvent {
        topic,
        ts: ts.into(),
        source,
        service_id: None,
        revision_id: None,
        build_id: None,
        activation_identity: None,
        runtime_id: None,
        provider_id: None,
        provider_revision: None,
        provider_capability: None,
        provider_target: None,
        request_id: None,
        client_request_id: None,
        trace_id: None,
        span_id: None,
        parent_span_id: None,
        target: None,
        level: None,
        name: None,
        message: None,
        attrs: None,
        error: None,
        duration_ms: None,
        dropped: None,
    }
}

pub fn telemetry_timestamp_now() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.millisecond()
    )
}
