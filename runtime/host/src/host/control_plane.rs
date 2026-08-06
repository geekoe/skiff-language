#[cfg(any())]
use std::{collections::HashSet, sync::Arc};

use serde_json::Value;
#[cfg(any())]
use serde_json::{json, Map};
#[cfg(any())]
use skiff_artifact_model::ConfigShape;
use skiff_artifact_model::{
    AssemblyActivationControl, GatewayDispatchMode, GatewayEntryProtocolSurface,
    GatewayProtocolSurface,
};
use skiff_runtime_request::{self as request_runner, RequestEnvelope, RouterWriterMessage};
#[cfg(any())]
use skiff_runtime_transport::protocol::{
    RouterControlEnvelope, RouterControlPackageConfig, RouterControlServiceConfig,
};
use skiff_runtime_transport::{
    assembly_activation::{encode_assembly_activation_frame, AssemblyActivationFrameDirection},
    protocol::{
        encode_binary_frame, RuntimeCapabilitiesFrameHeader,
        RuntimeCapabilitiesFrameHeaderMetadata, RuntimeDispatchModeCapability, TelemetryEvent,
        TelemetrySource, TelemetryTopic, RUNTIME_FRAME_SCHEMA_VERSION,
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

    pub(crate) fn queue_runtime_capabilities(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        // Capability advertisement is derived from the loaded deployment set
        // (M2 lazy-load registry): HTTP and WebSocket gateway surfaces
        // advertise exactly the dispatch modes they project, plus the artifact
        // root and loaded buildId set for lazy-load routing. An empty loaded
        // set means no dispatch capability and nothing loaded yet (fail closed).
        let surfaces = self.assembly_admission.loaded_gateway_surfaces();
        let dispatch_modes = dispatch_modes_from_gateway_entries(surfaces.iter());
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

/// Projection of the advertised dispatch-mode capability list from the
/// admitted assembly's gateway entry surfaces.
///
/// HTTP surfaces contribute their `dispatch_mode` (unary / serverStream);
/// WebSocket connect surfaces contribute unary (the connect handshake is a
/// unary dispatch), and WebSocket JSON-RPC surfaces contribute their
/// `dispatch_mode`. Order is fixed: `[unary, serverStream]`.
fn dispatch_modes_from_gateway_entries<'a>(
    entries: impl IntoIterator<Item = &'a GatewayEntryProtocolSurface>,
) -> Vec<RuntimeDispatchModeCapability> {
    let mut unary = false;
    let mut server_stream = false;
    for surface in entries {
        match &surface.protocol {
            GatewayProtocolSurface::Http(http) => match http.dispatch_mode {
                GatewayDispatchMode::Unary => unary = true,
                GatewayDispatchMode::ServerStream => server_stream = true,
            },
            GatewayProtocolSurface::WebSocketConnect(_) => unary = true,
            GatewayProtocolSurface::WebSocketJsonRpc(rpc) => match rpc.dispatch_mode {
                GatewayDispatchMode::Unary => unary = true,
                GatewayDispatchMode::ServerStream => server_stream = true,
            },
        }
    }
    let mut modes = Vec::new();
    if unary {
        modes.push(RuntimeDispatchModeCapability::Unary);
    }
    if server_stream {
        modes.push(RuntimeDispatchModeCapability::ServerStream);
    }
    modes
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

    fn http_surface(mode: GatewayDispatchMode) -> GatewayEntryProtocolSurface {
        GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::TypedJson,
                dispatch_mode: mode,
                external_sources: Vec::new(),
                request_body_schema: None,
                response_schema: None,
                stream_item_schema: None,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        }
    }

    fn websocket_surface() -> GatewayEntryProtocolSurface {
        GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::WebSocketConnect(
                GatewayWebSocketConnectProtocolSurface {
                    connect_request_shape: GatewayWebSocketShapeVersion::V1,
                    connect_result_shape: GatewayWebSocketShapeVersion::V1,
                    connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                    external_sources: Vec::new(),
                    downlink_frames: Vec::new(),
                    rpc_profiles: Vec::new(),
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        }
    }

    fn websocket_json_rpc_surface(mode: GatewayDispatchMode) -> GatewayEntryProtocolSurface {
        GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::WebSocketJsonRpc(
                GatewayWebSocketJsonRpcProtocolSurface {
                    profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                    dispatch_mode: mode,
                    external_sources: Vec::new(),
                    params_schema: GatewayExternalSchema::Null,
                    result_schema: GatewayExternalSchema::Null,
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        }
    }

    #[test]
    fn dispatch_modes_are_empty_without_gateway_surfaces() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(std::iter::empty()),
            Vec::new()
        );
    }

    #[test]
    fn dispatch_modes_advertise_unary_for_websocket_connect_surface() {
        assert_eq!(
            dispatch_modes_from_gateway_entries([websocket_surface()].iter()),
            vec![RuntimeDispatchModeCapability::Unary]
        );
    }

    #[test]
    fn dispatch_modes_advertise_unary_for_websocket_json_rpc_surface() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(
                [websocket_json_rpc_surface(GatewayDispatchMode::Unary)].iter()
            ),
            vec![RuntimeDispatchModeCapability::Unary]
        );
    }

    #[test]
    fn dispatch_modes_advertise_server_stream_for_websocket_json_rpc_surface() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(
                [websocket_json_rpc_surface(
                    GatewayDispatchMode::ServerStream
                )]
                .iter()
            ),
            vec![RuntimeDispatchModeCapability::ServerStream]
        );
    }

    #[test]
    fn dispatch_modes_advertise_unary_surface() {
        assert_eq!(
            dispatch_modes_from_gateway_entries([http_surface(GatewayDispatchMode::Unary)].iter()),
            vec![RuntimeDispatchModeCapability::Unary]
        );
    }

    #[test]
    fn dispatch_modes_advertise_server_stream_surface() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(
                [http_surface(GatewayDispatchMode::ServerStream)].iter()
            ),
            vec![RuntimeDispatchModeCapability::ServerStream]
        );
    }

    #[test]
    fn dispatch_modes_advertise_both_in_fixed_order() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(
                [
                    http_surface(GatewayDispatchMode::ServerStream),
                    websocket_surface(),
                    http_surface(GatewayDispatchMode::Unary),
                ]
                .iter()
            ),
            vec![
                RuntimeDispatchModeCapability::Unary,
                RuntimeDispatchModeCapability::ServerStream,
            ]
        );
    }

    #[test]
    fn dispatch_modes_advertise_http_only_deployment() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(
                [
                    http_surface(GatewayDispatchMode::Unary),
                    http_surface(GatewayDispatchMode::ServerStream),
                ]
                .iter()
            ),
            vec![
                RuntimeDispatchModeCapability::Unary,
                RuntimeDispatchModeCapability::ServerStream,
            ]
        );
    }

    #[test]
    fn dispatch_modes_advertise_websocket_only_deployment() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(
                [
                    websocket_surface(),
                    websocket_json_rpc_surface(GatewayDispatchMode::Unary),
                ]
                .iter()
            ),
            vec![RuntimeDispatchModeCapability::Unary]
        );
    }

    #[test]
    fn dispatch_modes_advertise_mixed_http_and_websocket_deployment() {
        assert_eq!(
            dispatch_modes_from_gateway_entries(
                [
                    websocket_surface(),
                    http_surface(GatewayDispatchMode::ServerStream),
                    websocket_json_rpc_surface(GatewayDispatchMode::Unary),
                    http_surface(GatewayDispatchMode::Unary),
                ]
                .iter()
            ),
            vec![
                RuntimeDispatchModeCapability::Unary,
                RuntimeDispatchModeCapability::ServerStream,
            ]
        );
    }

    #[test]
    fn capabilities_frame_stays_empty_without_admitted_assembly() {
        let host = RuntimeHost::new(RuntimeConfig {
            db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
            router_url: "ws://127.0.0.1:4001/runtime".to_string(),
            base_runtime_id: "runtime-no-admit".to_string(),
            runtime_home: std::env::temp_dir().join("skiff-runtime-no-admit-home"),
            profile: "test".to_string(),
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
        assert!(header.capabilities.dispatch_modes.is_empty());
        assert!(header.capabilities.request_cancel);
    }
}
