use std::sync::Arc;

use serde_json::{Map, Value};

use skiff_runtime_model::service_error::ErrorCorrelation;
use skiff_runtime_transport::protocol::{
    PlatformEvent, TelemetryEvent, TelemetrySource, TelemetryVisibility,
};

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
        self.emit_trace_event(name.into(), duration_ms, error, attrs, None);
    }

    pub fn emit_trace_with_error_correlation(
        &self,
        name: impl Into<String>,
        duration_ms: Option<f64>,
        error: Option<Map<String, Value>>,
        attrs: Option<Map<String, Value>>,
        correlation: &ErrorCorrelation,
    ) {
        self.emit_trace_event(name.into(), duration_ms, error, attrs, Some(correlation));
    }

    /// Emits a duration metric event (not a span: no `spanId`, duration is a
    /// numeric attr) so the consumer aggregates it into per-bucket
    /// count/sum/avg/min/max/p95 series keyed by `name` + `serviceId`.
    pub fn emit_duration_metric(&self, name: impl Into<String>, attrs: Option<Map<String, Value>>) {
        let mut event = PlatformEvent::new(name)
            .with_attrs(attrs)
            .into_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
        event.trace_id = self.trace_id.clone();
        self.apply_correlation(&mut event);
        event.span_id = None;
        event.parent_span_id = None;
        self.emit(event);
    }

    fn emit_trace_event(
        &self,
        name: String,
        duration_ms: Option<f64>,
        error: Option<Map<String, Value>>,
        attrs: Option<Map<String, Value>>,
        correlation: Option<&ErrorCorrelation>,
    ) {
        let mut event = PlatformEvent::new(name)
            .with_duration_ms(duration_ms)
            .with_error(error)
            .with_attrs(attrs)
            .into_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
        event.trace_id = correlation
            .map(|correlation| correlation.trace_id.clone())
            .or_else(|| self.trace_id.clone());
        event.error_id = correlation.map(|correlation| correlation.error_id.clone());
        self.apply_correlation(&mut event);
        self.emit(event);
    }

    fn apply_correlation(&self, event: &mut TelemetryEvent) {
        event.service_id = self.service_id.clone();
        event.revision_id = self.revision_id.clone();
        event.build_id = self.build_id.clone();
        event.activation_identity = self.activation_identity.clone();
        event.runtime_id = self.runtime_id.clone();
        event.request_id = self.request_id.clone();
        event.span_id = self.span_id.clone();
        event.parent_span_id = self.parent_span_id.clone();
        event.target = self.target.clone();
    }
}

pub fn telemetry_event(ts: impl Into<String>, source: TelemetrySource) -> TelemetryEvent {
    TelemetryEvent {
        ts: ts.into(),
        source,
        visibility: TelemetryVisibility::Operational,
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
        error_id: None,
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
