use std::sync::Arc;

use serde_json::Value;
use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_model::service_error::{ErrorCorrelation, ExceptionStackFrame};

use crate::CapabilityResult;

pub trait TelemetryCapabilityApi: Send + Sync {
    fn emit_native(&self, target: &str, args: &[Value]) -> CapabilityResult<Value>;
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
    restricted_service_diagnostic_sink: Arc<dyn RestrictedServiceDiagnosticSink>,
}

impl TelemetryCapabilityContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: TelemetryCapabilityApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
            restricted_service_diagnostic_sink: Arc::new(DiscardRestrictedServiceDiagnosticSink),
        }
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

    pub fn submit_restricted_service_diagnostic(
        &self,
        diagnostic: &RestrictedServiceDiagnostic,
    ) -> CapabilityResult<()> {
        self.restricted_service_diagnostic_sink.submit(diagnostic)
    }
}

#[cfg(test)]
mod tests;
