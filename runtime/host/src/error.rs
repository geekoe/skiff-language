use std::fmt;

use serde_json::{json, Map, Value};

pub use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::{CatchIdentity, PlatformBuiltinErrorIdentity},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSource {
    pub assembly_id: Option<u32>,
    pub source_id: u64,
}

#[derive(Debug, thiserror::Error)]
/// Host cancellation is an internal completion terminal, not a wire error.
///
/// ```compile_fail
/// use skiff_runtime_host::error::{RuntimeError, WirePayload};
///
/// let _ = WirePayload::payload(&RuntimeError::Cancelled);
/// ```
pub enum RuntimeError {
    #[error("{0}")]
    Decode(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("provider unavailable for {target}: {reason}")]
    ProviderUnavailable { target: String, reason: String },
    #[error("protocol error for {target}: {message}")]
    #[allow(dead_code)]
    Protocol { target: String, message: String },
    #[error("request was cancelled")]
    Cancelled,
    #[error("execution budget exceeded: {reason:?}")]
    ExecutionBudgetExceeded {
        reason: skiff_runtime_capability_context::ExecutionBudgetReason,
        instruction_count: u64,
        limit: Option<u64>,
        elapsed_ms: f64,
    },
    #[error("{0}")]
    Diagnosed(Diagnosed),
    #[error("{message}")]
    ExternalErrorPayload {
        code: String,
        message: String,
        status: Option<u16>,
        details: Option<Value>,
    },
    #[error("{0}")]
    Opaque(Box<dyn WirePayload>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostRuntimeLeafError {
    #[error("{0}")]
    InvalidArtifact(String),
    #[error("decode error for {target}: {message}")]
    DecodeTarget { target: String, message: String },
    #[error("http error: {message}")]
    HttpError {
        message: String,
        detail: Option<Value>,
    },
    #[error("{0}")]
    FileError(String),
    #[error("resource limit exceeded for {resource}: {reason}")]
    ResourceLimitExceeded {
        resource: String,
        reason: String,
        limit: usize,
        current: usize,
        requested_delta: usize,
    },
}

impl WirePayload for HostRuntimeLeafError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::InvalidArtifact(message) => RuntimeErrorPayload {
                code: "InvalidArtifact".to_string(),
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
                details: Some(serde_json::json!({
                    "target": target,
                    "message": message,
                })),
            },
            Self::HttpError { message, detail } => RuntimeErrorPayload {
                code: "std.http.HttpError".to_string(),
                message: message.clone(),
                status: None,
                details: detail.clone(),
            },
            Self::FileError(message) => RuntimeErrorPayload {
                code: "std.file.FileError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
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
                details: Some(serde_json::json!({
                    "resource": resource,
                    "reason": reason,
                    "limit": limit,
                    "current": current,
                    "requestedDelta": requested_delta,
                })),
            },
        }
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            Self::DecodeTarget { target, message } => {
                decode_target_error_code(target).and_then(|code| {
                    let identity = PlatformBuiltinErrorIdentity::from_symbol(code)?;
                    Some((
                        identity.catch_identity(),
                        serde_json::json!({
                            "target": target,
                            "message": message,
                        }),
                    ))
                })
            }
            Self::HttpError { message, detail } => Some((
                PlatformBuiltinErrorIdentity::Http.catch_identity(),
                serde_json::json!({
                    "message": message,
                    "detail": detail,
                }),
            )),
            Self::FileError(message) => Some((
                PlatformBuiltinErrorIdentity::File.catch_identity(),
                serde_json::json!({
                    "message": message,
                }),
            )),
            Self::InvalidArtifact(_) | Self::ResourceLimitExceeded { .. } => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Debug)]
pub struct Diagnosed {
    frames: Vec<DiagnosticFrame>,
    inner: DiagnosedInner,
}

#[derive(Debug)]
enum DiagnosedInner {
    Runtime(Box<RuntimeError>),
    Wire(Box<dyn WirePayload>),
}

impl DiagnosedInner {
    fn as_wire(&self) -> Option<&dyn WirePayload> {
        match self {
            Self::Runtime(_) => None,
            Self::Wire(error) => Some(error.as_ref()),
        }
    }

    fn as_runtime_error(&self) -> Option<&RuntimeError> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::Wire(_) => None,
        }
    }

    fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        match self {
            Self::Runtime(error) => error.ordinary_payload(),
            Self::Wire(error) => Some(error.payload()),
        }
    }

    fn ordinary_catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            Self::Runtime(error) => error.ordinary_catch_projection(),
            Self::Wire(error) => error.catch_projection(),
        }
    }

    fn is_cancellation_terminal(&self) -> bool {
        self.as_runtime_error()
            .is_some_and(RuntimeError::is_cancellation_terminal)
    }
}

#[derive(Debug)]
pub(crate) enum DiagnosticFrame {
    Source { source_id: u64, frame: Box<Value> },
    Diagnostic { frame: Box<Value> },
}

impl DiagnosticFrame {
    fn source(source_id: u64, frame: Value) -> Self {
        Self::Source {
            source_id,
            frame: Box::new(frame),
        }
    }

    fn diagnostic(frame: Value) -> Self {
        Self::Diagnostic {
            frame: Box::new(frame),
        }
    }

    fn is_diagnostic(&self) -> bool {
        matches!(self, Self::Diagnostic { .. })
    }

    fn source_context_matches(&self, source_id: u64, frame: &Value) -> bool {
        match self {
            Self::Source {
                source_id: existing_source_id,
                frame: existing_frame,
            } => source_context_matches(*existing_source_id, existing_frame, source_id, frame),
            Self::Diagnostic { .. } => false,
        }
    }

    fn merge_into(&self, payload: &mut RuntimeErrorPayload) {
        match self {
            Self::Source { source_id, frame } => {
                add_source_frame(payload, *source_id, (**frame).clone());
            }
            Self::Diagnostic { frame } => {
                add_diagnostic_frame(payload, (**frame).clone());
            }
        }
    }

    fn diagnostic_source(&self) -> Option<DiagnosticSource> {
        match self {
            Self::Source { source_id, frame } => {
                diagnostic_source_from_frame(frame).or(Some(DiagnosticSource {
                    assembly_id: None,
                    source_id: *source_id,
                }))
            }
            Self::Diagnostic { frame } => diagnostic_source_from_frame(frame),
        }
    }
}

impl Diagnosed {
    pub fn source(source_id: u64, frame: Value, inner: Box<dyn WirePayload>) -> Self {
        Self {
            frames: vec![DiagnosticFrame::source(source_id, frame)],
            inner: DiagnosedInner::Wire(inner),
        }
    }

    pub fn diagnostic(frame: Value, inner: Box<dyn WirePayload>) -> Self {
        Self {
            frames: vec![DiagnosticFrame::diagnostic(frame)],
            inner: DiagnosedInner::Wire(inner),
        }
    }

    fn runtime_source(source_id: u64, frame: Value, inner: RuntimeError) -> Self {
        Self {
            frames: vec![DiagnosticFrame::source(source_id, frame)],
            inner: DiagnosedInner::Runtime(Box::new(inner)),
        }
    }

    fn runtime_diagnostic(frame: Value, inner: RuntimeError) -> Self {
        Self {
            frames: vec![DiagnosticFrame::diagnostic(frame)],
            inner: DiagnosedInner::Runtime(Box::new(inner)),
        }
    }

    fn with_source(mut self, source_id: u64, source_frame: Value) -> Self {
        if self.has_source_context(source_id, &source_frame) {
            return self;
        }
        let insert_at = self
            .frames
            .iter()
            .take_while(|frame| frame.is_diagnostic())
            .count();
        self.frames
            .insert(insert_at, DiagnosticFrame::source(source_id, source_frame));
        self
    }

    fn with_diagnostic_frame(mut self, frame: Value) -> Self {
        if self
            .frames
            .first()
            .is_some_and(DiagnosticFrame::is_diagnostic)
        {
            return self;
        }
        self.frames.insert(0, DiagnosticFrame::diagnostic(frame));
        self
    }

    pub fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        let mut payload = self.inner.ordinary_payload()?;
        for frame in self.frames.iter().rev() {
            frame.merge_into(&mut payload);
        }
        Some(payload)
    }

    fn is_cancellation_terminal(&self) -> bool {
        self.inner.is_cancellation_terminal()
    }

    pub fn diagnostic_source_id(&self) -> Option<u64> {
        self.diagnostic_source().map(|source| source.source_id)
    }

    pub fn diagnostic_source(&self) -> Option<DiagnosticSource> {
        self.inner
            .as_runtime_error()
            .and_then(RuntimeError::diagnostic_source)
            .or_else(|| {
                self.inner
                    .as_wire()?
                    .as_any()
                    .downcast_ref::<OrdinaryRuntimeError>()
                    .and_then(|error| error.error().diagnostic_source())
            })
            .or_else(|| {
                self.frames
                    .iter()
                    .rev()
                    .find_map(DiagnosticFrame::diagnostic_source)
            })
    }

    pub(crate) fn try_into_runtime_parts(
        self,
    ) -> std::result::Result<(RuntimeError, Vec<DiagnosticFrame>), Self> {
        match self.inner {
            DiagnosedInner::Runtime(error) => Ok((*error, self.frames)),
            DiagnosedInner::Wire(error) => Err(Self {
                frames: self.frames,
                inner: DiagnosedInner::Wire(error),
            }),
        }
    }

    fn inner_diagnosed(&self) -> Option<&Diagnosed> {
        self.inner.as_runtime_error().and_then(|error| match error {
            RuntimeError::Diagnosed(diagnosed) => Some(diagnosed),
            _ => None,
        })
    }

    fn has_source_context(&self, source_id: u64, frame: &Value) -> bool {
        self.frames
            .iter()
            .any(|existing| existing.source_context_matches(source_id, frame))
            || self
                .inner
                .as_runtime_error()
                .is_some_and(|inner| error_has_source_context(inner, source_id, frame))
            || self
                .inner_diagnosed()
                .is_some_and(|inner| inner.has_source_context(source_id, frame))
    }
}

impl fmt::Display for Diagnosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            DiagnosedInner::Runtime(error) => write!(formatter, "{error}"),
            DiagnosedInner::Wire(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for Diagnosed {}

impl From<skiff_runtime_eval::error::RuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_eval::error::RuntimeError) -> Self {
        if error.is_cancellation_terminal() {
            return RuntimeError::Cancelled;
        }
        match error {
            skiff_runtime_eval::error::RuntimeError::WithSource {
                source_id,
                frame,
                error,
            } => RuntimeError::from(*error).with_source(source_id, *frame),
            skiff_runtime_eval::error::RuntimeError::WithDiagnosticFrame { frame, error } => {
                RuntimeError::from(*error).with_diagnostic_frame(*frame)
            }
            skiff_runtime_eval::error::RuntimeError::RootRuntimePayload(payload) => {
                RuntimeError::ExternalErrorPayload {
                    code: payload.code,
                    message: payload.message,
                    status: payload.status,
                    details: payload.details,
                }
            }
            skiff_runtime_eval::error::RuntimeError::Opaque(error) => RuntimeError::Opaque(error),
            error => RuntimeError::Opaque(Box::new(
                skiff_runtime_eval::error::OrdinaryRuntimeError::try_new(error)
                    .expect("eval cancellation was split before ordinary trait erasure"),
            )),
        }
    }
}

impl From<skiff_runtime_model::error::RuntimeModelError> for RuntimeError {
    fn from(error: skiff_runtime_model::error::RuntimeModelError) -> Self {
        RuntimeError::Opaque(Box::new(error))
    }
}

impl From<skiff_runtime_boundary::error::RuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_boundary::error::RuntimeError) -> Self {
        RuntimeError::Opaque(Box::new(error))
    }
}

impl From<skiff_runtime_linked_type_plan::Error> for RuntimeError {
    fn from(error: skiff_runtime_linked_type_plan::Error) -> Self {
        RuntimeError::Opaque(Box::new(error))
    }
}

impl From<skiff_runtime_native::error::RuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_native::error::RuntimeError) -> Self {
        if error.is_cancellation_terminal() {
            RuntimeError::Cancelled
        } else {
            RuntimeError::Opaque(Box::new(
                skiff_runtime_native::error::OrdinaryRuntimeError::try_new(error)
                    .expect("native cancellation was split before ordinary trait erasure"),
            ))
        }
    }
}

impl From<skiff_runtime_capability_context::ExecutionControlError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::ExecutionControlError) -> Self {
        match error {
            skiff_runtime_capability_context::ExecutionControlError::Cancelled => {
                RuntimeError::Cancelled
            }
            skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(failure) => {
                RuntimeError::execution_budget_exceeded(failure)
            }
        }
    }
}

#[derive(Debug)]
struct OrdinaryStreamRuntimeError(skiff_runtime_capability_context::StreamRuntimeError);

impl fmt::Display for OrdinaryStreamRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for OrdinaryStreamRuntimeError {}

impl WirePayload for OrdinaryStreamRuntimeError {
    fn payload(&self) -> RuntimeErrorPayload {
        self.0
            .ordinary_payload()
            .expect("ordinary stream wrapper construction excludes cancellation")
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        self.0.ordinary_catch_projection()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl From<skiff_runtime_capability_context::StreamRuntimeError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::StreamRuntimeError) -> Self {
        if error.is_cancellation_terminal() {
            RuntimeError::Cancelled
        } else {
            RuntimeError::Opaque(Box::new(OrdinaryStreamRuntimeError(error)))
        }
    }
}

impl From<skiff_runtime_capability_context::FileCapabilityError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::FileCapabilityError) -> Self {
        use skiff_runtime_capability_context::FileCapabilityError;

        match error {
            FileCapabilityError::Decode(message) => RuntimeError::Decode(message),
            FileCapabilityError::File(message) => RuntimeError::file_error(message),
            FileCapabilityError::Opaque(error) => RuntimeError::Opaque(error),
            FileCapabilityError::ProviderUnavailable { target, reason } => {
                RuntimeError::ProviderUnavailable { target, reason }
            }
            FileCapabilityError::ResourceLimitExceeded {
                resource,
                reason,
                limit,
                current,
                requested_delta,
            } => RuntimeError::resource_limit_exceeded(
                resource,
                reason,
                limit,
                current,
                requested_delta,
            ),
            FileCapabilityError::Stream(error) => RuntimeError::from(error),
            FileCapabilityError::Execution(error) => RuntimeError::from(error),
        }
    }
}

impl From<skiff_runtime_capability_context::CapabilityError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::CapabilityError) -> Self {
        RuntimeError::Opaque(Box::new(error))
    }
}

impl From<skiff_runtime_capability_context::DbCapabilityError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::DbCapabilityError) -> Self {
        RuntimeError::Opaque(Box::new(error))
    }
}

impl From<skiff_runtime_capability_context::RequestPayloadContextError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::RequestPayloadContextError) -> Self {
        RuntimeError::Opaque(Box::new(error))
    }
}

impl From<skiff_runtime_capability_context::OutboundRequestRegistryError> for RuntimeError {
    fn from(error: skiff_runtime_capability_context::OutboundRequestRegistryError) -> Self {
        RuntimeError::Opaque(Box::new(error))
    }
}

impl RuntimeError {
    pub(crate) fn invalid_artifact(message: impl Into<String>) -> Self {
        RuntimeError::Opaque(Box::new(HostRuntimeLeafError::InvalidArtifact(
            message.into(),
        )))
    }

    pub(crate) fn decode_target(target: impl Into<String>, message: impl Into<String>) -> Self {
        RuntimeError::Opaque(Box::new(HostRuntimeLeafError::DecodeTarget {
            target: target.into(),
            message: message.into(),
        }))
    }

    pub(crate) fn file_error(message: impl Into<String>) -> Self {
        RuntimeError::Opaque(Box::new(HostRuntimeLeafError::FileError(message.into())))
    }

    pub(crate) fn http_error(message: impl Into<String>, detail: Option<Value>) -> Self {
        RuntimeError::Opaque(Box::new(HostRuntimeLeafError::HttpError {
            message: message.into(),
            detail,
        }))
    }

    pub(crate) fn cancelled() -> Self {
        RuntimeError::Cancelled
    }

    pub(crate) fn execution_budget_exceeded(
        failure: skiff_runtime_capability_context::ExecutionBudgetFailure,
    ) -> Self {
        if failure.reason.is_cancellation_terminal() {
            RuntimeError::Cancelled
        } else {
            RuntimeError::ExecutionBudgetExceeded {
                reason: failure.reason,
                instruction_count: failure.instruction_count,
                limit: failure.limit,
                elapsed_ms: failure.elapsed_ms,
            }
        }
    }

    pub(crate) fn resource_limit_exceeded(
        resource: impl Into<String>,
        reason: impl Into<String>,
        limit: usize,
        current: usize,
        requested_delta: usize,
    ) -> Self {
        RuntimeError::Opaque(Box::new(HostRuntimeLeafError::ResourceLimitExceeded {
            resource: resource.into(),
            reason: reason.into(),
            limit,
            current,
            requested_delta,
        }))
    }

    pub fn with_source(self, source_id: u64, source_frame: Value) -> Self {
        if error_has_source_context(&self, source_id, &source_frame) {
            return self;
        }
        match self {
            RuntimeError::Diagnosed(diagnosed) => {
                RuntimeError::Diagnosed(diagnosed.with_source(source_id, source_frame))
            }
            error => {
                RuntimeError::Diagnosed(Diagnosed::runtime_source(source_id, source_frame, error))
            }
        }
    }

    pub fn with_diagnostic_frame(self, frame: Value) -> Self {
        match self {
            RuntimeError::Diagnosed(diagnosed) => {
                RuntimeError::Diagnosed(diagnosed.with_diagnostic_frame(frame))
            }
            error => RuntimeError::Diagnosed(Diagnosed::runtime_diagnostic(frame, error)),
        }
    }

    pub fn is_cancellation_terminal(&self) -> bool {
        match self {
            RuntimeError::Cancelled => true,
            RuntimeError::ExecutionBudgetExceeded { reason, .. } => {
                reason.is_cancellation_terminal()
            }
            RuntimeError::Diagnosed(diagnosed) => diagnosed.is_cancellation_terminal(),
            RuntimeError::Decode(_)
            | RuntimeError::Unsupported(_)
            | RuntimeError::ProviderUnavailable { .. }
            | RuntimeError::Protocol { .. }
            | RuntimeError::ExternalErrorPayload { .. }
            | RuntimeError::Opaque(_)
            | RuntimeError::Json(_) => false,
        }
    }

    pub fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        Some(match self {
            RuntimeError::Diagnosed(diagnosed) => return diagnosed.ordinary_payload(),
            RuntimeError::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            RuntimeError::Unsupported(message) => RuntimeErrorPayload {
                code: "UnsupportedRuntimeFeature".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            RuntimeError::ProviderUnavailable { target, reason } => RuntimeErrorPayload {
                code: "std.service.ProviderUnavailableError".to_string(),
                message: reason.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "reason": reason,
                })),
            },
            RuntimeError::Protocol { target, message } => RuntimeErrorPayload {
                code: "std.service.ProtocolError".to_string(),
                message: message.clone(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "message": message,
                })),
            },
            RuntimeError::Cancelled => return None,
            RuntimeError::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } => {
                if reason.is_cancellation_terminal() {
                    return None;
                }
                RuntimeErrorPayload {
                    code: "TimeoutError".to_string(),
                    message: match reason {
                        skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded => {
                            "execution deadline exceeded".to_string()
                        }
                        skiff_runtime_capability_context::ExecutionBudgetReason::InstructionLimitExceeded => {
                            "execution instruction limit exceeded".to_string()
                        }
                        skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled => {
                            unreachable!("cancellation terminal was split above")
                        }
                    },
                    status: None,
                    details: Some(serde_json::json!({
                        "reason": reason.as_str(),
                        "instructionCount": instruction_count,
                        "limit": limit,
                        "elapsedMs": elapsed_ms,
                    })),
                }
            }
            RuntimeError::ExternalErrorPayload {
                code,
                message,
                status,
                details,
            } => RuntimeErrorPayload {
                code: code.clone(),
                message: message.clone(),
                status: *status,
                details: details.clone(),
            },
            RuntimeError::Opaque(error) => error.payload(),
            RuntimeError::Json(error) => RuntimeErrorPayload {
                code: "JsonError".to_string(),
                message: error.to_string(),
                status: None,
                details: None,
            },
        })
    }

    pub fn ordinary_catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            RuntimeError::Diagnosed(error) => error.inner.ordinary_catch_projection(),
            RuntimeError::Opaque(error) => error.catch_projection(),
            RuntimeError::ProviderUnavailable { target, reason } => Some((
                PlatformBuiltinErrorIdentity::ServiceProviderUnavailable.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "reason": reason,
                }),
            )),
            RuntimeError::Protocol { target, message } => Some((
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                serde_json::json!({
                    "target": target,
                    "message": message,
                }),
            )),
            RuntimeError::ExecutionBudgetExceeded {
                reason,
                instruction_count,
                limit,
                elapsed_ms,
            } if !reason.is_cancellation_terminal() => Some((
                PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
                serde_json::json!({
                    "reason": reason.as_str(),
                    "instructionCount": instruction_count,
                    "limit": limit,
                    "elapsedMs": elapsed_ms,
                }),
            )),
            RuntimeError::Decode(_)
            | RuntimeError::Unsupported(_)
            | RuntimeError::Cancelled
            | RuntimeError::ExecutionBudgetExceeded { .. }
            | RuntimeError::ExternalErrorPayload { .. }
            | RuntimeError::Json(_) => None,
        }
    }

    pub fn diagnostic_source_id(&self) -> Option<u64> {
        self.diagnostic_source().map(|source| source.source_id)
    }

    pub fn diagnostic_source(&self) -> Option<DiagnosticSource> {
        match self {
            RuntimeError::Diagnosed(diagnosed) => diagnosed.diagnostic_source(),
            _ => None,
        }
    }
}

/// Ordinary-only Host carrier used at dynamic wire boundaries.
#[derive(Debug)]
pub(crate) struct OrdinaryRuntimeError(RuntimeError);

impl OrdinaryRuntimeError {
    pub(crate) fn try_new(error: RuntimeError) -> std::result::Result<Self, RuntimeError> {
        if error.is_cancellation_terminal() {
            return Err(error);
        }
        Ok(Self(error))
    }

    pub(crate) fn error(&self) -> &RuntimeError {
        &self.0
    }
}

impl fmt::Display for OrdinaryRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for OrdinaryRuntimeError {}

impl WirePayload for OrdinaryRuntimeError {
    fn payload(&self) -> RuntimeErrorPayload {
        self.0
            .ordinary_payload()
            .expect("OrdinaryRuntimeError construction excludes cancellation")
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        self.0.ordinary_catch_projection()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) fn decode_target_error_code(target: &str) -> Option<&'static str> {
    match target {
        "std.json.decode" | "std.json.encode" | "std.resource.json" => Some("std.json.DecodeError"),
        "config.require" | "config.optional" | "config.has" => Some("config.DecodeError"),
        "number.parse" | "number.assertSafeInteger" => Some("std.number.DecodeError"),
        target if target.starts_with("Date.") || target.starts_with("Duration.") => {
            Some("std.time.DecodeError")
        }
        _ => None,
    }
}

fn diagnostic_source_from_frame(frame: &Value) -> Option<DiagnosticSource> {
    Some(DiagnosticSource {
        assembly_id: source_assembly_id(frame),
        source_id: frame.get("sourceId").and_then(Value::as_u64)?,
    })
}

fn error_has_source_context(error: &RuntimeError, source_id: u64, frame: &Value) -> bool {
    match error {
        RuntimeError::Diagnosed(diagnosed) => diagnosed.has_source_context(source_id, frame),
        _ => false,
    }
}

fn source_context_matches(
    existing_source_id: u64,
    existing_frame: &Value,
    source_id: u64,
    frame: &Value,
) -> bool {
    if existing_source_id != source_id {
        return false;
    }
    let existing_assembly_id = source_assembly_id(existing_frame);
    let assembly_id = source_assembly_id(frame);
    existing_assembly_id == assembly_id || existing_assembly_id.is_none() || assembly_id.is_none()
}

fn source_assembly_id(frame: &Value) -> Option<u32> {
    frame
        .get("assemblyId")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn add_source_frame(payload: &mut RuntimeErrorPayload, source_id: u64, frame: Value) {
    let mut details = details_as_object(payload.details.take());
    details.insert("sourceId".to_string(), json!(source_id));
    details.insert("sourceFrame".to_string(), frame.clone());
    match details.get_mut("sourceFrames") {
        Some(Value::Array(frames)) => frames.insert(0, frame.clone()),
        Some(existing) => {
            let previous = existing.take();
            *existing = Value::Array(vec![frame.clone(), previous]);
        }
        None => {
            details.insert(
                "sourceFrames".to_string(),
                Value::Array(vec![frame.clone()]),
            );
        }
    }
    add_frame_to_details(&mut details, frame);
    payload.details = Some(Value::Object(details));
}

fn add_diagnostic_frame(payload: &mut RuntimeErrorPayload, frame: Value) {
    let mut details = details_as_object(payload.details.take());
    add_frame_to_details(&mut details, frame);
    payload.details = Some(Value::Object(details));
}

fn details_as_object(details: Option<Value>) -> Map<String, Value> {
    match details {
        Some(Value::Object(object)) => object,
        Some(details) => {
            let mut object = Map::new();
            object.insert("originalDetails".to_string(), details);
            object
        }
        None => Map::new(),
    }
}

fn add_frame_to_details(details: &mut Map<String, Value>, frame: Value) {
    match details.get_mut("frames") {
        Some(Value::Array(frames)) => frames.insert(0, frame),
        Some(existing) => {
            let previous = existing.take();
            *existing = Value::Array(vec![frame, previous]);
        }
        _ => {
            details.insert("frames".to_string(), Value::Array(vec![frame]));
        }
    }
}

#[cfg(test)]
#[path = "error/tests.rs"]
mod tests;
