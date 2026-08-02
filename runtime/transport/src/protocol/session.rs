use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_model::{
    validate_activation_environment, validate_activation_generation, RuntimeAssemblyRef,
    RuntimeConfigSnapshotRef,
};

use crate::{
    protocol::{frame::RUNTIME_FRAME_SCHEMA_VERSION, request::is_false},
    BinaryFrameError, TransportError,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRegisterFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
    pub service_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub build_id: String,
    pub revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    pub service_protocol_identity: String,
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gateway_entry_identities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<RuntimeCapabilitiesFrameHeaderMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCapabilitiesFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
    pub capabilities: RuntimeCapabilitiesFrameHeaderMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCapabilitiesFrameHeaderMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatch_modes: Vec<RuntimeDispatchModeCapability>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub package_test_dispatch: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub request_cancel: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub runtime_program: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeDispatchModeCapability {
    #[serde(rename = "unary")]
    Unary,
    #[serde(rename = "serverStream")]
    ServerStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeHealthCountersFrameHeader {
    pub outbound_requests_pending: usize,
    pub outbound_stream_leases_active: usize,
    pub stream_runtime_streams_active: usize,
    pub flag_backed_cancel_waiters_active: usize,
    pub spawned_tasks_active: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeHealthFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
    pub observed_at: String,
    pub counters: RuntimeHealthCountersFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRegisteredFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterBootstrapServiceDbFrameHeader {
    pub mongo_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterBootstrapHttpFrameHeader {
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterBootstrapActivationFrameHeader {
    pub environment: String,
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterBootstrapFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub artifacts_path: String,
    pub service_db: RouterBootstrapServiceDbFrameHeader,
    pub http: RouterBootstrapHttpFrameHeader,
    pub activation: RouterBootstrapActivationFrameHeader,
}

pub fn decode_router_bootstrap_frame_header(
    value: Value,
) -> std::result::Result<RouterBootstrapFrameHeader, BinaryFrameError> {
    let header: RouterBootstrapFrameHeader = serde_json::from_value(value).map_err(|error| {
        TransportError::decode(format!(
            "invalid router.bootstrap frame header: typed decode failed: {error}"
        ))
    })?;
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(TransportError::decode(format!(
            "invalid router.bootstrap frame header: schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        )));
    }
    if header.envelope_type != "router.bootstrap" {
        return Err(TransportError::decode(
            "invalid router.bootstrap frame header: type must be router.bootstrap",
        ));
    }
    if !is_normalized_absolute_artifacts_path(&header.artifacts_path) {
        return Err(TransportError::decode(
            "invalid router.bootstrap frame header: artifactsPath must be an absolute normalized path",
        ));
    }
    if header.service_db.mongo_url.trim().is_empty() {
        return Err(TransportError::decode(
            "invalid router.bootstrap frame header: serviceDb.mongoUrl must be a non-empty string",
        ));
    }
    if header.http.max_response_bytes == 0 || header.http.max_response_bytes > 9_007_199_254_740_991
    {
        return Err(TransportError::decode(
            "invalid router.bootstrap frame header: http.maxResponseBytes must be a positive safe integer",
        ));
    }
    validate_activation_environment(&header.activation.environment).map_err(|error| {
        TransportError::decode(format!(
            "invalid router.bootstrap frame header: activation.environment {error}"
        ))
    })?;
    validate_activation_generation(header.activation.generation, "activation.generation").map_err(
        |error| TransportError::decode(format!("invalid router.bootstrap frame header: {error}")),
    )?;
    Ok(header)
}

fn is_normalized_absolute_artifacts_path(value: &str) -> bool {
    if !value.starts_with('/') || (value.len() > 1 && value.ends_with('/')) {
        return false;
    }
    value == "/"
        || value[1..]
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRegisterEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: &'static str,
    pub runtime_id: String,
    pub service_id: String,
    // Stable published version this runtime serves. The router indexes
    // (service_id, version) -> current build_id from on-disk pointer records and
    // uses version as the cross-service addressing key; this field lets the live
    // registry confirm which version a registered build belongs to.
    pub version: String,
    pub build_id: String,
    // Compatibility fields required by the current TypeScript router.
    pub revision_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    pub service_protocol_identity: String,
    pub contract_identity: String,
    pub targets: Vec<String>,
    pub runtime_version: String,
    pub code_revision_id: String,
    pub implementation_identity: String,
    pub artifact_identity: String,
    pub capabilities: RuntimeCapabilitiesFrameHeaderMetadata,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gateway_entry_identities: Vec<String>,
}

impl From<RuntimeRegisterEnvelope> for RuntimeRegisterFrameHeader {
    fn from(envelope: RuntimeRegisterEnvelope) -> Self {
        Self {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: envelope.envelope_type.to_string(),
            runtime_id: envelope.runtime_id,
            service_id: envelope.service_id,
            version: Some(envelope.version),
            build_id: envelope.build_id,
            revision_id: envelope.revision_id,
            activation_identity: envelope.activation_identity,
            service_protocol_identity: envelope.service_protocol_identity,
            targets: envelope.targets,
            runtime_version: Some(envelope.runtime_version),
            code_revision_id: Some(envelope.code_revision_id),
            artifact_identity: Some(envelope.artifact_identity),
            gateway_entry_identities: envelope.gateway_entry_identities,
            capabilities: Some(envelope.capabilities),
        }
    }
}
