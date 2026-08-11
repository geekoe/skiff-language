#[cfg(any())]
use std::{collections::HashSet, sync::Arc};

use serde_json::Value;
#[cfg(any())]
use serde_json::{json, Map};
#[cfg(any())]
use skiff_artifact_model::ConfigShape;
use skiff_artifact_model::{GatewayDispatchMode, GatewayEntryProtocolSurface, GatewayProtocolSurface};
use skiff_runtime_request::{self as request_runner, RequestEnvelope, RouterWriterMessage};
#[cfg(any())]
use skiff_runtime_transport::protocol::{
    RouterControlEnvelope, RouterControlPackageConfig, RouterControlServiceConfig,
};
use skiff_runtime_transport::{
    protocol::{
        encode_binary_frame, RuntimeCapabilitiesFrameHeader,
        RuntimeCapabilitiesFrameHeaderMetadata, RuntimeDispatchModeCapability, TelemetryEvent,
        TelemetrySource, RUNTIME_FRAME_SCHEMA_VERSION,
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

    /// The single connection registration step: advertise capabilities only.
    /// There is no Register frame; the loaded buildId set and the artifact
    /// root are the whole registration payload.
    pub(crate) fn queue_connection_registration(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        self.queue_runtime_capabilities(sender)
    }

    #[cfg(any())]
    pub(crate) fn queue_registers(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        self.queue_connection_registration(sender)
    }

    pub(crate) fn queue_runtime_capabilities(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        // Capability advertisement is derived from the loaded deployment set
        // (M2 lazy-load registry): HTTP and WebSocket gateway surfaces
        // advertise exactly the dispatch modes they project, plus the artifact
        // root and loaded buildId set for lazy-load routing.
        //
        // M5 fix: a lazy-load holder can execute ANY engine-supported dispatch
        // mode even when nothing is loaded yet (cold start), so the advertised
        // modes are the engine superset — the router's capability gate would
        // otherwise exclude every cold-start runtime and every request would
        // fail with no eligible runtime (the deployment's own surface decides
        // the request shape after lazy loading).
        let _ = self.assembly_admission.loaded_gateway_surfaces();
        let dispatch_modes = engine_dispatch_modes();
        let loaded_build_ids = self.assembly_admission.loaded_build_ids();
        let artifact_root = self.bootstrap_artifact_root();
        let header = RuntimeCapabilitiesFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.capabilities".to_string(),
            runtime_id: self.base_runtime_id.clone(),
            capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
                dispatch_modes,
                package_test_dispatch: false,
                request_cancel: true,
                artifact_root,
                lazy_load: true,
                loaded_build_ids,
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
        let mut event = telemetry_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
        event.runtime_id = Some(self.base_runtime_id.clone());
        event.name = Some("runtime.control.reload".to_string());
        event.attrs = Some(attrs);
        self.telemetry.emit(event);
    }

    pub(crate) fn emit_request_route_error(&self, request: &RequestEnvelope, error: &RuntimeError) {
        let Some(response_error) = response_error_from_runtime_error(error) else {
            return;
        };
        let mut event = telemetry_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
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

/// Projection of the advertised dispatch-mode capability list from the
/// admitted assembly's gateway entry surfaces.
///
/// HTTP surfaces contribute their `dispatch_mode` (unary / serverStream);
/// WebSocket connect surfaces contribute unary (the connect handshake is a
/// unary dispatch), and WebSocket JSON-RPC surfaces contribute their
/// `dispatch_mode`. Order is fixed: `[unary, serverStream]`.
fn engine_dispatch_modes() -> Vec<RuntimeDispatchModeCapability> {
    vec![
        RuntimeDispatchModeCapability::Unary,
        RuntimeDispatchModeCapability::ServerStream,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::runtime_host::RuntimeConfig;
    use skiff_artifact_model::{
        GatewayAdapterKind, GatewayExternalErrorProjection, GatewayExternalSchema,
        GatewayHttpProtocolSurface, GatewayWebSocketConnectProtocolSurface,
        GatewayWebSocketJsonRpcProtocolSurface, GatewayWebSocketRpcProfile,
        GatewayWebSocketShapeVersion,
    };




    #[test]
    fn capabilities_frame_advertises_engine_modes_without_admitted_assembly() {
        let host = RuntimeHost::new(RuntimeConfig {
            db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
            router_url: "ws://127.0.0.1:4001/runtime".to_string(),
            base_runtime_id: "runtime-no-admit".to_string(),
            runtime_home: std::env::temp_dir().join("skiff-runtime-no-admit-home"),
            profile: "test".to_string(),
            bytecode_only: false,
            http_response_max_bytes: 1024,
            http_egress_proxy: None,
        })
        .expect("runtime host");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        host.queue_runtime_capabilities(sender)
            .expect("queue capabilities");
        let message = receiver.blocking_recv().expect("capabilities frame");
        let RouterWriterMessage::Binary(bytes) = message else {
            panic!("capabilities must be a binary frame");
        };
        let (header, _) = skiff_runtime_transport::protocol::decode_typed_binary_frame::<
            RuntimeCapabilitiesFrameHeader,
        >(&bytes)
        .expect("decode capabilities frame");
        assert_eq!(header.runtime_id, "runtime-no-admit");
        assert_eq!(
            header.capabilities.dispatch_modes,
            engine_dispatch_modes(),
            "cold-start lazy-load holder advertises every engine-supported mode"
        );
        assert!(header.capabilities.lazy_load);
        assert!(header.capabilities.loaded_build_ids.is_empty());
        assert!(header.capabilities.request_cancel);
    }
}
