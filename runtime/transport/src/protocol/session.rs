use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_model::validate_activation_profile;

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
    /// Absolute artifact root this runtime lazily loads deployment build ids from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_root: Option<String>,
    /// True when the runtime can lazy-load unregistered deployment build ids on demand.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lazy_load: bool,
    /// All currently loaded deployment build ids (empty = nothing loaded yet).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loaded_build_ids: Vec<String>,
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
    pub profile: String,
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
    validate_activation_profile(&header.activation.profile).map_err(|error| {
        TransportError::decode(format!(
            "invalid router.bootstrap frame header: activation.profile {error}"
        ))
    })?;
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

/// Captured router config + profile from which a stateless provider
/// constructs the one-shot `router.bootstrap` frame (plan §5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterBootstrapSource {
    pub artifacts_path: String,
    pub service_db: RouterBootstrapServiceDbFrameHeader,
    pub http: RouterBootstrapHttpFrameHeader,
    pub profile: String,
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
            activation: RouterBootstrapActivationFrameHeader {
                profile: self.profile.clone(),
            },
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

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
