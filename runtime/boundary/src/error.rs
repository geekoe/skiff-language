use std::fmt;

use serde_json::json;
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};
use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableExpectedTypePlan,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverableBoundaryErrorCode {
    UnsupportedEncode,
    UnsupportedDecode,
    CodeIdentityMissing,
    ArtifactUnavailable,
    NativeMissingAdapter,
    ExpectedTypeMismatch,
    InterfaceConformanceMissing,
    StateInvalid,
    RemoteCarrierNotPersistable,
    CrossServiceInterfaceCallbackUnavailable,
    CrossServiceRecoverableBehaviorUnavailable,
    UntrustedBehaviorPayload,
    SealedPayloadInvalid,
}

impl RecoverableBoundaryErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedEncode => "recoverableUnsupportedEncode",
            Self::UnsupportedDecode => "recoverableUnsupportedDecode",
            Self::CodeIdentityMissing => "recoverable_code_identity_missing",
            Self::ArtifactUnavailable => "recoverable_artifact_unavailable",
            Self::NativeMissingAdapter => "recoverable_native_missing_adapter",
            Self::ExpectedTypeMismatch => "recoverable_expected_type_mismatch",
            Self::InterfaceConformanceMissing => "recoverable_interface_conformance_missing",
            Self::StateInvalid => "recoverable_state_invalid",
            Self::RemoteCarrierNotPersistable => "recoverable_remote_carrier_not_persistable",
            Self::CrossServiceInterfaceCallbackUnavailable => {
                "cross_service_interface_callback_unavailable"
            }
            Self::CrossServiceRecoverableBehaviorUnavailable => {
                "cross_service_recoverable_behavior_unavailable"
            }
            Self::UntrustedBehaviorPayload => "recoverable_untrusted_behavior_payload",
            Self::SealedPayloadInvalid => "recoverable_sealed_payload_invalid",
        }
    }
}

impl fmt::Display for RecoverableBoundaryErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("recoverable boundary error {code}: {message}")]
pub struct RecoverableBoundaryError {
    code: RecoverableBoundaryErrorCode,
    message: String,
    context: RuntimeRecoverableBoundaryContext,
    expected: RuntimeRecoverableExpectedTypePlan,
    detail: Option<serde_json::Value>,
}

impl RecoverableBoundaryError {
    pub fn new(
        code: RecoverableBoundaryErrorCode,
        message: impl Into<String>,
        context: &RuntimeRecoverableBoundaryContext,
        expected: &RuntimeRecoverableExpectedTypePlan,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            context: context.clone(),
            expected: expected.clone(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn code(&self) -> RecoverableBoundaryErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn context(&self) -> &RuntimeRecoverableBoundaryContext {
        &self.context
    }

    pub fn expected(&self) -> &RuntimeRecoverableExpectedTypePlan {
        &self.expected
    }

    pub fn detail(&self) -> Option<&serde_json::Value> {
        self.detail.as_ref()
    }

    pub fn details_json(&self) -> serde_json::Value {
        let mut details = serde_json::json!({
            "context": &self.context,
            "expected": &self.expected,
        });
        if let (Some(object), Some(detail)) = (details.as_object_mut(), self.detail.as_ref()) {
            object.insert("detail".to_string(), detail.clone());
        }
        details
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("{0}")]
    InvalidArtifact(String),
    #[error("{0}")]
    Decode(String),
    #[error("decode error for {target}: {message}")]
    DecodeTarget { target: String, message: String },
    #[error("bytes decode error for {target}: {message}")]
    BytesDecode { target: String, message: String },
    #[error("db decode error for {target}: {message}")]
    DbDecode { target: String, message: String },
    #[error("file error: {message}")]
    FileError { message: String },
    #[error("http error: {message}")]
    HttpError {
        message: String,
        detail: Option<serde_json::Value>,
    },
    #[error("{0}")]
    Unsupported(String),
    #[error(transparent)]
    Recoverable(#[from] RecoverableBoundaryError),
    #[error("resource limit exceeded for {resource}: {reason}")]
    ResourceLimitExceeded {
        resource: String,
        reason: String,
        limit: usize,
        current: usize,
        requested_delta: usize,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use skiff_runtime_model::recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    };

    use super::*;

    fn recoverable_error(code: RecoverableBoundaryErrorCode) -> RecoverableBoundaryError {
        let context = RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_explicit_recoverable_slot();
        let expected = RuntimeRecoverableExpectedTypePlan::unresolved("string");

        RecoverableBoundaryError::new(
            code,
            "recoverable boundary is unsupported",
            &context,
            &expected,
        )
    }

    #[test]
    fn recoverable_error_codes_are_stable() {
        assert_eq!(
            RecoverableBoundaryErrorCode::UnsupportedEncode.as_str(),
            "recoverableUnsupportedEncode"
        );
        assert_eq!(
            RecoverableBoundaryErrorCode::UnsupportedDecode.as_str(),
            "recoverableUnsupportedDecode"
        );
        assert_eq!(
            RecoverableBoundaryErrorCode::ArtifactUnavailable.as_str(),
            "recoverable_artifact_unavailable"
        );
        assert_eq!(
            RecoverableBoundaryErrorCode::StateInvalid.as_str(),
            "recoverable_state_invalid"
        );
    }

    #[test]
    fn recoverable_details_json_contains_context_and_expected() {
        let error = recoverable_error(RecoverableBoundaryErrorCode::UnsupportedEncode);

        let details = error.details_json();
        let object = details
            .as_object()
            .expect("recoverable details should be an object");

        assert_eq!(object.len(), 2);
        assert_eq!(
            object.get("context"),
            Some(&serde_json::to_value(error.context()).expect("context should serialize"))
        );
        assert_eq!(
            object.get("expected"),
            Some(&serde_json::to_value(error.expected()).expect("expected should serialize"))
        );
    }

    #[test]
    fn recoverable_details_json_includes_optional_detail() {
        let error = recoverable_error(RecoverableBoundaryErrorCode::ArtifactUnavailable)
            .with_detail(serde_json::json!({ "artifactIdentity": "pkg/service" }));

        assert_eq!(
            error
                .details_json()
                .get("detail")
                .and_then(|detail| detail.get("artifactIdentity")),
            Some(&serde_json::json!("pkg/service"))
        );
    }

    #[test]
    fn boundary_payload_uses_domain_wire_codes() {
        let decode_target = RuntimeError::decode_target("std.json.decode", "invalid json");
        let payload = decode_target.payload();
        assert_eq!(payload.code, "std.json.DecodeError");
        assert_eq!(
            payload.details,
            Some(serde_json::json!({
                "target": "std.json.decode",
                "message": "invalid json",
            }))
        );

        let unknown_target = RuntimeError::decode_target("runtime.config", "invalid config");
        assert_eq!(unknown_target.payload().code, "InternalError");

        let resource = RuntimeError::ResourceLimitExceeded {
            resource: "response.body".to_string(),
            reason: "too large".to_string(),
            limit: 10,
            current: 8,
            requested_delta: 4,
        }
        .payload();
        assert_eq!(resource.code, "ResourceLimitExceeded");
        assert_eq!(
            resource.details,
            Some(serde_json::json!({
                "resource": "response.body",
                "reason": "too large",
                "limit": 10,
                "current": 8,
                "requestedDelta": 4,
            }))
        );
    }

    #[test]
    fn boundary_catch_projection_covers_public_catchable_variants() {
        assert_eq!(
            RuntimeError::decode_target("config.require", "missing config").catch_projection(),
            Some((
                TypeIdentity::builtin("config.DecodeError"),
                serde_json::json!({
                    "target": "config.require",
                    "message": "missing config",
                })
            ))
        );
        assert_eq!(
            RuntimeError::bytes_decode("bytes.toUtf8String", "invalid utf8").catch_projection(),
            Some((
                TypeIdentity::builtin("std.bytes.DecodeError"),
                serde_json::json!({
                    "target": "bytes.toUtf8String",
                    "message": "invalid utf8",
                })
            ))
        );
        assert_eq!(
            RuntimeError::db_decode("std.db", "missing id").catch_projection(),
            Some((
                TypeIdentity::builtin("std.db.DecodeError"),
                serde_json::json!({
                    "target": "std.db",
                    "message": "missing id",
                })
            ))
        );
        assert_eq!(
            RuntimeError::file_error("std.file not found").catch_projection(),
            Some((
                TypeIdentity::builtin("std.file.FileError"),
                serde_json::json!({
                    "message": "std.file not found",
                })
            ))
        );
        assert_eq!(
            RuntimeError::http_error(
                "std.http failed",
                Some(serde_json::json!({ "status": 500 }))
            )
            .catch_projection(),
            Some((
                TypeIdentity::builtin("std.http.HttpError"),
                serde_json::json!({
                    "message": "std.http failed",
                    "detail": { "status": 500 },
                })
            ))
        );
        assert_eq!(
            RuntimeError::decode_target("runtime.config", "invalid config").catch_projection(),
            None
        );
    }
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

impl RuntimeError {
    pub fn decode_target(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::DecodeTarget {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn bytes_decode(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::BytesDecode {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn db_decode(target: impl Into<String>, message: impl Into<String>) -> Self {
        Self::DbDecode {
            target: target.into(),
            message: message.into(),
        }
    }

    pub fn file_error(message: impl Into<String>) -> Self {
        Self::FileError {
            message: message.into(),
        }
    }

    pub fn http_error(message: impl Into<String>, detail: Option<serde_json::Value>) -> Self {
        Self::HttpError {
            message: message.into(),
            detail,
        }
    }
}

impl WirePayload for RuntimeError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::InvalidArtifact(message) => RuntimeErrorPayload {
                code: "InvalidArtifact".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::DecodeTarget { target, message } => RuntimeErrorPayload {
                code: decode_target_error_code(target)
                    .unwrap_or("InternalError")
                    .to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::BytesDecode { target, message } => RuntimeErrorPayload {
                code: "std.bytes.DecodeError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::DbDecode { target, message } => RuntimeErrorPayload {
                code: "std.db.DecodeError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::FileError { message } => RuntimeErrorPayload {
                code: "std.file.FileError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::HttpError { message, detail } => RuntimeErrorPayload {
                code: "std.http.HttpError".to_string(),
                message: message.clone(),
                status: None,
                details: detail.clone(),
            },
            Self::Unsupported(message) => RuntimeErrorPayload {
                code: "UnsupportedRuntimeFeature".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Recoverable(error) => RuntimeErrorPayload {
                code: error.code().as_str().to_string(),
                message: error.message().to_string(),
                status: None,
                details: Some(error.details_json()),
            },
            Self::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeErrorPayload {
                code: "ResourceLimitExceeded".to_string(),
                message: format!("resource limit exceeded for {resource}: {reason}"),
                status: None,
                details: Some(json!({
                    "resource": resource,
                    "reason": reason,
                    "limit": limit,
                    "current": current,
                    "requestedDelta": requested_delta,
                })),
            },
            Self::Json(error) => RuntimeErrorPayload {
                code: "JsonError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        match self {
            Self::DecodeTarget { target, message } => {
                decode_target_error_code(target).map(|code| {
                    (
                        TypeIdentity::builtin(code),
                        json!({
                            "target": target,
                            "message": message,
                        }),
                    )
                })
            }
            Self::BytesDecode { target, message } => Some((
                TypeIdentity::builtin("std.bytes.DecodeError"),
                json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::DbDecode { target, message } => Some((
                TypeIdentity::builtin("std.db.DecodeError"),
                json!({
                    "target": target,
                    "message": message,
                }),
            )),
            Self::FileError { message } => Some((
                TypeIdentity::builtin("std.file.FileError"),
                json!({
                    "message": message,
                }),
            )),
            Self::HttpError { message, detail } => Some((
                TypeIdentity::builtin("std.http.HttpError"),
                json!({
                    "message": message,
                    "detail": detail,
                }),
            )),
            Self::InvalidArtifact(_)
            | Self::Decode(_)
            | Self::Unsupported(_)
            | Self::Recoverable(_)
            | Self::ResourceLimitExceeded { .. }
            | Self::Json(_) => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub fn decode_target_error_code(target: &str) -> Option<&'static str> {
    match target {
        "std.json.decode" | "std.json.encode" | "std.resource.json" => {
            Some("std.json.DecodeError")
        }
        "config.require" | "config.optional" | "config.has" => Some("config.DecodeError"),
        "number.parse" | "number.assertSafeInteger" => Some("std.number.DecodeError"),
        target if target.starts_with("Date.") || target.starts_with("Duration.") => {
            Some("std.time.DecodeError")
        }
        _ => None,
    }
}

impl From<skiff_runtime_model::error::RuntimeModelError> for RuntimeError {
    fn from(error: skiff_runtime_model::error::RuntimeModelError) -> Self {
        match error {
            skiff_runtime_model::error::RuntimeModelError::Decode(message) => Self::Decode(message),
            skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => Self::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            },
            skiff_runtime_model::error::RuntimeModelError::Json(error) => Self::Json(error),
        }
    }
}
