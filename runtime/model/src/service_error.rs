use serde::{Deserialize, Deserializer, Serialize};
use skiff_artifact_model::{InstructionSourceSite, LiteralIr, PackageSchemaTypeId};

use crate::{addr::TypeAddr, value::RuntimeValueCarrier};

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
    #[serde(rename = "CancelError")]
    Cancel,
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
    Local { value: RuntimeValueCarrier },
    OpaqueService { error: OpaqueServiceError },
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
        validate_correlation(&correlation.trace_id, &correlation.error_id)?;
        Ok(Self {
            cause: RequestExceptionCause::Local { value },
            source,
            stack,
            correlation,
        })
    }

    pub fn opaque(
        error: OpaqueServiceError,
        source: InstructionSourceSite,
        stack: Vec<ExceptionStackFrame>,
    ) -> Self {
        let correlation = ErrorCorrelation {
            trace_id: error.envelope().trace_id().to_string(),
            error_id: error.envelope().error_id().to_string(),
        };
        Self {
            cause: RequestExceptionCause::OpaqueService { error },
            source,
            stack,
            correlation,
        }
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
        match &self.cause {
            RequestExceptionCause::Local { value } => value.catch_identity(),
            RequestExceptionCause::OpaqueService { .. } => None,
        }
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
        let envelopes = [
            public_envelope(),
            ServiceErrorEnvelope::InternalError {
                payload: InternalErrorPayload {
                    message: "The service could not complete the request.".to_string(),
                    trace_id: "trace-1".to_string(),
                    error_id: "error-2".to_string(),
                },
            },
            ServiceErrorEnvelope::PlatformError {
                builtin_error_identity: PlatformBuiltinErrorIdentity::DbConflict,
                encoded_payload: br#"{"retryable":true}"#.to_vec(),
                trace_id: "trace-1".to_string(),
                error_id: "error-3".to_string(),
            },
        ];

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
    fn opaque_service_error_preserves_exact_bytes_and_has_no_local_identity() {
        let encoded = br#"{
          "kind":"publicTypedError",
          "packageId":"example.errors",
          "stableSchemaKey":"NotFound",
          "packageSchemaTypeId":"schema:not-found",
          "encodedPayload":[123,125],
          "traceId":"trace-1",
          "errorId":"error-1"
        }"#
        .to_vec();
        let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
        let exception = RequestException::opaque(opaque, site(), Vec::new());

        assert_eq!(exception.local_catch_identity(), None);
        let RequestExceptionCause::OpaqueService { error } = exception.cause() else {
            panic!("expected opaque cause");
        };
        assert_eq!(error.encoded_bytes(), encoded);
    }

    #[test]
    fn local_exception_requires_and_exposes_actual_identity() {
        let identity = local_identity(4);
        let value =
            RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
        let exception = RequestException::local(
            value,
            site(),
            vec![ExceptionStackFrame::Local { site: site() }],
            ErrorCorrelation {
                trace_id: "trace".to_string(),
                error_id: "error".to_string(),
            },
        )
        .unwrap();

        assert_eq!(exception.local_catch_identity(), Some(&identity));
        assert_eq!(exception.stack().len(), 1);
    }
}
