//! Runtime-error to response-error mapping helper.

use skiff_runtime_capability_context::ResponseError;

use crate::error::RuntimeError;

pub fn response_error_from_runtime_error(error: &RuntimeError) -> Option<ResponseError> {
    let payload = error.ordinary_payload()?;
    Some(ResponseError {
        code: payload.code,
        message: payload.message,
        status: payload.status,
        details: payload.details,
    })
}

impl skiff_runtime_transport::response_mapper::OrdinaryResponseErrorSource for RuntimeError {
    fn ordinary_response_error(&self) -> Option<ResponseError> {
        response_error_from_runtime_error(self)
    }
}
