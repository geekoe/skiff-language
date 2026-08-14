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

#[test]
fn phase_5_bytecode_http_server_stream_has_no_second_terminal_response() {
    let message = response_into_transport_message(
        "request-stream-complete".to_string(),
        BoundaryResponse::StreamSent,
    )
    .expect("completed stream is a valid boundary response");

    assert!(
        message.is_none(),
        "the acknowledged response.end is the stream's only terminal frame"
    );
}

#[test]
fn phase_5_bytecode_http_server_stream_preserves_validated_response_ceiling() {
    assert_eq!(
        assembly::validated_http_response_max_bytes(8192).get(),
        8192
    );
}

#[test]
#[should_panic(expected = "Router bootstrap validates http.maxResponseBytes")]
fn phase_5_bytecode_http_server_stream_rejects_impossible_zero_response_ceiling() {
    let _ = assembly::validated_http_response_max_bytes(0);
}
