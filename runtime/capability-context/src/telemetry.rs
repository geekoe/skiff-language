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
mod tests {
    use std::sync::Mutex;

    use skiff_artifact_model::SyntheticInstructionSiteReason;

    use super::*;
    use crate::CapabilityError;

    struct TestTelemetry;

    impl TelemetryCapabilityApi for TestTelemetry {
        fn emit_native(&self, _target: &str, _args: &[Value]) -> CapabilityResult<Value> {
            Err(CapabilityError::unsupported(
                "user telemetry is independent from restricted diagnostics",
            ))
        }
    }

    struct RecordingSink(Arc<Mutex<Vec<RestrictedServiceDiagnostic>>>);

    impl RestrictedServiceDiagnosticSink for RecordingSink {
        fn submit(&self, diagnostic: &RestrictedServiceDiagnostic) -> CapabilityResult<()> {
            self.0.lock().unwrap().push(diagnostic.clone());
            Ok(())
        }
    }

    struct FailingSink;

    impl RestrictedServiceDiagnosticSink for FailingSink {
        fn submit(&self, _diagnostic: &RestrictedServiceDiagnostic) -> CapabilityResult<()> {
            Err(CapabilityError::provider_unavailable(
                "restricted-service-diagnostic",
                "test sink unavailable",
            ))
        }
    }

    fn diagnostic() -> RestrictedServiceDiagnostic {
        let source = InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::RuntimeBoundaryDispatch,
        };
        RestrictedServiceDiagnostic {
            owner: RestrictedServiceDiagnosticOwner {
                provider_service_id: "service:provider".to_string(),
                operation_id: "operation:run".to_string(),
                provider_activation_id: "activation:provider".to_string(),
                request_generation: 7,
            },
            correlation: ErrorCorrelation {
                trace_id: "trace:restricted".to_string(),
                error_id: "trace:restricted:error:1".to_string(),
            },
            source: source.clone(),
            stack: vec![ExceptionStackFrame::Local { site: source }],
            cause_kind: RestrictedServiceDiagnosticCauseKind::InternalError,
        }
    }

    #[test]
    fn restricted_service_diagnostic_sink_is_clone_safe_and_not_emit_native() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let context = TelemetryCapabilityContext::new(TestTelemetry)
            .with_restricted_service_diagnostic_sink(RecordingSink(Arc::clone(&records)));

        context
            .clone()
            .submit_restricted_service_diagnostic(&diagnostic())
            .unwrap();

        assert_eq!(*records.lock().unwrap(), vec![diagnostic()]);
        assert!(context.emit_native("log", &[]).is_err());
    }

    #[test]
    fn restricted_service_diagnostic_default_discards_and_failure_is_reported_to_caller() {
        TelemetryCapabilityContext::new(TestTelemetry)
            .submit_restricted_service_diagnostic(&diagnostic())
            .unwrap();

        assert!(TelemetryCapabilityContext::new(TestTelemetry)
            .with_restricted_service_diagnostic_sink(FailingSink)
            .submit_restricted_service_diagnostic(&diagnostic())
            .is_err());
    }
}
