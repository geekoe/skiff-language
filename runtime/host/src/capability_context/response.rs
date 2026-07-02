//! Runtime-error to response-error mapping helper.

use skiff_runtime_capability_context::ResponseError;

use crate::error::RuntimeError;

pub fn response_error_from_runtime_error(error: &RuntimeError) -> ResponseError {
    let payload = error.payload();
    ResponseError {
        code: payload.code,
        message: payload.message,
        status: payload.status,
        details: payload.details,
    }
}
