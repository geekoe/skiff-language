use skiff_runtime_boundary::error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode};
use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
    RuntimeRecoverableTrustBoundary,
};

use super::*;

fn recoverable_boundary_error() -> RecoverableBoundaryError {
    let context = RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    let expected = RuntimeRecoverableExpectedTypePlan::unresolved("string");

    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::UnsupportedDecode,
        "recoverable decode is unsupported",
        &context,
        &expected,
    )
}

#[test]
fn boundary_recoverable_response_uses_boundary_details_contract() {
    let error = recoverable_boundary_error();
    let expected_details = error.details_json();

    let response = RequestError::Boundary(
        skiff_runtime_boundary::error::RuntimeError::Recoverable(error),
    )
    .ordinary_response_error()
    .expect("boundary failure is ordinary");

    assert_eq!(response.code, "recoverableUnsupportedDecode");
    assert_eq!(response.status, None);
    assert_eq!(response.details, Some(expected_details));
}

#[test]
fn response_error_is_derived_from_ordinary_payload() {
    let error = RequestError::ExecutionBudgetExceeded {
        reason: ExecutionBudgetReason::DeadlineExceeded,
        instruction_count: 42,
        limit: Some(100),
        elapsed_ms: 12.5,
    };

    let payload = error.ordinary_payload().expect("deadline is ordinary");
    let response = error
        .ordinary_response_error()
        .expect("deadline is ordinary");

    assert_eq!(response.code, payload.code);
    assert_eq!(response.message, payload.message);
    assert_eq!(response.status, payload.status);
    assert_eq!(response.details, payload.details);
}

#[test]
fn cancellation_terminal_has_no_ordinary_payload_catch_or_response_projection() {
    for error in [
        RequestError::Cancelled,
        RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
        RequestError::Eval(EvalRuntimeError::Cancelled),
        RequestError::Eval(
            EvalRuntimeError::Cancelled
                .with_diagnostic_frame(serde_json::json!({ "operation": "request.cancel" })),
        ),
    ] {
        assert!(error.is_cancellation_terminal());
        assert_eq!(error.ordinary_payload(), None);
        assert_eq!(error.ordinary_catch_projection(), None);
        assert_eq!(error.ordinary_response_error(), None);
    }
}

#[test]
fn request_ordinary_payload_preserves_external_payload_shape() {
    let error = RequestError::external_error_payload(
        "DownstreamError".to_string(),
        "downstream failed".to_string(),
        Some(503),
        Some(serde_json::json!({ "service": "account" })),
    );

    let payload = error
        .ordinary_payload()
        .expect("external error is ordinary");

    assert_eq!(payload.code, "DownstreamError");
    assert_eq!(payload.message, "downstream failed");
    assert_eq!(payload.status, Some(503));
    assert_eq!(
        payload.details,
        Some(serde_json::json!({ "service": "account" }))
    );
    assert_eq!(error.ordinary_catch_projection(), None);
}

#[test]
fn request_ordinary_payload_delegates_boundary_projection() {
    let boundary = skiff_runtime_boundary::error::RuntimeError::http_error(
        "std.http failed",
        Some(serde_json::json!({ "status": 500 })),
    );
    let expected_payload = boundary.payload();
    let expected_catch_projection = boundary.catch_projection();
    let error = RequestError::Boundary(boundary);

    assert_eq!(error.ordinary_payload(), Some(expected_payload));
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
}

#[test]
fn request_catch_projection_covers_only_ordinary_errors() {
    assert_eq!(
        RequestError::protocol("svc.account", "bad frame").ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
            serde_json::json!({
                "target": "svc.account",
                "message": "bad frame",
            })
        ))
    );
    assert_eq!(RequestError::Cancelled.ordinary_catch_projection(), None);
    assert_eq!(
        RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::InstructionLimitExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        }
        .ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
            serde_json::json!({
                "reason": "instructionLimitExceeded",
                "instructionCount": 42,
                "limit": 100,
                "elapsedMs": 12.5,
            })
        ))
    );
}

#[test]
fn fixed_service_failure_is_extracted_only_from_the_typed_eval_carrier() {
    let encoded = br#"{
          "kind":"internalError",
          "payload":{
            "message":"The service could not complete the request.",
            "traceId":"trace-request-fixed",
            "errorId":"error-request-fixed"
          }
        }"#
    .to_vec();
    let fixed = OpaqueServiceError::decode(encoded.clone()).expect("fixed fixture");
    let error = RequestError::Eval(EvalRuntimeError::WithDiagnosticFrame {
        frame: Box::new(serde_json::json!({
            "message": "provider-private-secret",
        })),
        error: Box::new(EvalRuntimeError::FixedServiceFailure(fixed)),
    });

    let extracted = error
        .fixed_service_failure()
        .expect("typed fixed carrier must remain available to request");
    assert_eq!(extracted.encoded_bytes(), encoded);

    let generic = RequestError::external_error_payload(
        "InternalError".to_string(),
        "canonical service failure".to_string(),
        Some(500),
        None,
    );
    assert!(
        generic.fixed_service_failure().is_none(),
        "matching generic control values must not be upgraded"
    );
}

#[test]
fn fixed_service_response_failure_preserves_all_envelope_bytes() {
    let fixtures = [
            br#"{"kind":"publicTypedError","packageId":"example.com/errors","stableSchemaKey":"not-found","packageSchemaTypeId":"type:not-found","encodedPayload":[123,125],"traceId":"trace-public","errorId":"error-public"}"#
                .as_slice(),
            br#"{
              "kind":"internalError",
              "payload":{
                "message":"The service could not complete the request.",
                "traceId":"trace-internal",
                "errorId":"error-internal"
              }
            }"#
            .as_slice(),
            br#"{"kind":"platformError","builtinErrorIdentity":"std.db.ConflictError","encodedPayload":[123,125],"traceId":"trace-platform","errorId":"error-platform"}"#
                .as_slice(),
        ];

    for encoded in fixtures {
        let encoded = encoded.to_vec();
        let fixed = OpaqueServiceError::decode(encoded.clone()).expect("fixed fixture");
        let error = RequestError::Eval(EvalRuntimeError::FixedServiceFailure(fixed));

        let response = error
            .fixed_service_response_failure()
            .expect("typed response failure");
        assert_eq!(response.error().encoded_bytes(), encoded);
    }
}
