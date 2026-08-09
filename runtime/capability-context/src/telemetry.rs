use std::sync::Arc;

use serde_json::Value;
use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_model::service_error::{ErrorCorrelation, ExceptionStackFrame};

use crate::CapabilityResult;

pub trait TelemetryCapabilityApi: Send + Sync {
    fn emit_native(&self, target: &str, args: &[Value]) -> CapabilityResult<Value>;
}

/// Finite origin classification for one Runtime-owned exception log.
///
/// These values are deliberately independent from the exception payload. They
/// are safe to project into operational business logs without accepting an
/// arbitrary error message or reason from user/runtime data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeExceptionLogReason {
    Throw,
    RuntimeProjection,
    Timeout,
    Internal,
}

/// Closed, payload-free description supplied when Eval first allocates an
/// exception correlation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeExceptionLogMetadata {
    pub identity: String,
    pub identity_hash: String,
    pub reason: RuntimeExceptionLogReason,
    pub callable: Option<String>,
}

/// Runtime-internal exception log handoff.
///
/// The DTO intentionally cannot carry a heap value, arbitrary exception
/// payload, user-authored message/reason, source text, or open-ended attrs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeExceptionLog {
    pub correlation: ErrorCorrelation,
    pub metadata: RuntimeExceptionLogMetadata,
}

/// Internal sink for Runtime-owned exception logs. This stays separate from
/// the user-visible telemetry effect surface while allowing the host to route
/// the record through the same business-log pipeline as `std.log.error`.
pub trait RuntimeExceptionLogSink: Send + Sync {
    fn submit(&self, log: &RuntimeExceptionLog) -> CapabilityResult<()>;
}

#[derive(Debug, Default)]
struct DiscardRuntimeExceptionLogSink;

impl RuntimeExceptionLogSink for DiscardRuntimeExceptionLogSink {
    fn submit(&self, _log: &RuntimeExceptionLog) -> CapabilityResult<()> {
        Ok(())
    }
}

/// Clone-safe provider ownership attached to one restricted service diagnostic.
///
/// This is an internal runtime handoff. It intentionally carries no heap,
/// runtime-value, type-address, or open-ended attribute state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestrictedServiceDiagnosticOwner {
    pub provider_service_id: String,
    pub operation_id: String,
    pub provider_activation_id: String,
    pub request_generation: u64,
}

/// Finite, public-safe classification of the already-fixed service cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestrictedServiceDiagnosticCauseKind {
    PublicTypedError,
    InternalError,
    PlatformError,
}

/// Provider-local diagnostic retained separately from the fixed service wire error.
///
/// The value is deliberately not serializable. A later host/telemetry checkpoint
/// owns projection into a restricted telemetry DTO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestrictedServiceDiagnostic {
    pub owner: RestrictedServiceDiagnosticOwner,
    pub correlation: ErrorCorrelation,
    pub source: InstructionSourceSite,
    pub stack: Vec<ExceptionStackFrame>,
    pub cause_kind: RestrictedServiceDiagnosticCauseKind,
}

/// Runtime-internal handoff for restricted service diagnostics.
///
/// This seam is separate from [`TelemetryCapabilityApi::emit_native`], which
/// remains the user-visible telemetry effect surface.
pub trait RestrictedServiceDiagnosticSink: Send + Sync {
    fn submit(&self, diagnostic: &RestrictedServiceDiagnostic) -> CapabilityResult<()>;
}

#[derive(Debug, Default)]
struct DiscardRestrictedServiceDiagnosticSink;

impl RestrictedServiceDiagnosticSink for DiscardRestrictedServiceDiagnosticSink {
    fn submit(&self, _diagnostic: &RestrictedServiceDiagnostic) -> CapabilityResult<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct TelemetryCapabilityContext {
    inner: Arc<dyn TelemetryCapabilityApi>,
    runtime_exception_log_sink: Arc<dyn RuntimeExceptionLogSink>,
    restricted_service_diagnostic_sink: Arc<dyn RestrictedServiceDiagnosticSink>,
}

impl TelemetryCapabilityContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: TelemetryCapabilityApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
            runtime_exception_log_sink: Arc::new(DiscardRuntimeExceptionLogSink),
            restricted_service_diagnostic_sink: Arc::new(DiscardRestrictedServiceDiagnosticSink),
        }
    }

    pub fn with_runtime_exception_log_sink<T>(mut self, sink: T) -> Self
    where
        T: RuntimeExceptionLogSink + 'static,
    {
        self.runtime_exception_log_sink = Arc::new(sink);
        self
    }

    pub fn with_restricted_service_diagnostic_sink<T>(mut self, sink: T) -> Self
    where
        T: RestrictedServiceDiagnosticSink + 'static,
    {
        self.restricted_service_diagnostic_sink = Arc::new(sink);
        self
    }

    pub fn emit_native(&self, target: &str, args: &[Value]) -> CapabilityResult<Value> {
        self.inner.emit_native(target, args)
    }

    pub fn submit_runtime_exception_log(&self, log: &RuntimeExceptionLog) -> CapabilityResult<()> {
        self.runtime_exception_log_sink.submit(log)
    }

    pub fn submit_restricted_service_diagnostic(
        &self,
        diagnostic: &RestrictedServiceDiagnostic,
    ) -> CapabilityResult<()> {
        self.restricted_service_diagnostic_sink.submit(diagnostic)
    }
}

#[cfg(test)]
mod tests;
