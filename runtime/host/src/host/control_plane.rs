use std::{collections::HashSet, sync::Arc};

use serde_json::{json, Map, Value};
use skiff_artifact_model::ConfigShape;
use skiff_runtime_linked_program::{package_config_shape, LinkedProgramImage};
use skiff_runtime_request::{self as request_runner, RequestEnvelope, RouterWriterMessage};
use skiff_runtime_transport::protocol::{
    encode_binary_frame, RouterControlEnvelope, RouterControlPackageConfig,
    RouterControlServiceConfig, RuntimeCapabilitiesFrameHeader,
    RuntimeCapabilitiesFrameHeaderMetadata, RuntimeRegisterFrameHeader, TelemetryEvent,
    TelemetrySource, TelemetryTopic, RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{
    capability_context::{response_error_from_runtime_error, DbProviderConfig},
    config_view::RuntimeConfigView,
    error::{Result, RuntimeError},
    loader::{artifact_roots_control_fingerprint, ArtifactLoadOptions},
    telemetry::{telemetry_event, telemetry_timestamp_now, RequestTelemetryContext},
};

use super::telemetry::{TelemetryExporter, EXPORTER_SHUTDOWN_FLUSH_TIMEOUT};
use super::{
    blob_store::{blob_store_from_control, BlobStore},
    register_mapper::runtime_register_envelope_from_program_layers,
    request_trace, route_registry, ArtifactLoadState, RuntimeHost, RuntimeServiceConfig,
    ServiceRuntimeContext,
};

impl RuntimeHost {
    pub(super) async fn reload_from_control(
        &self,
        control: &RouterControlEnvelope,
        _sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<String> {
        control
            .reject_legacy_config_fields()
            .map_err(RuntimeError::invalid_artifact)?;
        self.apply_telemetry_control(control).await;
        self.apply_file_backend_control(control)?;
        self.emit_runtime_control_reload(control);
        let control_artifact_roots = control
            .ordered_artifact_roots()
            .map_err(RuntimeError::invalid_artifact)?;
        let fingerprint = router_control_fingerprint(control)
            .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
        let load_options = ArtifactLoadOptions::from_control(control.dev_reload);
        let artifact_roots = if self.configured_artifact_roots.is_empty() {
            control_artifact_roots.clone()
        } else {
            self.configured_artifact_roots.as_ref().clone()
        };
        let mut state = self.artifact_load_state.lock().await;
        let epoch = state.epoch.wrapping_add(1);
        *state = ArtifactLoadState {
            artifact_roots: artifact_roots.clone(),
            load_options,
            service_config: control.service_config.clone(),
            epoch,
        };
        let cleared_package_test_templates = self.artifact_caches.package_test_templates.clear();
        info!(
            event = "runtime.artifact_loader_configured",
            artifacts = %artifact_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(","),
            dev_reload = control.dev_reload.unwrap_or(false),
            generation = control.generation.as_deref().unwrap_or("unknown")
        );
        if cleared_package_test_templates > 0 {
            info!(
                event = "runtime.package_test_template_cache_cleared",
                entries = cleared_package_test_templates
            );
        }
        Ok(fingerprint)
    }

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

    pub(crate) fn queue_registers(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        self.queue_runtime_capabilities(sender.clone())?;
        let services = self.service_snapshot();
        self.queue_service_registers(sender, &services)
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
                package_test_dispatch: true,
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

    pub(super) fn queue_service_registers(
        &self,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        services: &[Arc<ServiceRuntimeContext>],
    ) -> Result<()> {
        for service in services {
            let register = runtime_register_envelope_from_program_layers(
                service.runtime_id.clone(),
                &service.runtime_program_identity,
                service.linked_image.as_ref(),
                service.runtime_activation.as_ref(),
                service.revision_id.clone(),
                service.contract_identity.clone(),
                service.implementation_identity.clone(),
                service.artifact_identity.clone(),
                service.activation_identity.clone(),
            )?;
            info!(
                event = "runtime.registering",
                runtime_id = %service.runtime_id,
                service_id = %service.service_id,
                version = %register.version,
                build_id = %service.build_id,
                dynamic_build_id = %service.runtime_program_identity.dynamic_build_id,
                linked_image_identity = %service.runtime_program_identity.linked_image_identity,
                contract_identity = %service.contract_identity,
                implementation_identity = %service.implementation_identity
            );
            self.emit_service_trace(service, "runtime.register", None, None, None);
            let header = RuntimeRegisterFrameHeader::from(register);
            let frame = encode_binary_frame(&header, &[])
                .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            sender
                .send(RouterWriterMessage::Binary(frame))
                .map_err(|_| RuntimeError::Decode("runtime writer channel closed".to_string()))?;
        }
        Ok(())
    }

    pub(super) fn log_registered(&self, rest: &serde_json::Map<String, Value>) {
        let runtime_id = rest.get("runtimeId").and_then(Value::as_str);
        let service = runtime_id.and_then(|runtime_id| {
            self.service_snapshot()
                .iter()
                .find(|service| service.runtime_id == runtime_id)
                .cloned()
        });
        if let (Some(runtime_id), Some(service)) = (runtime_id, service) {
            info!(
                event = "runtime.registered",
                runtime_id = %runtime_id,
                service_id = %service.service_id
            );
            self.emit_service_trace(&service, "runtime.registered", None, None, None);
        } else {
            warn!(
                event = "runtime.registered_unknown",
                runtime_id = runtime_id.unwrap_or("unknown")
            );
        }
    }

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

    pub(super) fn emit_service_trace(
        &self,
        service: &ServiceRuntimeContext,
        name: &str,
        target: Option<String>,
        duration_ms: Option<f64>,
        error: Option<serde_json::Map<String, Value>>,
    ) {
        let mut event = telemetry_event(
            TelemetryTopic::Trace,
            telemetry_timestamp_now(),
            TelemetrySource::Runtime,
        );
        event.service_id = Some(service.service_id.clone());
        event.build_id = Some(service.build_id.clone());
        event.activation_identity = service.activation_identity.clone();
        event.runtime_id = Some(service.runtime_id.clone());
        event.name = Some(name.to_string());
        event.target = target;
        event.duration_ms = duration_ms;
        event.error = error;
        self.telemetry.emit(event);
    }

    pub(crate) fn emit_request_route_error(&self, request: &RequestEnvelope, error: &RuntimeError) {
        let response_error = response_error_from_runtime_error(error);
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

    pub(crate) fn request_telemetry_context(
        &self,
        request: &RequestEnvelope,
        service: &ServiceRuntimeContext,
    ) -> RequestTelemetryContext {
        let trace = request_trace::RequestTraceFields::from_request(request);
        let mut context = RequestTelemetryContext::new(self.telemetry.clone());
        context.service_id = Some(service.service_id.clone());
        context.build_id = Some(service.build_id.clone());
        context.activation_identity = service
            .activation_identity
            .clone()
            .or_else(|| request.activation_identity.clone());
        context.runtime_id = Some(service.runtime_id.clone());
        context.request_id = Some(request.request_id.clone());
        context.trace_id = trace.trace_id;
        context.span_id = trace.span_id;
        context.parent_span_id = trace.parent_span_id;
        context.target = Some(request.target.clone());
        context
    }
}

fn router_control_fingerprint(control: &RouterControlEnvelope) -> anyhow::Result<String> {
    if control.dev_reload.unwrap_or(false) {
        return artifact_roots_control_fingerprint(
            &control
                .ordered_artifact_roots()
                .map_err(|error| anyhow::anyhow!(error))?,
            control.dev_reload,
        );
    }
    if let Some(fingerprint) = control.fingerprint.as_deref() {
        return Ok(fingerprint.to_string());
    }
    Ok(control
        .generation
        .as_deref()
        .map(|generation| format!("generation:{generation}"))
        .unwrap_or_else(|| "release-control".to_string()))
}

pub(crate) fn apply_control_config(
    services: Vec<RuntimeServiceConfig>,
    service_config: &[RouterControlServiceConfig],
) -> anyhow::Result<Vec<RuntimeServiceConfig>> {
    if service_config.is_empty() {
        return Ok(services);
    }

    let mut expanded = Vec::new();
    for service in services {
        let service_id = route_registry::runtime_service_identity(&service)?.0;
        let selector_build_id =
            route_registry::runtime_service_selector_build_id(&service).to_string();
        let matching = service_config
            .iter()
            .filter(|config| {
                config.service_id == service_id && config.build_id == selector_build_id
            })
            .collect::<Vec<_>>();
        if matching.is_empty() {
            expanded.push(service);
            continue;
        }
        for config in matching {
            if !config.redacted_resolved_config.is_null()
                && config.redaction_projection_identity.as_deref().is_none()
            {
                anyhow::bail!(
                    "router.control serviceConfig redactionProjectionIdentity is required when redactedResolvedConfig is present"
                );
            }
            let config_shape = config
                .config_shape
                .clone()
                .unwrap_or_else(|| service.config.config_shape().clone());
            let runtime_config = if config.redacted_resolved_config.is_null() {
                RuntimeConfigView::from_resolved_config(
                    config.resolved_config.clone(),
                    config_shape,
                )?
            } else {
                RuntimeConfigView::from_resolved_config_with_redaction(
                    config.resolved_config.clone(),
                    config_shape,
                    config.redacted_resolved_config.clone(),
                )?
            };
            let mut activated = service.clone();
            activated.activation_identity = Some(config.activation_identity.clone());
            activated.resolved_config_identity = Some(config.resolved_config_identity.clone());
            activated.config = runtime_config;
            activated.package_configs = apply_package_control_config(&service, config)?;
            activated.service_db = config.service_db.as_ref().map(|service_db| {
                DbProviderConfig::opaque(json!({
                    "mongoUrl": service_db.mongo_url.clone(),
                    "storageServiceId": service_db.storage_service_id.clone(),
                }))
            });
            if !activated.runtime_id.contains(&config.activation_identity) {
                activated.runtime_id = format!(
                    "{}:act:{}",
                    activated.runtime_id, config.activation_identity
                );
            }
            expanded.push(activated);
        }
    }
    Ok(expanded)
}

fn apply_package_control_config(
    service: &RuntimeServiceConfig,
    config: &RouterControlServiceConfig,
) -> anyhow::Result<Vec<RuntimeConfigView>> {
    let mut package_configs = service.package_configs.clone();
    if config.package_configs.is_empty() {
        return Ok(package_configs);
    }
    while package_configs.len() < service.linked_image.packages.len() {
        let slot = package_configs.len();
        package_configs.push(RuntimeConfigView::empty_unvalidated_with_shape(
            package_config_shape_for_slot(service.linked_image.as_ref(), slot)?,
        ));
    }

    let mut seen_slots = HashSet::new();
    for (index, package_config) in config.package_configs.iter().enumerate() {
        if package_config.resolved_config_identity.trim().is_empty() {
            anyhow::bail!(
                "router.control serviceConfig packageConfigs[{index}] resolvedConfigIdentity must be non-empty"
            );
        }
        if !package_config.redacted_resolved_config.is_null()
            && package_config
                .redaction_projection_identity
                .as_deref()
                .is_none()
        {
            anyhow::bail!(
                "router.control serviceConfig packageConfigs redactionProjectionIdentity is required when redactedResolvedConfig is present"
            );
        }
        let slot = resolve_control_package_slot(service, package_config, index)?;
        if !seen_slots.insert(slot) {
            anyhow::bail!(
                "router.control serviceConfig packageConfigs declares duplicate package slot {slot}"
            );
        }
        let default_config_view = package_configs.get(slot);
        let default_config = default_config_view
            .map(|config| config.resolved_config_value().clone())
            .unwrap_or_else(|| Value::Object(Map::new()));
        let config_shape = match &package_config.config_shape {
            Some(config_shape) => config_shape.clone(),
            None => match default_config_view {
                Some(config) => config.config_shape().clone(),
                None => package_config_shape_for_slot(service.linked_image.as_ref(), slot)?,
            },
        };
        let resolved_config = merge_config_object_overlay(
            default_config.clone(),
            package_config.resolved_config.clone(),
            "router.control serviceConfig packageConfigs resolvedConfig",
        )?;
        let runtime_config = if package_config.redacted_resolved_config.is_null() {
            RuntimeConfigView::from_resolved_config(resolved_config, config_shape)?
        } else {
            let redacted_resolved_config = merge_config_object_overlay(
                default_config,
                package_config.redacted_resolved_config.clone(),
                "router.control serviceConfig packageConfigs redactedResolvedConfig",
            )?;
            RuntimeConfigView::from_resolved_config_with_redaction(
                resolved_config,
                config_shape,
                redacted_resolved_config,
            )?
        };
        package_configs[slot] = runtime_config;
    }
    Ok(package_configs)
}

fn package_config_shape_for_slot(
    linked_image: &LinkedProgramImage,
    slot: usize,
) -> anyhow::Result<ConfigShape> {
    linked_image
        .packages
        .get(slot)
        .map(|package| package_config_shape(package.as_ref()))
        .transpose()
        .map(|shape| shape.unwrap_or_else(ConfigShape::empty))
}

fn resolve_control_package_slot(
    service: &RuntimeServiceConfig,
    package_config: &RouterControlPackageConfig,
    index: usize,
) -> anyhow::Result<usize> {
    let slot = if let Some(slot) = package_config.package_slot {
        slot
    } else if !package_config.alias.trim().is_empty() {
        service
            .linked_image
            .link_overlay
            .package_slot_for_dependency_ref(&package_config.alias)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "router.control serviceConfig packageConfigs[{index}] alias {} is not loaded",
                    package_config.alias
                )
            })?
    } else {
        service
            .linked_image
            .packages
            .iter()
            .position(|package| package.package_id == package_config.package_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "router.control serviceConfig packageConfigs[{index}] packageId {} is not loaded",
                    package_config.package_id
                )
            })?
    };
    let package = service
        .linked_image
        .packages
        .get(slot)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "router.control serviceConfig packageConfigs[{index}] packageSlot {slot} is out of range"
            )
        })?;
    if package.package_id != package_config.package_id {
        anyhow::bail!(
            "router.control serviceConfig packageConfigs[{index}] packageId {} does not match packageSlot {slot} packageId {}",
            package_config.package_id,
            package.package_id
        );
    }
    Ok(slot)
}

fn merge_config_object_overlay(base: Value, overlay: Value, label: &str) -> anyhow::Result<Value> {
    let mut base = match base {
        Value::Null => Map::new(),
        Value::Object(object) => object,
        _ => anyhow::bail!("{label} artifact default must be a JSON object"),
    };
    let overlay = match overlay {
        Value::Null => Map::new(),
        Value::Object(object) => object,
        _ => anyhow::bail!("{label} must be a JSON object"),
    };
    overlay_config_map(&mut base, overlay);
    Ok(Value::Object(base))
}

fn overlay_config_map(target: &mut Map<String, Value>, overlay: Map<String, Value>) {
    for (key, value) in overlay {
        if value.is_null() {
            target.remove(&key);
            continue;
        }
        if let Value::Object(overlay_object) = value {
            if let Some(Value::Object(target_object)) = target.get_mut(&key) {
                overlay_config_map(target_object, overlay_object);
            } else {
                target.insert(key, Value::Object(overlay_object));
            }
            continue;
        }
        target.insert(key, value);
    }
}

fn apply_request_trace_fields(event: &mut TelemetryEvent, request: &RequestEnvelope) {
    request_trace::RequestTraceFields::from_request(request).apply_to_event(event);
}
