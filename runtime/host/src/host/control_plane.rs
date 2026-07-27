#[cfg(any())]
use std::{collections::HashSet, sync::Arc};

use serde_json::Value;
#[cfg(any())]
use serde_json::{json, Map};
use skiff_artifact_model::AssemblyActivationControl;
#[cfg(any())]
use skiff_artifact_model::ConfigShape;
#[cfg(any())]
use skiff_runtime_linked_program::{package_config_shape, LinkedProgramImage};
use skiff_runtime_request::{self as request_runner, RequestEnvelope, RouterWriterMessage};
#[cfg(any())]
use skiff_runtime_transport::protocol::{
    RouterControlEnvelope, RouterControlPackageConfig, RouterControlServiceConfig,
};
use skiff_runtime_transport::{
    assembly_activation::{encode_assembly_activation_frame, AssemblyActivationFrameDirection},
    protocol::{
        encode_binary_frame, RuntimeCapabilitiesFrameHeader,
        RuntimeCapabilitiesFrameHeaderMetadata, TelemetryEvent, TelemetrySource, TelemetryTopic,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
    telemetry::{telemetry_event, telemetry_timestamp_now},
};
#[cfg(any())]
use crate::{
    capability_context::DbProviderConfig, config_view::RuntimeConfigView,
    loader::artifact_roots_control_fingerprint,
};

#[cfg(any())]
use super::telemetry::TelemetryExporter;
use super::telemetry::EXPORTER_SHUTDOWN_FLUSH_TIMEOUT;
use super::{request_trace, RuntimeHost};

impl RuntimeHost {
    #[cfg(any())]
    fn apply_file_backend_control(&self, control: &RouterControlEnvelope) -> Result<()> {
        let backend = control.file_backend.as_ref().map(|config| {
            if config.local.is_some() {
                "local"
            } else {
                "oss"
            }
        });
        let next_store = control
            .file_backend
            .as_ref()
            .map(blob_store_from_control)
            .transpose()
            .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?
            .map(Arc::<dyn BlobStore>::from);
        let mut store = self.blob_store.lock().map_err(|_| {
            RuntimeError::invalid_artifact("runtime blob store lock is poisoned".to_string())
        })?;
        *store = next_store;
        if let Some(backend) = backend {
            info!(event = "runtime.file_backend_configured", backend = backend);
        }
        Ok(())
    }

    #[cfg(any())]
    pub(crate) async fn apply_telemetry_control(&self, control: &RouterControlEnvelope) {
        let telemetry = control.telemetry.as_ref();
        let Some(config) = telemetry.filter(|config| config.enabled) else {
            self.stop_telemetry_exporter().await;
            return;
        };

        self.telemetry.update_from_control(config);
        let mut exporter = self.telemetry_exporter.lock().await;
        if let Some(existing) = exporter.take() {
            existing.shutdown(EXPORTER_SHUTDOWN_FLUSH_TIMEOUT).await;
        }
        *exporter =
            Some(TelemetryExporter::new(config.endpoint.clone(), self.telemetry.clone()).start());
    }

    pub(super) async fn stop_telemetry_exporter(&self) {
        if let Some(exporter) = self.telemetry_exporter.lock().await.take() {
            exporter.shutdown(EXPORTER_SHUTDOWN_FLUSH_TIMEOUT).await;
        }
    }

    pub(crate) fn queue_connection_registration(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        self.queue_runtime_capabilities(sender.clone())?;
        if let Some(register) = self
            .active_assembly_registration()
            .map_err(|error| RuntimeError::Decode(error.to_string()))?
        {
            Self::queue_assembly_activation(sender, &register)?;
        }
        Ok(())
    }

    #[cfg(any())]
    pub(crate) fn queue_registers(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        self.queue_connection_registration(sender)
    }

    fn queue_runtime_capabilities(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        let header = RuntimeCapabilitiesFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.capabilities".to_string(),
            runtime_id: self.base_runtime_id.clone(),
            capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
                package_test_dispatch: false,
                request_cancel: true,
                ..RuntimeCapabilitiesFrameHeaderMetadata::default()
            },
        };
        let frame = encode_binary_frame(&header, &[])
            .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        sender
            .send(RouterWriterMessage::Binary(frame))
            .map_err(|_| RuntimeError::Decode("runtime writer channel closed".to_string()))?;
        Ok(())
    }

    pub(crate) fn queue_assembly_activation(
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        control: &AssemblyActivationControl,
    ) -> Result<()> {
        let frame = encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RuntimeToRouter,
            control,
        )
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        sender
            .send(RouterWriterMessage::Binary(frame))
            .map_err(|_| RuntimeError::Decode("runtime writer channel closed".to_string()))
    }

    pub(super) fn log_registered(&self, rest: &serde_json::Map<String, Value>) {
        let runtime_id = rest.get("runtimeId").and_then(Value::as_str);
        if let Some(runtime_id) = runtime_id {
            info!(
                event = "runtime.registered",
                runtime_id = %runtime_id
            );
        } else {
            warn!(event = "runtime.registered_unknown", runtime_id = "unknown");
        }
    }

    #[cfg(any())]
    fn emit_runtime_control_reload(&self, control: &RouterControlEnvelope) {
        let mut attrs = serde_json::Map::new();
        if let Ok(artifact_roots) = control.ordered_artifact_roots() {
            attrs.insert(
                "artifactRoots".to_string(),
                Value::Array(
                    artifact_roots
                        .into_iter()
                        .map(|root| Value::String(root.display().to_string()))
                        .collect(),
                ),
            );
        }
        if let Some(generation) = control.generation.as_deref() {
            attrs.insert(
                "generation".to_string(),
                Value::String(generation.to_string()),
            );
        }
        attrs.insert(
            "telemetryEnabled".to_string(),
            Value::Bool(
                control
                    .telemetry
                    .as_ref()
                    .is_some_and(|config| config.enabled),
            ),
        );
        let mut event = telemetry_event(
            TelemetryTopic::Trace,
            telemetry_timestamp_now(),
            TelemetrySource::Runtime,
        );
        event.runtime_id = Some(self.base_runtime_id.clone());
        event.name = Some("runtime.control.reload".to_string());
        event.attrs = Some(attrs);
        self.telemetry.emit(event);
    }

    pub(crate) fn emit_request_route_error(&self, request: &RequestEnvelope, error: &RuntimeError) {
        let Some(response_error) = response_error_from_runtime_error(error) else {
            return;
        };
        let mut event = telemetry_event(
            TelemetryTopic::Trace,
            telemetry_timestamp_now(),
            TelemetrySource::Runtime,
        );
        event.runtime_id = Some(self.base_runtime_id.clone());
        event.request_id = Some(request.request_id.clone());
        event.target = Some(request.target.clone());
        event.build_id = Some(request.build_id().to_string());
        event.activation_identity = request.activation_identity.clone();
        event.name = Some("request.error".to_string());
        event.error = Some(request_runner::response_error_to_telemetry_map(
            &response_error,
        ));
        apply_request_trace_fields(&mut event, request);
        self.telemetry.emit(event);
    }
}

fn apply_request_trace_fields(event: &mut TelemetryEvent, request: &RequestEnvelope) {
    request_trace::RequestTraceFields::from_request(request).apply_to_event(event);
}
