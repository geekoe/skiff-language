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
