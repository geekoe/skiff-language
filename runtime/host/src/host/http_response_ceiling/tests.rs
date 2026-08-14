use skiff_runtime_capability_context::HttpResponseMetadata;
use skiff_runtime_model::service_error::OpaqueServiceError;
use skiff_runtime_request::{BoundaryResponse, FixedServiceResponseFailure, ResponseEvent};

use super::*;

#[test]
fn unary_http_exact_boundary_succeeds() {
    let response = BoundaryResponse::http(vec![0; 4], HttpResponseMetadata::new(200, vec![]));
    assert!(validate_unary_response(&response, 4, true).is_ok());
}

#[test]
fn unary_http_first_byte_over_boundary_is_canonical_error() {
    let response = BoundaryResponse::http(vec![0; 5], HttpResponseMetadata::new(200, vec![]));
    let error = validate_unary_response(&response, 4, true).expect_err("oversize must fail");
    assert_eq!(
        error
            .ordinary_payload()
            .expect("response limit is ordinary")
            .code,
        RESPONSE_LIMIT_CODE
    );
}

#[test]
fn non_http_responses_do_not_consume_http_budget() {
    assert!(validate_unary_response(&BoundaryResponse::payload(vec![0; 100]), 1, false).is_ok());
}

#[test]
fn fixed_service_failure_is_a_legal_terminal_not_an_http_body() {
    let fixed = OpaqueServiceError::internal_error(
        "The service could not complete the request.",
        "trace-fixed",
        "error-fixed",
    )
    .expect("fixed fixture");
    let response = BoundaryResponse::Event(ResponseEvent::FixedServiceFailure(
        FixedServiceResponseFailure::new(fixed),
    ));

    assert!(validate_unary_response(&response, 0, true).is_ok());
}
