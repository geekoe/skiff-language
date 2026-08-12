use serde_json::{Map, Value};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionEventSink, BytecodeExecutionObservation,
};
use skiff_runtime_transport::protocol::{PlatformEvent, TelemetrySource};

use crate::telemetry::telemetry_timestamp_now;

use super::telemetry::TelemetryProducer;

pub(crate) const BYTECODE_EXECUTION_OBSERVATION_SOURCE: &str =
    "skiff-runtime-host::bytecode-execution-observer";

/// Default bounded production projection for typed execution observations.
pub(super) struct TelemetryBytecodeExecutionEventSink {
    telemetry: TelemetryProducer,
    runtime_id: String,
}

impl TelemetryBytecodeExecutionEventSink {
    pub(super) fn new(telemetry: TelemetryProducer, runtime_id: String) -> Self {
        Self {
            telemetry,
            runtime_id,
        }
    }
}

impl BytecodeExecutionEventSink for TelemetryBytecodeExecutionEventSink {
    fn observe(&self, observation: BytecodeExecutionObservation) {
        let Ok(Value::Object(mut event)) = serde_json::to_value(observation.event) else {
            return;
        };
        let Some(Value::String(kind)) = event.remove("kind") else {
            return;
        };
        let Some(payload) = event.remove("payload") else {
            return;
        };
        let mut attrs = Map::new();
        attrs.insert(
            "observationSource".to_string(),
            Value::String(BYTECODE_EXECUTION_OBSERVATION_SOURCE.to_string()),
        );
        attrs.insert(
            "routerSessionId".to_string(),
            Value::String(observation.correlation.router_session_id),
        );
        attrs.insert(
            "requestId".to_string(),
            Value::String(observation.correlation.request_id.clone()),
        );
        attrs.insert("ordinal".to_string(), Value::from(observation.ordinal));
        attrs.insert("payload".to_string(), payload);

        let mut projected = PlatformEvent::new(kind)
            .with_attrs(Some(attrs))
            .into_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
        projected.runtime_id = Some(self.runtime_id.clone());
        projected.request_id = Some(observation.correlation.request_id);
        let _ = self.telemetry.try_emit(projected);
    }
}
