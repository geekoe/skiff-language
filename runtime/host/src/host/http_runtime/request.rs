use std::sync::atomic::AtomicBool;

use serde_json::Value;
use skiff_runtime_capability_context::CancellationSignals;

use super::{
    call_context::HttpCallContext,
    response::{
        read_response_body, read_response_body_classified, response_value, ResponseBodyReadFailure,
    },
    response_parts::{HttpResponseHead, HttpResponseParts},
    transport::{send_request, send_request_without_input_timeout},
};
use crate::{
    capability_context::{HttpRuntimeOptions, TARGET_STD_HTTP_REQUEST},
    error::Result,
};

pub(crate) enum HttpRequestLowerFailure {
    Runtime(crate::error::RuntimeError),
    ResponseLimitExceeded {
        limit_bytes: usize,
        received_bytes: usize,
    },
}

impl From<crate::error::RuntimeError> for HttpRequestLowerFailure {
    fn from(error: crate::error::RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

#[allow(dead_code)]
pub async fn request(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    service_max_response_bytes: usize,
    cancelled: Option<&AtomicBool>,
) -> Result<Value> {
    request_with_options(
        input,
        frame_deadline_ms,
        service_max_response_bytes,
        cancelled,
        HttpRuntimeOptions::from_env(),
    )
    .await
}

pub(crate) async fn request_with_options(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    service_max_response_bytes: usize,
    cancelled: Option<&AtomicBool>,
    options: HttpRuntimeOptions,
) -> Result<Value> {
    request_inner(
        input,
        frame_deadline_ms,
        service_max_response_bytes,
        cancelled,
        options,
    )
    .await
}

pub(super) async fn request_inner(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    service_max_response_bytes: usize,
    cancelled: Option<&AtomicBool>,
    options: HttpRuntimeOptions,
) -> Result<Value> {
    request_with_cancellation_and_options(
        input,
        frame_deadline_ms,
        service_max_response_bytes,
        CancellationSignals::from_borrowed_flag(cancelled),
        options,
    )
    .await
}

pub(crate) async fn request_with_cancellation_and_options(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    service_max_response_bytes: usize,
    cancellation: CancellationSignals<'_>,
    options: HttpRuntimeOptions,
) -> Result<Value> {
    let context = HttpCallContext::new(
        input,
        frame_deadline_ms,
        service_max_response_bytes,
        cancellation,
        options,
        TARGET_STD_HTTP_REQUEST,
    );

    let response = send_request(&context).await?;
    let head = HttpResponseHead::from_response(&response);
    let body = read_response_body(response, &context).await?;

    Ok(response_value(HttpResponseParts::new(head, body)))
}

/// Current-scope bytecode request lower.
///
/// It reuses the production parser/egress/cache/transport but leaves the
/// request's primitive timeout to the caller's typed scope race and preserves
/// exact response-cap facts instead of converting them to a protocol string.
pub(crate) async fn request_without_input_timeout_with_cancellation_and_options(
    input: &Value,
    service_max_response_bytes: usize,
    cancellation: CancellationSignals<'_>,
    options: HttpRuntimeOptions,
) -> std::result::Result<Value, HttpRequestLowerFailure> {
    let context = HttpCallContext::new(
        input,
        None,
        service_max_response_bytes,
        cancellation,
        options,
        TARGET_STD_HTTP_REQUEST,
    );

    let response = send_request_without_input_timeout(&context).await?;
    let head = HttpResponseHead::from_response(&response);
    let body = read_response_body_classified(response, &context)
        .await
        .map_err(|failure| match failure {
            ResponseBodyReadFailure::Runtime(error) => HttpRequestLowerFailure::Runtime(error),
            ResponseBodyReadFailure::LimitExceeded {
                limit_bytes,
                received_bytes,
            } => HttpRequestLowerFailure::ResponseLimitExceeded {
                limit_bytes,
                received_bytes,
            },
        })?;

    Ok(response_value(HttpResponseParts::new(head, body)))
}
