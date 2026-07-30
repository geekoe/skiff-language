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
mod tests {
    use serde_json::json;
    use skiff_artifact_model::{SourcePosition, SourceSpanRef};

    use super::*;
    use crate::{
        addr::{FileAddr, UnitAddr},
        value::{RuntimeValue, RuntimeValueCarrier},
    };

    fn site() -> InstructionSourceSite {
        InstructionSourceSite::Source {
            span: SourceSpanRef {
                source_id: 7,
                start: SourcePosition::new(3, 4),
                end: SourcePosition::new(3, 9),
            },
        }
    }

    fn public_envelope() -> ServiceErrorEnvelope {
        ServiceErrorEnvelope::PublicTypedError {
            package_id: "example.errors".to_string(),
            stable_schema_key: "NotFound".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("schema:not-found"),
            encoded_payload: br#"{"id":"42"}"#.to_vec(),
            trace_id: "trace-1".to_string(),
            error_id: "error-1".to_string(),
        }
    }

    fn internal_envelope() -> ServiceErrorEnvelope {
        ServiceErrorEnvelope::InternalError {
            payload: InternalErrorPayload {
                message: "The service could not complete the request.".to_string(),
                trace_id: "trace-1".to_string(),
                error_id: "error-2".to_string(),
            },
        }
    }

    fn platform_envelope() -> ServiceErrorEnvelope {
        ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: PlatformBuiltinErrorIdentity::DbConflict,
            encoded_payload: br#"{"retryable":true}"#.to_vec(),
            trace_id: "trace-1".to_string(),
            error_id: "error-3".to_string(),
        }
    }

    #[test]
    fn legacy_cancel_platform_error_envelope_is_rejected_by_the_finite_registry() {
        let legacy = r#"{
          "kind": "platformError",
          "builtinErrorIdentity": "CancelError",
          "encodedPayload": [],
          "traceId": "trace-cancel",
          "errorId": "error-cancel"
        }"#;

        let error = serde_json::from_str::<ServiceErrorEnvelope>(legacy).unwrap_err();
        assert!(
            error.to_string().contains("unknown variant `CancelError`"),
            "legacy identity must be rejected before payload validation: {error}"
        );
    }

    #[test]
    fn legacy_cancel_symbol_is_not_a_platform_builtin_identity() {
        assert_eq!(
            PlatformBuiltinErrorIdentity::from_symbol("CancelError"),
            None
        );
    }

    #[test]
    fn legacy_cancel_json_string_is_rejected_by_the_finite_registry() {
        assert!(serde_json::from_str::<PlatformBuiltinErrorIdentity>(r#""CancelError""#).is_err());
    }

    #[test]
    fn websocket_request_errors_keep_all_five_exact_named_union_branch_identities() {
        let owner = NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index: 42,
            },
            type_arguments: Vec::new(),
        });
        for kind in WebSocketRequestErrorKind::ALL {
            let remote = kind == WebSocketRequestErrorKind::Remote;
            let error = WebSocketRequestError::new(
                owner.clone(),
                kind,
                "sanitized",
                remote.then_some(-32603),
                remote.then(|| json!({"peer": true})),
            )
            .expect("exact WebSocket request branch");
            assert_eq!(
                error.exact_catch_identity(),
                CatchIdentity::NamedUnionBranch {
                    union: owner.clone(),
                    branch: NamedUnionBranchIdentity::SyntheticDiscriminator {
                        discriminator_field: "kind".to_string(),
                        discriminator_value: kind.discriminator().to_string(),
                    },
                }
            );
            assert_eq!(
                error.catch_projection().unwrap().1["kind"],
                kind.discriminator()
            );
        }
        assert_eq!(
            PlatformBuiltinErrorIdentity::from_symbol("std.websocket.WebSocketRequestError"),
            None
        );
        assert_eq!(
            PlatformBuiltinErrorIdentity::JsonDecode.catch_identity(),
            CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(
                PlatformBuiltinErrorIdentity::JsonDecode
            ))
        );
        assert_eq!(
            PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
            CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(
                PlatformBuiltinErrorIdentity::Timeout
            ))
        );
    }

    #[test]
    fn timeout_platform_identity_and_envelope_round_trip_unchanged() {
        let identity = PlatformBuiltinErrorIdentity::Timeout;

        assert_eq!(identity.symbol(), "TimeoutError");
        assert_eq!(
            PlatformBuiltinErrorIdentity::from_symbol("TimeoutError"),
            Some(identity)
        );
        assert_eq!(
            identity.catch_identity(),
            CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(identity))
        );

        let identity_json = serde_json::to_string(&identity).unwrap();
        assert_eq!(identity_json, r#""TimeoutError""#);
        assert_eq!(
            serde_json::from_str::<PlatformBuiltinErrorIdentity>(&identity_json).unwrap(),
            identity
        );

        let envelope = ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: identity,
            encoded_payload: br#"{"message":"deadline exceeded"}"#.to_vec(),
            trace_id: "trace-timeout".to_string(),
            error_id: "error-timeout".to_string(),
        };
        let wire = serde_json::to_vec(&envelope).unwrap();
        assert_eq!(
            serde_json::from_slice::<ServiceErrorEnvelope>(&wire).unwrap(),
            envelope
        );
    }

    fn exact_public_bytes() -> Vec<u8> {
        br#"{
          "kind":"publicTypedError",
          "packageId":"example.errors",
          "stableSchemaKey":"NotFound",
          "packageSchemaTypeId":"schema:not-found",
          "encodedPayload":[123,125],
          "traceId":"trace-1",
          "errorId":"error-1"
        }"#
        .to_vec()
    }

    fn local_identity(type_index: usize) -> CatchIdentity {
        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
            LocalExecutionTypeIdentity {
                addr: TypeAddr {
                    unit: UnitAddr::Service,
                    file: FileAddr::loaded_file(0),
                    type_index,
                },
                type_arguments: Vec::new(),
            },
        ))
    }

    #[test]
    fn service_error_envelopes_round_trip_all_variants() {
        let envelopes = [public_envelope(), internal_envelope(), platform_envelope()];

        for expected in envelopes {
            let wire = serde_json::to_value(&expected).unwrap();
            assert_eq!(
                serde_json::from_value::<ServiceErrorEnvelope>(wire).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn service_error_envelope_strictly_rejects_invalid_wire() {
        let base = serde_json::to_value(public_envelope()).unwrap();
        let mut cases = Vec::new();

        let mut unknown_variant = base.clone();
        unknown_variant["kind"] = json!("futureError");
        cases.push(unknown_variant);

        let mut extra = base.clone();
        extra["details"] = json!({});
        cases.push(extra);

        for missing in [
            "packageId",
            "stableSchemaKey",
            "packageSchemaTypeId",
            "encodedPayload",
            "traceId",
            "errorId",
        ] {
            let mut value = base.clone();
            value.as_object_mut().unwrap().remove(missing);
            cases.push(value);
        }

        let mut empty_owner = base.clone();
        empty_owner["packageId"] = json!(" ");
        cases.push(empty_owner);

        let mut unknown_builtin = serde_json::to_value(ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: PlatformBuiltinErrorIdentity::DbConflict,
            encoded_payload: vec![1],
            trace_id: "trace".to_string(),
            error_id: "error".to_string(),
        })
        .unwrap();
        unknown_builtin["builtinErrorIdentity"] = json!("std.resource.ResourceError");
        cases.push(unknown_builtin);

        let mut internal_extra = serde_json::to_value(ServiceErrorEnvelope::InternalError {
            payload: InternalErrorPayload {
                message: "sanitized".to_string(),
                trace_id: "trace".to_string(),
                error_id: "error".to_string(),
            },
        })
        .unwrap();
        internal_extra["payload"]["details"] = json!({ "private": true });
        cases.push(internal_extra);

        let mut internal_missing = serde_json::to_value(ServiceErrorEnvelope::InternalError {
            payload: InternalErrorPayload {
                message: "sanitized".to_string(),
                trace_id: "trace".to_string(),
                error_id: "error".to_string(),
            },
        })
        .unwrap();
        internal_missing["payload"]
            .as_object_mut()
            .unwrap()
            .remove("message");
        cases.push(internal_missing);

        for case in cases {
            assert!(
                serde_json::from_value::<ServiceErrorEnvelope>(case).is_err(),
                "invalid service envelope must fail closed"
            );
        }
    }

    #[test]
    fn linked_imported_error_catches_exactly_and_preserves_fixed_bytes() {
        let encoded = exact_public_bytes();
        let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
        let identity = local_identity(4);
        let local_value =
            RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
        let exception =
            RequestException::imported(opaque, Some(local_value), site(), Vec::new()).unwrap();

        assert_eq!(exception.local_catch_identity(), Some(&identity));
        assert_eq!(
            exception.fixed_service_error().unwrap().encoded_bytes(),
            encoded
        );
        let RequestExceptionCause::OpaqueService {
            error,
            local_value: Some(local_value),
        } = exception.cause()
        else {
            panic!("expected linked imported cause");
        };
        assert_eq!(error.encoded_bytes(), encoded);
        assert_eq!(local_value.catch_identity(), Some(&identity));

        let mapped = exception.map_local_value(|_| {
            RuntimeValueCarrier::identified(RuntimeValue::from("moved"), identity.clone())
        });
        assert_eq!(mapped.local_catch_identity(), Some(&identity));
        assert_eq!(
            mapped.fixed_service_error().unwrap().encoded_bytes(),
            encoded
        );
    }

    #[test]
    fn unlinked_imported_error_misses_catch_and_map_keeps_fixed_bytes() {
        let encoded = exact_public_bytes();
        let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
        let exception = RequestException::imported(opaque, None, site(), Vec::new()).unwrap();

        assert_eq!(exception.local_catch_identity(), None);
        assert_eq!(exception.local_value(), None);
        let mapped = exception.map_local_value(|_| panic!("None must not materialize a carrier"));
        assert_eq!(mapped.local_catch_identity(), None);
        assert_eq!(mapped.local_value(), None);
        assert_eq!(
            mapped.fixed_service_error().unwrap().encoded_bytes(),
            encoded
        );
    }

    #[test]
    fn every_fixed_error_kind_can_retain_a_local_carrier() {
        for (type_index, expected) in [
            (7, public_envelope()),
            (8, internal_envelope()),
            (9, platform_envelope()),
        ] {
            let encoded = serde_json::to_vec(&expected).unwrap();
            let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
            let identity = local_identity(type_index);
            let local_value =
                RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
            let exception =
                RequestException::imported(opaque, Some(local_value), site(), Vec::new()).unwrap();

            assert_eq!(exception.local_catch_identity(), Some(&identity));
            assert_eq!(
                exception.fixed_service_error().unwrap().envelope(),
                &expected
            );
            assert_eq!(
                exception.fixed_service_error().unwrap().encoded_bytes(),
                encoded
            );
        }
    }

    #[test]
    fn local_exception_rethrow_state_stays_local_and_has_no_fixed_error() {
        let identity = local_identity(4);
        let source = site();
        let stack = vec![
            ExceptionStackFrame::Local {
                site: source.clone(),
            },
            ExceptionStackFrame::RemoteBoundary {
                service_id: "skiff.run/catalog".to_string(),
                operation_id: "lookup".to_string(),
                error_id: "error".to_string(),
            },
        ];
        let correlation = ErrorCorrelation {
            trace_id: "trace".to_string(),
            error_id: "error".to_string(),
        };
        let value =
            RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
        let exception =
            RequestException::local(value, source.clone(), stack.clone(), correlation.clone())
                .unwrap();
        let rethrown = exception.map_local_value(|_| {
            RuntimeValueCarrier::identified(RuntimeValue::from("moved"), identity.clone())
        });

        assert_eq!(rethrown.local_catch_identity(), Some(&identity));
        assert_eq!(rethrown.fixed_service_error(), None);
        assert_eq!(rethrown.source(), &source);
        assert_eq!(rethrown.stack(), stack);
        assert_eq!(rethrown.correlation(), &correlation);
        assert!(matches!(
            rethrown.cause(),
            RequestExceptionCause::Local { .. }
        ));
    }

    #[test]
    fn local_exception_rejects_missing_identity_stack_and_correlation() {
        let identity = local_identity(5);
        let correlation = ErrorCorrelation {
            trace_id: "trace".to_string(),
            error_id: "error".to_string(),
        };
        assert!(RequestException::local(
            RuntimeValue::from("payload").into(),
            site(),
            vec![ExceptionStackFrame::Local { site: site() }],
            correlation.clone(),
        )
        .is_err());
        assert!(RequestException::local(
            RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone(),),
            site(),
            Vec::new(),
            correlation,
        )
        .is_err());
        assert!(RequestException::local(
            RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity,),
            site(),
            vec![ExceptionStackFrame::Local { site: site() }],
            ErrorCorrelation {
                trace_id: " ".to_string(),
                error_id: "error".to_string(),
            },
        )
        .is_err());
    }

    #[test]
    fn imported_error_rejects_an_unidentified_local_value() {
        let opaque = OpaqueServiceError::decode(exact_public_bytes()).unwrap();
        assert!(RequestException::imported(
            opaque,
            Some(RuntimeValue::from("payload").into()),
            site(),
            Vec::new(),
        )
        .is_err());
    }

    #[test]
    fn opaque_service_error_decode_remains_strict() {
        let malformed = br#"{
          "kind":"internalError",
          "payload":{
            "message":"sanitized",
            "traceId":"trace",
            "errorId":"error",
            "private":true
          }
        }"#
        .to_vec();

        assert!(OpaqueServiceError::decode(malformed).is_err());
    }
}
