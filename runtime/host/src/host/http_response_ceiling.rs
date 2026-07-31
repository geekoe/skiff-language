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
mod tests;
