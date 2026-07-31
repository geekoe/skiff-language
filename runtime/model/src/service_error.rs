use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use skiff_artifact_model::{InstructionSourceSite, LiteralIr, PackageSchemaTypeId};

use crate::{
    addr::TypeAddr,
    error::{RuntimeErrorPayload, WirePayload},
    value::RuntimeValueCarrier,
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
        builtin_error_identity: PlatformBuiltinErrorIdentity,
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
        builtin_error_identity: PlatformBuiltinErrorIdentity,
        encoded_payload: Vec<u8>,
        trace_id: String,
        error_id: String,
    },
}

impl<'de> Deserialize<'de> for ServiceErrorEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ServiceErrorEnvelopeWire::deserialize(deserializer)?;
        let envelope = match wire {
            ServiceErrorEnvelopeWire::PublicTypedError {
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                trace_id,
                error_id,
            } => Self::PublicTypedError {
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                trace_id,
                error_id,
            },
            ServiceErrorEnvelopeWire::InternalError { payload } => Self::InternalError { payload },
            ServiceErrorEnvelopeWire::PlatformError {
                builtin_error_identity,
                encoded_payload,
                trace_id,
                error_id,
            } => Self::PlatformError {
                builtin_error_identity,
                encoded_payload,
                trace_id,
                error_id,
            },
        };
        envelope.validate().map_err(serde::de::Error::custom)?;
        Ok(envelope)
    }
}

impl ServiceErrorEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::PublicTypedError {
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                trace_id,
                error_id,
            } => {
                non_empty_ref("packageId", package_id)?;
                non_empty_ref("stableSchemaKey", stable_schema_key)?;
                non_empty_ref("packageSchemaTypeId", package_schema_type_id.as_str())?;
                non_empty_bytes("encodedPayload", encoded_payload)?;
                validate_correlation(trace_id, error_id)
            }
            Self::InternalError { payload } => {
                non_empty_ref("payload.message", &payload.message)?;
                validate_correlation(&payload.trace_id, &payload.error_id)
            }
            Self::PlatformError {
                encoded_payload,
                trace_id,
                error_id,
                ..
            } => {
                non_empty_bytes("encodedPayload", encoded_payload)?;
                validate_correlation(trace_id, error_id)
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

/// Strictly-decoded service error plus the exact bytes received from the
/// boundary. Unlinked services forward `encoded_bytes` without serializing the
/// envelope again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueServiceError {
    envelope: ServiceErrorEnvelope,
    encoded_bytes: Vec<u8>,
}

impl OpaqueServiceError {
    pub fn decode(encoded_bytes: Vec<u8>) -> serde_json::Result<Self> {
        let envelope = serde_json::from_slice::<ServiceErrorEnvelope>(&encoded_bytes)?;
        Ok(Self {
            envelope,
            encoded_bytes,
        })
    }

    pub fn envelope(&self) -> &ServiceErrorEnvelope {
        &self.envelope
    }

    pub fn encoded_bytes(&self) -> &[u8] {
        &self.encoded_bytes
    }

    pub fn into_encoded_bytes(self) -> Vec<u8> {
        self.encoded_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorCorrelation {
    pub trace_id: String,
    pub error_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExceptionStackFrame {
    Local {
        site: InstructionSourceSite,
    },
    RemoteBoundary {
        service_id: String,
        operation_id: String,
        error_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestExceptionCause {
    Local {
        value: RuntimeValueCarrier,
    },
    /// An imported service failure. `local_value` is present only when the
    /// fixed error was materialized into an exact caller-local value.
    OpaqueService {
        error: OpaqueServiceError,
        local_value: Option<RuntimeValueCarrier>,
    },
}

/// Request-local exception state. It is deliberately not serializable.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestException {
    cause: RequestExceptionCause,
    source: InstructionSourceSite,
    stack: Vec<ExceptionStackFrame>,
    correlation: ErrorCorrelation,
}

impl RequestException {
    pub fn local(
        value: RuntimeValueCarrier,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
        correlation: ErrorCorrelation,
    ) -> Result<Self, String> {
        if value.catch_identity().is_none() {
            return Err("local exception value requires an actual catch identity".to_string());
        }
        if stack.is_empty() {
            return Err("local exception requires a non-empty request-local stack".to_string());
        }
        validate_correlation(&correlation.trace_id, &correlation.error_id)?;
        Ok(Self {
            cause: RequestExceptionCause::Local { value },
            source,
            stack,
            correlation,
        })
    }

    pub fn imported(
        error: OpaqueServiceError,
        local_value: Option<RuntimeValueCarrier>,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
    ) -> Result<Self, String> {
        if let Some(value) = &local_value {
            if value.catch_identity().is_none() {
                return Err(
                    "imported local exception value requires an actual catch identity".to_string(),
                );
            }
        }
        let correlation = ErrorCorrelation {
            trace_id: error.envelope().trace_id().to_string(),
            error_id: error.envelope().error_id().to_string(),
        };
        Ok(Self {
            cause: RequestExceptionCause::OpaqueService { error, local_value },
            source,
            stack,
            correlation,
        })
    }

    pub fn cause(&self) -> &RequestExceptionCause {
        &self.cause
    }

    pub fn source(&self) -> &InstructionSourceSite {
        &self.source
    }

    pub fn stack(&self) -> &[ExceptionStackFrame] {
        &self.stack
    }

    pub fn correlation(&self) -> &ErrorCorrelation {
        &self.correlation
    }

    pub fn local_catch_identity(&self) -> Option<&CatchIdentity> {
        self.local_value()
            .and_then(RuntimeValueCarrier::catch_identity)
    }

    pub fn local_value(&self) -> Option<&RuntimeValueCarrier> {
        match &self.cause {
            RequestExceptionCause::Local { value } => Some(value),
            RequestExceptionCause::OpaqueService { local_value, .. } => local_value.as_ref(),
        }
    }

    pub fn fixed_service_error(&self) -> Option<&OpaqueServiceError> {
        match &self.cause {
            RequestExceptionCause::Local { .. } => None,
            RequestExceptionCause::OpaqueService { error, .. } => Some(error),
        }
    }

    pub fn map_local_value(
        self,
        map: impl FnOnce(RuntimeValueCarrier) -> RuntimeValueCarrier,
    ) -> Self {
        let cause = match self.cause {
            RequestExceptionCause::Local { value } => {
                RequestExceptionCause::Local { value: map(value) }
            }
            RequestExceptionCause::OpaqueService {
                error,
                local_value: Some(value),
            } => RequestExceptionCause::OpaqueService {
                error,
                local_value: Some(map(value)),
            },
            imported @ RequestExceptionCause::OpaqueService {
                local_value: None, ..
            } => imported,
        };
        Self { cause, ..self }
    }
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

fn non_empty_bytes(label: &str, value: &[u8]) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    Ok(())
}

fn validate_correlation(trace_id: &str, error_id: &str) -> Result<(), String> {
    non_empty_ref("traceId", trace_id)?;
    non_empty_ref("errorId", error_id)
}

#[cfg(test)]
mod tests;
