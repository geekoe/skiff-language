use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use skiff_artifact_model::{LiteralIr, PackageSchemaTypeId};

use crate::{
    addr::TypeAddr,
    error::{RuntimeErrorPayload, WirePayload},
    platform_error_projection::{
        decode_platform_error_projection_payload, encode_platform_error_projection_payload,
        EncodedPlatformErrorProjectionPayload, PlatformErrorProjectionCodecError,
        PlatformErrorProjectionDecodeOutcome, PlatformErrorProjectionPayload,
        ValidatedKnownPlatformErrorProjection,
    },
};

/// Canonical identity of one fully-instantiated type argument. The value is a
/// typed identity input, never a display label or runtime shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstantiatedTypeArgumentIdentity(String);

impl InstantiatedTypeArgumentIdentity {
    pub fn new(identity: impl Into<String>) -> Result<Self, String> {
        non_empty("instantiated type argument identity", identity.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalExecutionTypeIdentity {
    pub addr: TypeAddr,
    pub type_arguments: Vec<InstantiatedTypeArgumentIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageSchemaTypeIdentity {
    pub package_id: String,
    pub stable_schema_key: String,
    pub package_schema_type_id: PackageSchemaTypeId,
}

impl PackageSchemaTypeIdentity {
    pub fn new(
        package_id: impl Into<String>,
        stable_schema_key: impl Into<String>,
        package_schema_type_id: PackageSchemaTypeId,
    ) -> Result<Self, String> {
        let package_id = non_empty("packageId", package_id.into())?;
        let stable_schema_key = non_empty("stableSchemaKey", stable_schema_key.into())?;
        if package_schema_type_id.as_str().trim().is_empty() {
            return Err("packageSchemaTypeId must not be empty".to_string());
        }
        Ok(Self {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        })
    }
}

/// The finite platform-error registry. `std.resource.ResourceError` is
/// intentionally absent: it is a normal Package-owned public typed error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlatformBuiltinErrorIdentity {
    #[serde(rename = "TimeoutError")]
    Timeout,
    #[serde(rename = "config.DecodeError")]
    ConfigDecode,
    #[serde(rename = "std.bytes.DecodeError")]
    BytesDecode,
    #[serde(rename = "std.number.DecodeError")]
    NumberDecode,
    #[serde(rename = "std.json.DecodeError")]
    JsonDecode,
    #[serde(rename = "std.db.ConflictError")]
    DbConflict,
    #[serde(rename = "std.db.ConstraintError")]
    DbConstraint,
    #[serde(rename = "std.db.DecodeError")]
    DbDecode,
    #[serde(rename = "std.file.FileError")]
    File,
    #[serde(rename = "std.time.DecodeError")]
    TimeDecode,
    #[serde(rename = "std.service.ProviderUnavailableError")]
    ServiceProviderUnavailable,
    #[serde(rename = "std.service.ProtocolError")]
    ServiceProtocol,
    #[serde(rename = "std.http.HttpError")]
    Http,
}

impl PlatformBuiltinErrorIdentity {
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Some(match symbol {
            "TimeoutError" => Self::Timeout,
            "config.DecodeError" => Self::ConfigDecode,
            "std.bytes.DecodeError" => Self::BytesDecode,
            "std.number.DecodeError" => Self::NumberDecode,
            "std.json.DecodeError" => Self::JsonDecode,
            "std.db.ConflictError" => Self::DbConflict,
            "std.db.ConstraintError" => Self::DbConstraint,
            "std.db.DecodeError" => Self::DbDecode,
            "std.file.FileError" => Self::File,
            "std.time.DecodeError" => Self::TimeDecode,
            "std.service.ProviderUnavailableError" => Self::ServiceProviderUnavailable,
            "std.service.ProtocolError" => Self::ServiceProtocol,
            "std.http.HttpError" => Self::Http,
            _ => return None,
        })
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Timeout => "TimeoutError",
            Self::ConfigDecode => "config.DecodeError",
            Self::BytesDecode => "std.bytes.DecodeError",
            Self::NumberDecode => "std.number.DecodeError",
            Self::JsonDecode => "std.json.DecodeError",
            Self::DbConflict => "std.db.ConflictError",
            Self::DbConstraint => "std.db.ConstraintError",
            Self::DbDecode => "std.db.DecodeError",
            Self::File => "std.file.FileError",
            Self::TimeDecode => "std.time.DecodeError",
            Self::ServiceProviderUnavailable => "std.service.ProviderUnavailableError",
            Self::ServiceProtocol => "std.service.ProtocolError",
            Self::Http => "std.http.HttpError",
        }
    }

    pub fn catch_identity(self) -> CatchIdentity {
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(self))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NominalTypeIdentity {
    LocalExecution(LocalExecutionTypeIdentity),
    PackageSchema(PackageSchemaTypeIdentity),
    PlatformBuiltin(PlatformBuiltinErrorIdentity),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NamedUnionOwnerIdentity {
    LocalExecution(LocalExecutionTypeIdentity),
    PackageSchema(PackageSchemaTypeIdentity),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NamedUnionBranchIdentity {
    ConcreteNominal {
        identity: NominalTypeIdentity,
    },
    SyntheticDiscriminator {
        discriminator_field: String,
        discriminator_value: String,
    },
    Literal {
        value: LiteralIdentity,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LiteralIdentity {
    Null,
    Bool(bool),
    Number(String),
    String(String),
}

impl From<&LiteralIr> for LiteralIdentity {
    fn from(value: &LiteralIr) -> Self {
        match value {
            LiteralIr::Null => Self::Null,
            LiteralIr::Bool { value } => Self::Bool(*value),
            LiteralIr::Number { value } => Self::Number(value.to_string()),
            LiteralIr::String { value } => Self::String(value.clone()),
        }
    }
}

/// Exact identity used by local catch matching.
///
/// Anonymous unions use their selected branch's identity directly. Named
/// unions add the enclosing nominal owner so equal branch inputs in different
/// unions remain distinct.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CatchIdentity {
    Nominal(NominalTypeIdentity),
    NamedUnionBranch {
        union: NamedUnionOwnerIdentity,
        branch: NamedUnionBranchIdentity,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketRequestErrorKind {
    ConnectionUnavailable,
    TransportUnavailable,
    ProtocolError,
    ResourceLimit,
    Remote,
}

impl WebSocketRequestErrorKind {
    pub const ALL: [Self; 5] = [
        Self::ConnectionUnavailable,
        Self::TransportUnavailable,
        Self::ProtocolError,
        Self::ResourceLimit,
        Self::Remote,
    ];

    pub const fn discriminator(self) -> &'static str {
        match self {
            Self::ConnectionUnavailable => "connectionUnavailable",
            Self::TransportUnavailable => "transportUnavailable",
            Self::ProtocolError => "protocolError",
            Self::ResourceLimit => "resourceLimit",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebSocketRequestError {
    owner: NamedUnionOwnerIdentity,
    kind: WebSocketRequestErrorKind,
    message: String,
    code: Option<i64>,
    data: Option<Value>,
}

impl WebSocketRequestError {
    pub fn new(
        owner: NamedUnionOwnerIdentity,
        kind: WebSocketRequestErrorKind,
        message: impl Into<String>,
        code: Option<i64>,
        data: Option<Value>,
    ) -> Result<Self, String> {
        let message = non_empty("WebSocket request error message", message.into())?;
        if (kind == WebSocketRequestErrorKind::Remote) != code.is_some() {
            return Err("only remote WebSocket request errors carry a code".to_string());
        }
        if kind != WebSocketRequestErrorKind::Remote && data.is_some() {
            return Err("only remote WebSocket request errors carry data".to_string());
        }
        Ok(Self {
            owner,
            kind,
            message,
            code,
            data,
        })
    }

    pub fn kind(&self) -> WebSocketRequestErrorKind {
        self.kind
    }

    pub fn exact_catch_identity(&self) -> CatchIdentity {
        CatchIdentity::NamedUnionBranch {
            union: self.owner.clone(),
            branch: NamedUnionBranchIdentity::SyntheticDiscriminator {
                discriminator_field: "kind".to_string(),
                discriminator_value: self.kind.discriminator().to_string(),
            },
        }
    }

    pub fn local_payload(&self) -> Value {
        match self.kind {
            WebSocketRequestErrorKind::Remote => json!({
                "kind": self.kind.discriminator(),
                "code": self.code.expect("remote error code validated"),
                "message": self.message,
                "data": self.data,
            }),
            _ => json!({
                "kind": self.kind.discriminator(),
                "message": self.message,
            }),
        }
    }
}

impl std::fmt::Display for WebSocketRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WebSocketRequestError {}

impl WirePayload for WebSocketRequestError {
    fn payload(&self) -> RuntimeErrorPayload {
        RuntimeErrorPayload {
            code: "std.websocket.WebSocketRequestError".to_string(),
            message: self.message.clone(),
            status: None,
            details: Some(self.local_payload()),
        }
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        Some((self.exact_catch_identity(), self.local_payload()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub const MAX_PLATFORM_ERROR_PROJECTION_KEY_BYTES: usize = 128;
pub const MAX_PLATFORM_ERROR_ENCODED_PAYLOAD_BYTES: usize = 64 * 1024;

const ENTRY_FINGERPRINT_PREFIX: &str = "sha256:";
const ENTRY_FINGERPRINT_HEX_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorTextField {
    PackageId,
    StableSchemaKey,
    PackageSchemaTypeId,
    InternalMessage,
    TraceId,
    ErrorId,
}

impl std::fmt::Display for ServiceErrorTextField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PackageId => "packageId",
            Self::StableSchemaKey => "stableSchemaKey",
            Self::PackageSchemaTypeId => "packageSchemaTypeId",
            Self::InternalMessage => "payload.message",
            Self::TraceId => "traceId",
            Self::ErrorId => "errorId",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorTextViolation {
    Empty,
    SurroundingWhitespace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceErrorOuterValidationError {
    InvalidWireShape,
    InvalidText {
        field: ServiceErrorTextField,
        violation: ServiceErrorTextViolation,
    },
    EmptyPublicTypedPayload,
    InvalidProjectionKeyLength {
        length: usize,
    },
    InvalidProjectionKeyCharacter {
        byte_index: usize,
    },
    VersionedProjectionKeySuffix,
    InvalidEntryFingerprint,
    InvalidPlatformPayloadLength {
        length: usize,
    },
}

impl std::fmt::Display for ServiceErrorOuterValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWireShape => formatter.write_str("invalid service error envelope shape"),
            Self::InvalidText { field, violation } => match violation {
                ServiceErrorTextViolation::Empty => write!(formatter, "{field} must not be empty"),
                ServiceErrorTextViolation::SurroundingWhitespace => {
                    write!(formatter, "{field} must not contain surrounding whitespace")
                }
            },
            Self::EmptyPublicTypedPayload => {
                formatter.write_str("public typed encodedPayload must not be empty")
            }
            Self::InvalidProjectionKeyLength { length } => write!(
                formatter,
                "projectionKey length {length} is outside the allowed range"
            ),
            Self::InvalidProjectionKeyCharacter { byte_index } => write!(
                formatter,
                "projectionKey contains an invalid byte at index {byte_index}"
            ),
            Self::VersionedProjectionKeySuffix => {
                formatter.write_str("projectionKey must not have a numeric version suffix")
            }
            Self::InvalidEntryFingerprint => {
                formatter.write_str("entryFingerprint has an invalid format")
            }
            Self::InvalidPlatformPayloadLength { length } => write!(
                formatter,
                "platform encodedPayload length {length} is outside the allowed range"
            ),
        }
    }
}

impl std::error::Error for ServiceErrorOuterValidationError {}

#[derive(Debug)]
pub enum ServiceErrorDecodeError {
    MalformedJson { source: serde_json::Error },
    InvalidOuter(ServiceErrorOuterValidationError),
    CanonicalOuterEncoding { source: serde_json::Error },
    NonCanonicalOuterBytes,
    ExactKnownPlatformPayload(PlatformErrorProjectionCodecError),
}

impl std::fmt::Display for ServiceErrorDecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedJson { .. } => {
                formatter.write_str("service error envelope is not valid JSON")
            }
            Self::InvalidOuter(error) => {
                write!(formatter, "invalid service error envelope: {error}")
            }
            Self::CanonicalOuterEncoding { .. } => {
                formatter.write_str("failed to produce canonical service error envelope bytes")
            }
            Self::NonCanonicalOuterBytes => {
                formatter.write_str("service error envelope bytes are not canonical JSON")
            }
            Self::ExactKnownPlatformPayload(_) => formatter
                .write_str("exact-known platform error payload failed generated validation"),
        }
    }
}

impl std::error::Error for ServiceErrorDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MalformedJson { source } | Self::CanonicalOuterEncoding { source } => {
                Some(source)
            }
            Self::InvalidOuter(error) => Some(error),
            Self::ExactKnownPlatformPayload(error) => Some(error),
            Self::NonCanonicalOuterBytes => None,
        }
    }
}

#[derive(Debug)]
pub enum ServiceErrorEncodeError {
    PlatformProjectionCodec(PlatformErrorProjectionCodecError),
    InvalidOuter(ServiceErrorOuterValidationError),
    CanonicalOuterEncoding { source: serde_json::Error },
}

impl std::fmt::Display for ServiceErrorEncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PlatformProjectionCodec(_) => {
                formatter.write_str("generated platform error payload encoding failed")
            }
            Self::InvalidOuter(error) => {
                write!(formatter, "invalid service error envelope: {error}")
            }
            Self::CanonicalOuterEncoding { .. } => {
                formatter.write_str("failed to produce canonical service error envelope bytes")
            }
        }
    }
}

impl std::error::Error for ServiceErrorEncodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PlatformProjectionCodec(error) => Some(error),
            Self::InvalidOuter(error) => Some(error),
            Self::CanonicalOuterEncoding { source } => Some(source),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServiceErrorEnvelope {
    PublicTypedError {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
        encoded_payload: Vec<u8>,
        trace_id: String,
        error_id: String,
    },
    InternalError {
        payload: InternalErrorPayload,
    },
    PlatformError {
        projection_key: String,
        entry_fingerprint: String,
        encoded_payload: Vec<u8>,
        trace_id: String,
        error_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalErrorPayload {
    pub message: String,
    pub trace_id: String,
    pub error_id: String,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[allow(
    clippy::enum_variant_names,
    reason = "the Error suffix preserves the canonical wire kind values and mirrors the validated public envelope variants"
)]
enum ServiceErrorEnvelopeWire {
    PublicTypedError {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
        encoded_payload: Vec<u8>,
        trace_id: String,
        error_id: String,
    },
    InternalError {
        payload: InternalErrorPayload,
    },
    PlatformError {
        projection_key: String,
        entry_fingerprint: String,
        encoded_payload: Vec<u8>,
        trace_id: String,
        error_id: String,
    },
}

impl ServiceErrorEnvelopeWire {
    fn into_envelope(self) -> ServiceErrorEnvelope {
        match self {
            Self::PublicTypedError {
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                trace_id,
                error_id,
            } => ServiceErrorEnvelope::PublicTypedError {
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                trace_id,
                error_id,
            },
            Self::InternalError { payload } => ServiceErrorEnvelope::InternalError { payload },
            Self::PlatformError {
                projection_key,
                entry_fingerprint,
                encoded_payload,
                trace_id,
                error_id,
            } => ServiceErrorEnvelope::PlatformError {
                projection_key,
                entry_fingerprint,
                encoded_payload,
                trace_id,
                error_id,
            },
        }
    }
}

impl<'de> Deserialize<'de> for ServiceErrorEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let envelope = ServiceErrorEnvelopeWire::deserialize(deserializer)?.into_envelope();
        envelope.validate().map_err(serde::de::Error::custom)?;
        Ok(envelope)
    }
}

impl ServiceErrorEnvelope {
    fn validate(&self) -> Result<(), ServiceErrorOuterValidationError> {
        match self {
            Self::PublicTypedError {
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                trace_id,
                error_id,
            } => {
                validate_text(ServiceErrorTextField::PackageId, package_id)?;
                validate_text(ServiceErrorTextField::StableSchemaKey, stable_schema_key)?;
                validate_text(
                    ServiceErrorTextField::PackageSchemaTypeId,
                    package_schema_type_id.as_str(),
                )?;
                if encoded_payload.is_empty() {
                    return Err(ServiceErrorOuterValidationError::EmptyPublicTypedPayload);
                }
                validate_service_error_correlation(trace_id, error_id)
            }
            Self::InternalError { payload } => {
                validate_text(ServiceErrorTextField::InternalMessage, &payload.message)?;
                validate_service_error_correlation(&payload.trace_id, &payload.error_id)
            }
            Self::PlatformError {
                projection_key,
                entry_fingerprint,
                encoded_payload,
                trace_id,
                error_id,
            } => {
                validate_projection_key(projection_key)?;
                validate_entry_fingerprint(entry_fingerprint)?;
                validate_platform_payload(encoded_payload)?;
                validate_service_error_correlation(trace_id, error_id)
            }
        }
    }

    pub fn trace_id(&self) -> &str {
        match self {
            Self::PublicTypedError { trace_id, .. } | Self::PlatformError { trace_id, .. } => {
                trace_id
            }
            Self::InternalError { payload } => &payload.trace_id,
        }
    }

    pub fn error_id(&self) -> &str {
        match self {
            Self::PublicTypedError { error_id, .. } | Self::PlatformError { error_id, .. } => {
                error_id
            }
            Self::InternalError { payload } => &payload.error_id,
        }
    }
}

/// A canonical fixed service error. The generated typed cache is derived only
/// after exact-pair validation and does not participate in carrier identity.
#[derive(Debug, Clone)]
pub struct OpaqueServiceError {
    envelope: ServiceErrorEnvelope,
    encoded_bytes: Vec<u8>,
    validated_known_platform_projection: Option<ValidatedKnownPlatformErrorProjection>,
}

impl PartialEq for OpaqueServiceError {
    fn eq(&self, other: &Self) -> bool {
        self.envelope == other.envelope && self.encoded_bytes == other.encoded_bytes
    }
}

impl Eq for OpaqueServiceError {}

impl OpaqueServiceError {
    pub fn decode(encoded_bytes: Vec<u8>) -> Result<Self, ServiceErrorDecodeError> {
        let wire = serde_json::from_slice::<ServiceErrorEnvelopeWire>(&encoded_bytes).map_err(
            |source| match source.classify() {
                serde_json::error::Category::Data => ServiceErrorDecodeError::InvalidOuter(
                    ServiceErrorOuterValidationError::InvalidWireShape,
                ),
                serde_json::error::Category::Io
                | serde_json::error::Category::Syntax
                | serde_json::error::Category::Eof => {
                    ServiceErrorDecodeError::MalformedJson { source }
                }
            },
        )?;
        let envelope = wire.into_envelope();
        envelope
            .validate()
            .map_err(ServiceErrorDecodeError::InvalidOuter)?;

        let canonical_bytes = skiff_canonical_json::canonical_json_bytes(&envelope)
            .map_err(|source| ServiceErrorDecodeError::CanonicalOuterEncoding { source })?;
        if canonical_bytes != encoded_bytes {
            return Err(ServiceErrorDecodeError::NonCanonicalOuterBytes);
        }

        let validated_known_platform_projection = match &envelope {
            ServiceErrorEnvelope::PlatformError {
                projection_key,
                entry_fingerprint,
                encoded_payload,
                ..
            } => match decode_platform_error_projection_payload(
                projection_key,
                entry_fingerprint,
                encoded_payload,
            )
            .map_err(ServiceErrorDecodeError::ExactKnownPlatformPayload)?
            {
                PlatformErrorProjectionDecodeOutcome::Known(payload) => {
                    Some(ValidatedKnownPlatformErrorProjection::new(payload))
                }
                PlatformErrorProjectionDecodeOutcome::UnknownValid => None,
            },
            ServiceErrorEnvelope::PublicTypedError { .. }
            | ServiceErrorEnvelope::InternalError { .. } => None,
        };

        Ok(Self {
            envelope,
            encoded_bytes,
            validated_known_platform_projection,
        })
    }

    pub fn platform_error(
        payload: &PlatformErrorProjectionPayload,
        trace_id: &str,
        error_id: &str,
    ) -> Result<Self, ServiceErrorEncodeError> {
        let encoded = encode_platform_error_projection_payload(payload)
            .map_err(ServiceErrorEncodeError::PlatformProjectionCodec)?;
        let validated = ValidatedKnownPlatformErrorProjection::new(payload.clone());
        Self::from_encoded_platform_error_projection(encoded, validated, trace_id, error_id)
    }

    pub fn internal_error(
        message: &str,
        trace_id: &str,
        error_id: &str,
    ) -> Result<Self, ServiceErrorEncodeError> {
        Self::encode_local(
            ServiceErrorEnvelope::InternalError {
                payload: InternalErrorPayload {
                    message: message.to_owned(),
                    trace_id: trace_id.to_owned(),
                    error_id: error_id.to_owned(),
                },
            },
            None,
        )
    }

    pub fn public_typed_error(
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: PackageSchemaTypeId,
        encoded_payload: &[u8],
        trace_id: &str,
        error_id: &str,
    ) -> Result<Self, ServiceErrorEncodeError> {
        Self::encode_local(
            ServiceErrorEnvelope::PublicTypedError {
                package_id: package_id.to_owned(),
                stable_schema_key: stable_schema_key.to_owned(),
                package_schema_type_id,
                encoded_payload: encoded_payload.to_vec(),
                trace_id: trace_id.to_owned(),
                error_id: error_id.to_owned(),
            },
            None,
        )
    }

    fn from_encoded_platform_error_projection(
        encoded: EncodedPlatformErrorProjectionPayload,
        validated: ValidatedKnownPlatformErrorProjection,
        trace_id: &str,
        error_id: &str,
    ) -> Result<Self, ServiceErrorEncodeError> {
        debug_assert_eq!(encoded.projection_key(), validated.projection_key());
        let projection_key = encoded.projection_key().as_str().to_owned();
        let entry_fingerprint = encoded.entry_fingerprint().to_owned();
        let encoded_payload = encoded.into_canonical_payload();
        Self::encode_local(
            ServiceErrorEnvelope::PlatformError {
                projection_key,
                entry_fingerprint,
                encoded_payload,
                trace_id: trace_id.to_owned(),
                error_id: error_id.to_owned(),
            },
            Some(validated),
        )
    }

    fn encode_local(
        envelope: ServiceErrorEnvelope,
        validated_known_platform_projection: Option<ValidatedKnownPlatformErrorProjection>,
    ) -> Result<Self, ServiceErrorEncodeError> {
        envelope
            .validate()
            .map_err(ServiceErrorEncodeError::InvalidOuter)?;
        let encoded_bytes = skiff_canonical_json::canonical_json_bytes(&envelope)
            .map_err(|source| ServiceErrorEncodeError::CanonicalOuterEncoding { source })?;
        Ok(Self {
            envelope,
            encoded_bytes,
            validated_known_platform_projection,
        })
    }

    pub fn envelope(&self) -> &ServiceErrorEnvelope {
        &self.envelope
    }

    pub fn known_platform_projection(&self) -> Option<&ValidatedKnownPlatformErrorProjection> {
        self.validated_known_platform_projection.as_ref()
    }

    pub fn encoded_bytes(&self) -> &[u8] {
        &self.encoded_bytes
    }

    pub fn into_encoded_bytes(self) -> Vec<u8> {
        self.encoded_bytes
    }
}

fn validate_text(
    field: ServiceErrorTextField,
    value: &str,
) -> Result<(), ServiceErrorOuterValidationError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ServiceErrorOuterValidationError::InvalidText {
            field,
            violation: ServiceErrorTextViolation::Empty,
        });
    }
    if trimmed != value {
        return Err(ServiceErrorOuterValidationError::InvalidText {
            field,
            violation: ServiceErrorTextViolation::SurroundingWhitespace,
        });
    }
    Ok(())
}

fn validate_projection_key(value: &str) -> Result<(), ServiceErrorOuterValidationError> {
    let length = value.len();
    if !(1..=MAX_PLATFORM_ERROR_PROJECTION_KEY_BYTES).contains(&length) {
        return Err(ServiceErrorOuterValidationError::InvalidProjectionKeyLength { length });
    }
    if let Some(byte_index) = value.bytes().position(
        |byte| !matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'),
    ) {
        return Err(ServiceErrorOuterValidationError::InvalidProjectionKeyCharacter { byte_index });
    }
    if value.rsplit_once(".v").is_some_and(|(_, suffix)| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return Err(ServiceErrorOuterValidationError::VersionedProjectionKeySuffix);
    }
    Ok(())
}

fn validate_entry_fingerprint(value: &str) -> Result<(), ServiceErrorOuterValidationError> {
    let valid = value
        .strip_prefix(ENTRY_FINGERPRINT_PREFIX)
        .is_some_and(|hex| {
            hex.len() == ENTRY_FINGERPRINT_HEX_BYTES
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        });
    if !valid {
        return Err(ServiceErrorOuterValidationError::InvalidEntryFingerprint);
    }
    Ok(())
}

fn validate_platform_payload(value: &[u8]) -> Result<(), ServiceErrorOuterValidationError> {
    let length = value.len();
    if !(1..=MAX_PLATFORM_ERROR_ENCODED_PAYLOAD_BYTES).contains(&length) {
        return Err(ServiceErrorOuterValidationError::InvalidPlatformPayloadLength { length });
    }
    Ok(())
}

fn validate_service_error_correlation(
    trace_id: &str,
    error_id: &str,
) -> Result<(), ServiceErrorOuterValidationError> {
    validate_text(ServiceErrorTextField::TraceId, trace_id)?;
    validate_text(ServiceErrorTextField::ErrorId, error_id)
}

fn non_empty(label: &str, value: String) -> Result<String, String> {
    non_empty_ref(label, &value)?;
    Ok(value)
}

fn non_empty_ref(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(format!(
            "{label} must not be empty or contain surrounding whitespace"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
