use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_model::{
    validate_activation_environment, validate_activation_generation,
    validate_runtime_assembly_identity, validate_runtime_config_snapshot_id, AssemblyIdentity,
    RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};

use crate::{
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, frame::RUNTIME_FRAME_SCHEMA_VERSION,
        request::is_false,
    },
    BinaryFrameError, TransportError,
};

pub const ROUTER_BOOTSTRAP_FRAME_TYPE: &str = "router.bootstrap";
pub const RUNTIME_CAPABILITIES_FRAME_TYPE: &str = "runtime.capabilities";
pub const RUNTIME_REGISTERED_FRAME_TYPE: &str = "runtime.registered";
pub const RUNTIME_HEALTH_FRAME_TYPE: &str = "runtime.health";

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
    pub task_requests_active: usize,
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
    if header.envelope_type != ROUTER_BOOTSTRAP_FRAME_TYPE {
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

/// Captured activation tuple used to construct the `router.bootstrap` frame
/// (§3.3/§3.5, C-model-bootstrap-wire §2.3). W-bootstrap later maps the
/// durable `RoutingEpoch` onto this wire-facing view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedBootstrapEpoch {
    pub environment: String,
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
}

impl CapturedBootstrapEpoch {
    /// Strict constructor: every wire-visible field is validated the same way
    /// the typed bootstrap header Deserialize path validates it.
    pub fn new(
        environment: impl Into<String>,
        generation: u64,
        assembly_identity: impl Into<String>,
        config_snapshot_id: impl Into<String>,
    ) -> Result<Self, String> {
        let environment = environment.into();
        validate_activation_environment(&environment)?;
        validate_activation_generation(generation, "generation")?;
        let assembly_identity = assembly_identity.into();
        validate_runtime_assembly_identity(&assembly_identity)?;
        let config_snapshot_id = config_snapshot_id.into();
        validate_runtime_config_snapshot_id(&config_snapshot_id)?;
        Ok(Self {
            environment,
            generation,
            assembly: RuntimeAssemblyRef {
                assembly_identity: AssemblyIdentity::new(assembly_identity),
            },
            config_snapshot: RuntimeConfigSnapshotRef {
                snapshot_id: RuntimeConfigSnapshotId::parse(config_snapshot_id)
                    .map_err(|error| error.to_string())?,
            },
        })
    }

    pub fn to_activation_header(&self) -> RouterBootstrapActivationFrameHeader {
        RouterBootstrapActivationFrameHeader {
            environment: self.environment.clone(),
            generation: self.generation,
            assembly: self.assembly.clone(),
            config_snapshot: self.config_snapshot.clone(),
        }
    }
}

/// Captured router config + activation tuple from which a stateless provider
/// constructs the one-shot `router.bootstrap` frame (plan §5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterBootstrapSource {
    pub artifacts_path: String,
    pub service_db: RouterBootstrapServiceDbFrameHeader,
    pub http: RouterBootstrapHttpFrameHeader,
    pub activation: CapturedBootstrapEpoch,
}

impl RouterBootstrapSource {
    /// Validates the full header through the canonical strict decoder and
    /// returns the typed frame header (payload is always empty).
    pub fn to_frame_header(&self) -> Result<RouterBootstrapFrameHeader, BinaryFrameError> {
        let header = RouterBootstrapFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: ROUTER_BOOTSTRAP_FRAME_TYPE.to_string(),
            artifacts_path: self.artifacts_path.clone(),
            service_db: self.service_db.clone(),
            http: self.http.clone(),
            activation: self.activation.to_activation_header(),
        };
        let value = serde_json::to_value(&header).map_err(|error| {
            TransportError::decode(format!(
                "invalid router.bootstrap frame header: serialization failed: {error}"
            ))
        })?;
        decode_router_bootstrap_frame_header(value)?;
        Ok(header)
    }
}

/// Router→Runtime bootstrap provider port (C-model-bootstrap-wire §5/§6).
///
/// The frozen contract signature references `RoutingEpoch`, which is a
/// W-bootstrap production type not present in this batch; transport owns the
/// wire-facing captured view (`RouterBootstrapSource`) until W-bootstrap maps
/// `RoutingEpoch` onto it.
pub trait RuntimeBootstrapProvider: Send + Sync {
    fn bootstrap_frame(
        &self,
        source: &RouterBootstrapSource,
    ) -> Result<RouterBootstrapFrameHeader, BinaryFrameError>;
}

/// Default stateless provider: validates the captured source and produces the
/// canonical `router.bootstrap` header (plan §5.5).
#[derive(Debug, Clone, Copy, Default)]
pub struct StatelessRuntimeBootstrapProvider;

impl RuntimeBootstrapProvider for StatelessRuntimeBootstrapProvider {
    fn bootstrap_frame(
        &self,
        source: &RouterBootstrapSource,
    ) -> Result<RouterBootstrapFrameHeader, BinaryFrameError> {
        source.to_frame_header()
    }
}

pub fn encode_router_bootstrap_frame(
    header: &RouterBootstrapFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_router_bootstrap_frame(header, &[])?;
    encode_binary_frame(header, &[])
}

pub fn decode_router_bootstrap_frame(
    frame: &[u8],
) -> Result<RouterBootstrapFrameHeader, BinaryFrameError> {
    let (header, payload): (RouterBootstrapFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(frame)?;
    validate_router_bootstrap_frame(&header, &payload)?;
    Ok(header)
}

fn validate_router_bootstrap_frame(
    header: &RouterBootstrapFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    let value = serde_json::to_value(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid router.bootstrap frame header: serialization failed: {error}"
        ))
    })?;
    decode_router_bootstrap_frame_header(value)?;
    if !payload.is_empty() {
        return Err(TransportError::decode(
            "router.bootstrap frame payload must be empty",
        ));
    }
    Ok(())
}

pub fn encode_runtime_capabilities_frame(
    header: &RuntimeCapabilitiesFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_empty_session_frame(
        &header.schema_version,
        &header.envelope_type,
        RUNTIME_CAPABILITIES_FRAME_TYPE,
        &[],
    )?;
    encode_binary_frame(header, &[])
}

pub fn decode_runtime_capabilities_frame(
    frame: &[u8],
) -> Result<RuntimeCapabilitiesFrameHeader, BinaryFrameError> {
    let (header, payload): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(frame)?;
    validate_empty_session_frame(
        &header.schema_version,
        &header.envelope_type,
        RUNTIME_CAPABILITIES_FRAME_TYPE,
        &payload,
    )?;
    Ok(header)
}

pub fn encode_runtime_registered_frame(
    header: &RuntimeRegisteredFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_empty_session_frame(
        &header.schema_version,
        &header.envelope_type,
        RUNTIME_REGISTERED_FRAME_TYPE,
        &[],
    )?;
    encode_binary_frame(header, &[])
}

pub fn decode_runtime_registered_frame(
    frame: &[u8],
) -> Result<RuntimeRegisteredFrameHeader, BinaryFrameError> {
    let (header, payload): (RuntimeRegisteredFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(frame)?;
    validate_empty_session_frame(
        &header.schema_version,
        &header.envelope_type,
        RUNTIME_REGISTERED_FRAME_TYPE,
        &payload,
    )?;
    Ok(header)
}

pub fn encode_runtime_health_frame(
    header: &RuntimeHealthFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_empty_session_frame(
        &header.schema_version,
        &header.envelope_type,
        RUNTIME_HEALTH_FRAME_TYPE,
        &[],
    )?;
    encode_binary_frame(header, &[])
}

pub fn decode_runtime_health_frame(
    frame: &[u8],
) -> Result<RuntimeHealthFrameHeader, BinaryFrameError> {
    let (header, payload): (RuntimeHealthFrameHeader, Vec<u8>) = decode_typed_binary_frame(frame)?;
    validate_empty_session_frame(
        &header.schema_version,
        &header.envelope_type,
        RUNTIME_HEALTH_FRAME_TYPE,
        &payload,
    )?;
    Ok(header)
}

fn validate_empty_session_frame(
    schema_version: &str,
    envelope_type: &str,
    expected_type: &str,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    if schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(TransportError::decode(format!(
            "{expected_type} frame schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        )));
    }
    if envelope_type != expected_type {
        return Err(TransportError::decode(format!(
            "{expected_type} frame type must be {expected_type}"
        )));
    }
    if !payload.is_empty() {
        return Err(TransportError::decode(format!(
            "{expected_type} frame payload must be empty"
        )));
    }
    Ok(())
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

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
