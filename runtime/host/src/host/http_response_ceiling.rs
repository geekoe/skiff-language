use skiff_runtime_request::{BoundaryResponse, ResponseEnd, ResponseEvent, ResponseStreamEvent};

use crate::error::RuntimeError;

const RESPONSE_LIMIT_CODE: &str = "ResourceLimitExceeded";

pub(super) fn validate_unary_response(
    response: &BoundaryResponse,
    max_bytes: usize,
    is_http_ingress: bool,
) -> Result<(), RuntimeError> {
    if !is_http_ingress {
        return Ok(());
    }
    let payload_bytes = match response {
        BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))
        | BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Http { payload, .. })) => {
            payload.len()
        }
        BoundaryResponse::Event(ResponseEvent::FixedServiceFailure(_))
        | BoundaryResponse::Event(ResponseEvent::Error(_))
        | BoundaryResponse::StreamSent => return Ok(()),
    };
    HttpResponseCeiling::new(max_bytes).account(payload_bytes)
}

#[derive(Debug)]
pub(super) struct HttpResponseCeiling {
    max_bytes: usize,
    emitted_bytes: usize,
}

impl HttpResponseCeiling {
    pub(super) fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            emitted_bytes: 0,
        }
    }

    pub(super) fn account_stream_event(
        &mut self,
        event: &ResponseStreamEvent,
    ) -> Result<(), RuntimeError> {
        match event {
            ResponseStreamEvent::Chunk { payload, .. } => self.account(payload.len()),
            ResponseStreamEvent::Start { .. } | ResponseStreamEvent::End => Ok(()),
        }
    }

    fn account(&mut self, bytes: usize) -> Result<(), RuntimeError> {
        let Some(next) = self.emitted_bytes.checked_add(bytes) else {
            return Err(response_limit_error(self.max_bytes));
        };
        if next > self.max_bytes {
            return Err(response_limit_error(self.max_bytes));
        }
        self.emitted_bytes = next;
        Ok(())
    }
}

fn response_limit_error(max_bytes: usize) -> RuntimeError {
    RuntimeError::ExternalErrorPayload {
        code: RESPONSE_LIMIT_CODE.to_string(),
        message: format!("HTTP response exceeds max size of {max_bytes} bytes"),
        status: None,
        details: Some(serde_json::json!({
            "resource": "http.response",
            "maxBytes": max_bytes,
        })),
    }
}

#[cfg(test)]
mod tests {
    use skiff_runtime_capability_context::HttpResponseMetadata;
    use skiff_runtime_model::service_error::OpaqueServiceError;
    use skiff_runtime_request::{
        BoundaryResponse, FixedServiceResponseFailure, ResponseEvent, ResponseStreamEvent,
    };

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
    fn streaming_http_chunks_share_one_cumulative_exact_boundary() {
        let mut ceiling = HttpResponseCeiling::new(5);
        for (seq, payload) in [(0, b"ab".to_vec()), (1, b"cde".to_vec())] {
            ceiling
                .account_stream_event(&ResponseStreamEvent::Chunk { seq, payload })
                .expect("exact cumulative boundary must succeed");
        }
    }

    #[test]
    fn streaming_http_first_cumulative_byte_over_boundary_fails() {
        let mut ceiling = HttpResponseCeiling::new(4);
        ceiling
            .account_stream_event(&ResponseStreamEvent::Chunk {
                seq: 0,
                payload: b"ab".to_vec(),
            })
            .expect("first chunk fits");
        let error = ceiling
            .account_stream_event(&ResponseStreamEvent::Chunk {
                seq: 1,
                payload: b"cde".to_vec(),
            })
            .expect_err("cumulative oversize must fail");
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
        assert!(
            validate_unary_response(&BoundaryResponse::payload(vec![0; 100]), 1, false).is_ok()
        );
    }

    #[test]
    fn fixed_service_failure_is_a_legal_terminal_not_an_http_body() {
        let fixed = OpaqueServiceError::decode(
            br#"{"kind":"internalError","payload":{"message":"The service could not complete the request.","traceId":"trace-fixed","errorId":"error-fixed"}}"#
                .to_vec(),
        )
        .expect("fixed fixture");
        let response = BoundaryResponse::Event(ResponseEvent::FixedServiceFailure(
            FixedServiceResponseFailure::new(fixed),
        ));

        assert!(validate_unary_response(&response, 0, true).is_ok());
    }
}
