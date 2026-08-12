use serde_json::json;

use super::{RuntimeErrorPayload, WirePayload};
use crate::service_error::PlatformBuiltinErrorIdentity;

#[derive(Debug)]
struct DiagnosticOnlyError;

impl std::fmt::Display for DiagnosticOnlyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("diagnostic-only failure")
    }
}

impl std::error::Error for DiagnosticOnlyError {}

impl WirePayload for DiagnosticOnlyError {
    fn payload(&self) -> RuntimeErrorPayload {
        RuntimeErrorPayload {
            code: PlatformBuiltinErrorIdentity::Timeout.symbol().to_string(),
            message: self.to_string(),
            status: None,
            details: None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn diagnostic_code_does_not_grant_a_catch_projection() {
    let error = DiagnosticOnlyError;
    let payload = error.payload();

    assert_eq!(payload.code, "TimeoutError");
    assert_eq!(payload.to_string(), "diagnostic-only failure");
    assert_eq!(
        serde_json::to_value(&payload).expect("diagnostic payload serializes"),
        json!({
            "code": "TimeoutError",
            "message": "diagnostic-only failure",
        })
    );
    assert_eq!(error.catch_projection(), None);
}
