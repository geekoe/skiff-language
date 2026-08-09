use base64::Engine as _;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};

use crate::{
    protocol::{decode_binary_frame, encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    BinaryFrameError, TransportError,
};

pub const ACTOR_ARGUMENTS_ENCODING_V1: &str = "skiff-actor-arguments-v1";
pub const ACTOR_RETURN_ENCODING_V1: &str = "skiff-actor-return-v1";
const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorLogicalRefFrameHeader {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorDeclarationOwnerFrameHeader {
    pub unit: ActorOwnerUnitFrameHeader,
    pub file: ActorOwnerFileFrameHeader,
    pub actor_symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ActorOwnerUnitFrameHeader {
    Service,
    Package(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ActorOwnerFileFrameHeader {
    LoadedFileIndex(u64),
    FileIrIdentity(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorMethodInvokeFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub invocation_id: String,
    pub actor_ref: ActorLogicalRefFrameHeader,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub method_identity: ActorMethodIdentity,
    pub arguments_encoding_version: String,
    pub deadline: ActorMethodDeadlineFrameHeader,
    pub cancellation_correlation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_case_capability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_case_parent_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActorMethodInvokeFrameHeader {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    invocation_id: String,
    actor_ref: ActorLogicalRefFrameHeader,
    declaration_owner: ActorDeclarationOwnerFrameHeader,
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: ActorImplementationIdentity,
    method_identity: ActorMethodIdentity,
    arguments_encoding_version: String,
    deadline: ActorMethodDeadlineFrameHeader,
    cancellation_correlation: String,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    test_case_capability: Option<String>,
    #[serde(default)]
    test_case_parent_request_id: Option<String>,
}

impl<'de> Deserialize<'de> for ActorMethodInvokeFrameHeader {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawActorMethodInvokeFrameHeader::deserialize(deserializer)?;
        validate_common(
            &raw.schema_version,
            &raw.envelope_type,
            "actor.method.invoke",
        )
        .map_err(de::Error::custom)?;
        validate_token(&raw.invocation_id, "invocationId").map_err(de::Error::custom)?;
        validate_actor_ref(&raw.actor_ref).map_err(de::Error::custom)?;
        validate_owner(&raw.declaration_owner).map_err(de::Error::custom)?;
        validate_identity(raw.actor_abi_identity.as_str(), "skiff-actor-abi-v1:sha256")
            .map_err(de::Error::custom)?;
        validate_identity(
            raw.actor_implementation_identity.as_str(),
            "skiff-actor-implementation-v1:sha256",
        )
        .map_err(de::Error::custom)?;
        validate_identity(raw.method_identity.as_str(), "skiff-actor-method-v1:sha256")
            .map_err(de::Error::custom)?;
        if raw.arguments_encoding_version != ACTOR_ARGUMENTS_ENCODING_V1 {
            return Err(de::Error::custom("unsupported argumentsEncodingVersion"));
        }
        validate_deadline(&raw.deadline).map_err(de::Error::custom)?;
        validate_token(&raw.cancellation_correlation, "cancellationCorrelation")
            .map_err(de::Error::custom)?;
        if raw
            .trace_id
            .as_deref()
            .is_some_and(|trace_id| trace_id.trim().is_empty())
        {
            return Err(de::Error::custom("traceId must be non-empty when present"));
        }
        if let Some(test_case_capability) = raw.test_case_capability.as_deref() {
            validate_token(test_case_capability, "testCaseCapability")
                .map_err(de::Error::custom)?;
        }
        if let Some(parent_request_id) = raw.test_case_parent_request_id.as_deref() {
            validate_token(parent_request_id, "testCaseParentRequestId")
                .map_err(de::Error::custom)?;
        }
        if raw.test_case_capability.is_some() != raw.test_case_parent_request_id.is_some() {
            return Err(de::Error::custom(
                "testCaseCapability and testCaseParentRequestId must be present together",
            ));
        }
        Ok(Self {
            schema_version: raw.schema_version,
            envelope_type: raw.envelope_type,
            invocation_id: raw.invocation_id,
            actor_ref: raw.actor_ref,
            declaration_owner: raw.declaration_owner,
            actor_abi_identity: raw.actor_abi_identity,
            actor_implementation_identity: raw.actor_implementation_identity,
            method_identity: raw.method_identity,
            arguments_encoding_version: raw.arguments_encoding_version,
            deadline: raw.deadline,
            cancellation_correlation: raw.cancellation_correlation,
            trace_id: raw.trace_id,
            test_case_capability: raw.test_case_capability,
            test_case_parent_request_id: raw.test_case_parent_request_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorMethodDeadlineFrameHeader {
    pub timeout_ms: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorMethodReturnFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub invocation_id: String,
    pub return_encoding_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorMethodCancelFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub invocation_id: String,
    pub cancellation_correlation: String,
    pub reason: ActorMethodCancelReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorMethodCancelReason {
    Cancelled,
    DeadlineExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorMethodErrorFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub invocation_id: String,
    pub error: ActorMethodErrorFramePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "name",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ActorMethodErrorFramePayload {
    ActorUpgradingError {
        actor_ref: ActorLogicalRefFrameHeader,
        retry_after_ms: u64,
    },
    ActorVersionRejectedError {
        actor_ref: ActorLogicalRefFrameHeader,
        requested_implementation_identity: ActorImplementationIdentity,
        accepted_implementation_identity: ActorImplementationIdentity,
    },
    ActorIncarnationReplacedError {
        actor_ref: ActorLogicalRefFrameHeader,
        current_epoch: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
// This public frame enum intentionally owns its headers and payloads; boxing a
// variant would change its API and impose allocation on every caller.
#[allow(clippy::large_enum_variant)]
pub enum ActorMethodFrame {
    Invoke(ActorMethodInvokeFrameHeader, Vec<u8>),
    Return(ActorMethodReturnFrameHeader, Vec<u8>),
    Error(ActorMethodErrorFrameHeader),
    Cancel(ActorMethodCancelFrameHeader),
}

pub fn encode_actor_method_frame(frame: &ActorMethodFrame) -> Result<Vec<u8>, BinaryFrameError> {
    match frame {
        ActorMethodFrame::Invoke(header, payload) => {
            validate_invoke(header).map_err(TransportError::decode)?;
            encode_binary_frame(header, payload)
        }
        ActorMethodFrame::Return(header, payload) => {
            validate_return(header).map_err(TransportError::decode)?;
            encode_binary_frame(header, payload)
        }
        ActorMethodFrame::Error(header) => {
            validate_error_header(header).map_err(TransportError::decode)?;
            encode_binary_frame(header, &[])
        }
        ActorMethodFrame::Cancel(header) => {
            validate_cancel(header).map_err(TransportError::decode)?;
            encode_binary_frame(header, &[])
        }
    }
}

pub fn decode_actor_method_frame(bytes: &[u8]) -> Result<ActorMethodFrame, BinaryFrameError> {
    let frame = decode_binary_frame(bytes)?;
    let kind = frame
        .header
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| TransportError::decode("invalid actor method frame: type must be a string"))?
        .to_string();
    match kind.as_str() {
        "actor.method.invoke" => {
            let header: ActorMethodInvokeFrameHeader = typed(frame.header)?;
            Ok(ActorMethodFrame::Invoke(header, frame.payload_bytes))
        }
        "actor.method.return" => {
            let header: ActorMethodReturnFrameHeader = typed(frame.header)?;
            validate_return(&header).map_err(TransportError::decode)?;
            Ok(ActorMethodFrame::Return(header, frame.payload_bytes))
        }
        "actor.method.error" => {
            reject_payload(&frame.payload_bytes, &kind)?;
            let header: ActorMethodErrorFrameHeader = typed(frame.header)?;
            validate_error_header(&header).map_err(TransportError::decode)?;
            Ok(ActorMethodFrame::Error(header))
        }
        "actor.method.cancel" => {
            reject_payload(&frame.payload_bytes, &kind)?;
            let header: ActorMethodCancelFrameHeader = typed(frame.header)?;
            validate_cancel(&header).map_err(TransportError::decode)?;
            Ok(ActorMethodFrame::Cancel(header))
        }
        _ => Err(TransportError::decode(format!(
            "invalid actor method frame: unsupported type {kind}"
        ))),
    }
}

fn validate_invoke(header: &ActorMethodInvokeFrameHeader) -> Result<(), String> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        "actor.method.invoke",
    )?;
    validate_token(&header.invocation_id, "invocationId")?;
    validate_actor_ref(&header.actor_ref)?;
    validate_owner(&header.declaration_owner)?;
    validate_identity(
        header.actor_abi_identity.as_str(),
        "skiff-actor-abi-v1:sha256",
    )?;
    validate_identity(
        header.actor_implementation_identity.as_str(),
        "skiff-actor-implementation-v1:sha256",
    )?;
    validate_identity(
        header.method_identity.as_str(),
        "skiff-actor-method-v1:sha256",
    )?;
    if header.arguments_encoding_version != ACTOR_ARGUMENTS_ENCODING_V1 {
        return Err("unsupported argumentsEncodingVersion".into());
    }
    validate_deadline(&header.deadline)?;
    validate_token(&header.cancellation_correlation, "cancellationCorrelation")?;
    if header
        .trace_id
        .as_deref()
        .is_some_and(|trace_id| trace_id.trim().is_empty())
    {
        return Err("traceId must be non-empty when present".into());
    }
    if let Some(test_case_capability) = header.test_case_capability.as_deref() {
        validate_token(test_case_capability, "testCaseCapability")?;
    }
    if let Some(parent_request_id) = header.test_case_parent_request_id.as_deref() {
        validate_token(parent_request_id, "testCaseParentRequestId")?;
    }
    if header.test_case_capability.is_some() != header.test_case_parent_request_id.is_some() {
        return Err(
            "testCaseCapability and testCaseParentRequestId must be present together".into(),
        );
    }
    Ok(())
}

fn validate_return(header: &ActorMethodReturnFrameHeader) -> Result<(), String> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        "actor.method.return",
    )?;
    validate_token(&header.invocation_id, "invocationId")?;
    if header.return_encoding_version != ACTOR_RETURN_ENCODING_V1 {
        return Err("unsupported returnEncodingVersion".into());
    }
    Ok(())
}

fn validate_error_header(header: &ActorMethodErrorFrameHeader) -> Result<(), String> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        "actor.method.error",
    )?;
    validate_token(&header.invocation_id, "invocationId")?;
    validate_error(&header.error)
}

fn validate_cancel(header: &ActorMethodCancelFrameHeader) -> Result<(), String> {
    validate_common(
        &header.schema_version,
        &header.envelope_type,
        "actor.method.cancel",
    )?;
    validate_token(&header.invocation_id, "invocationId")?;
    validate_token(&header.cancellation_correlation, "cancellationCorrelation")
}

fn typed<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, BinaryFrameError> {
    serde_json::from_value(value)
        .map_err(|error| TransportError::decode(format!("invalid actor method frame: {error}")))
}

fn validate_common(schema: &str, actual: &str, expected: &str) -> Result<(), String> {
    if schema != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err("unsupported actor method frame schemaVersion".into());
    }
    if actual != expected {
        return Err(format!("actor method frame type must be {expected}"));
    }
    Ok(())
}

pub(crate) fn validate_token(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    {
        return Err(format!("{name} must be a non-empty canonical token"));
    }
    Ok(())
}

pub(crate) fn validate_identity(value: &str, prefix: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix(&format!("{prefix}:")) else {
        return Err(format!("identity must use {prefix}"));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("identity must contain a lowercase 64-byte hexadecimal digest".into());
    }
    Ok(())
}

pub(crate) fn validate_actor_ref(actor_ref: &ActorLogicalRefFrameHeader) -> Result<(), String> {
    for (value, name) in [
        (&actor_ref.service_id, "actorRef.serviceId"),
        (&actor_ref.actor_type_identity, "actorRef.actorTypeIdentity"),
        (
            &actor_ref.actor_id_type_identity,
            "actorRef.actorIdTypeIdentity",
        ),
        (
            &actor_ref.actor_id_encoding_version,
            "actorRef.actorIdEncodingVersion",
        ),
        (
            &actor_ref.canonical_actor_id_key_bytes_base64,
            "actorRef.canonicalActorIdKeyBytesBase64",
        ),
        (&actor_ref.actor_id_hash, "actorRef.actorIdHash"),
    ] {
        if value.trim().is_empty() {
            return Err(format!("{name} must be non-empty"));
        }
    }
    validate_safe_positive(actor_ref.epoch, "actorRef.epoch")?;
    validate_sha256(&actor_ref.actor_id_hash, "actorRef.actorIdHash")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&actor_ref.canonical_actor_id_key_bytes_base64)
        .map_err(|_| "actorRef.canonicalActorIdKeyBytesBase64 must be canonical base64")?;
    if base64::engine::general_purpose::STANDARD.encode(decoded)
        != actor_ref.canonical_actor_id_key_bytes_base64
    {
        return Err("actorRef.canonicalActorIdKeyBytesBase64 must be canonical base64".into());
    }
    Ok(())
}

fn validate_sha256(value: &str, name: &str) -> Result<(), String> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(format!("{name} must use sha256"));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{name} must contain a lowercase sha256 digest"));
    }
    Ok(())
}

pub(crate) fn validate_owner(owner: &ActorDeclarationOwnerFrameHeader) -> Result<(), String> {
    if owner.actor_symbol.trim().is_empty() {
        return Err("declarationOwner.actorSymbol must be non-empty".into());
    }
    if let ActorOwnerFileFrameHeader::FileIrIdentity(identity) = &owner.file {
        if identity.trim().is_empty() {
            return Err("declarationOwner file identity must be non-empty".into());
        }
    }
    if let ActorOwnerUnitFrameHeader::Package(value) = &owner.unit {
        validate_safe_integer(*value, "declarationOwner.unit.value")?;
    }
    if let ActorOwnerFileFrameHeader::LoadedFileIndex(value) = &owner.file {
        validate_safe_integer(*value, "declarationOwner.file.value")?;
    }
    Ok(())
}

fn validate_deadline(deadline: &ActorMethodDeadlineFrameHeader) -> Result<(), String> {
    if deadline.expires_at.trim().is_empty() {
        return Err("deadline timeoutMs must be positive and expiresAt must be non-empty".into());
    }
    validate_safe_positive(deadline.timeout_ms, "deadline.timeoutMs")?;
    Ok(())
}

fn validate_error(error: &ActorMethodErrorFramePayload) -> Result<(), String> {
    let actor_ref = match error {
        ActorMethodErrorFramePayload::ActorUpgradingError {
            actor_ref,
            retry_after_ms,
        } => {
            validate_safe_integer(*retry_after_ms, "error.retryAfterMs")?;
            actor_ref
        }
        ActorMethodErrorFramePayload::ActorVersionRejectedError {
            actor_ref,
            requested_implementation_identity,
            accepted_implementation_identity,
        } => {
            validate_identity(
                requested_implementation_identity.as_str(),
                "skiff-actor-implementation-v1:sha256",
            )?;
            validate_identity(
                accepted_implementation_identity.as_str(),
                "skiff-actor-implementation-v1:sha256",
            )?;
            actor_ref
        }
        ActorMethodErrorFramePayload::ActorIncarnationReplacedError {
            actor_ref,
            current_epoch,
        } => {
            validate_safe_positive(*current_epoch, "error.currentEpoch")?;
            if *current_epoch == actor_ref.epoch {
                return Err("currentEpoch must be positive and differ from requested epoch".into());
            }
            actor_ref
        }
    };
    validate_actor_ref(actor_ref)
}

fn validate_safe_integer(value: u64, name: &str) -> Result<(), String> {
    if value > JAVASCRIPT_MAX_SAFE_INTEGER {
        return Err(format!("{name} must be a JavaScript safe integer"));
    }
    Ok(())
}

fn validate_safe_positive(value: u64, name: &str) -> Result<(), String> {
    validate_safe_integer(value, name)?;
    if value == 0 {
        return Err(format!("{name} must be positive"));
    }
    Ok(())
}

fn reject_payload(payload: &[u8], kind: &str) -> Result<(), BinaryFrameError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(TransportError::decode(format!(
            "{kind} payload must be empty"
        )))
    }
}

#[cfg(test)]
mod tests;
