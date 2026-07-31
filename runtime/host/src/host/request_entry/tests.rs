use skiff_runtime_capability_context::ExecutionBudgetReason;
use skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity;

use crate::error::RuntimeError;

use super::*;

#[test]
fn request_error_bridge_boxes_and_delegates_payload_and_catch_projection() {
    let request_error = RequestError::protocol("svc.account", "bad frame");
    let expected_payload = request_error
        .ordinary_payload()
        .expect("protocol failure is ordinary");
    let expected_catch_projection = request_error.ordinary_catch_projection();

    let error = request_error_into_runtime_error(request_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(
        error.ordinary_payload().expect("bridged error is ordinary"),
        expected_payload
    );
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
    assert_eq!(
        error.ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
            serde_json::json!({
                "target": "svc.account",
                "message": "bad frame",
            })
        ))
    );
}

#[test]
fn request_error_bridge_preserves_carried_cancellation_detection() {
    let error = request_error_into_runtime_error(RequestError::Cancelled);
    assert!(matches!(error, RuntimeError::Cancelled));
    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);

    let error = request_error_into_runtime_error(RequestError::ExecutionBudgetExceeded {
        reason: ExecutionBudgetReason::Cancelled,
        instruction_count: 0,
        limit: None,
        elapsed_ms: 0.0,
    });
    assert!(matches!(error, RuntimeError::Cancelled));
    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);
}
