use serde_json::{Map, Value};

use super::{
    call_context::HttpCallContext,
    cancel::wait_for_cancel_signals,
    response_parts::{HttpResponseHead, HttpResponseParts},
    transport::map_reqwest_error_for,
};
use crate::error::{Result, RuntimeError};
use skiff_runtime_boundary::value::bytes_value;

pub(super) enum ResponseBodyReadFailure {
    Runtime(RuntimeError),
    LimitExceeded {
        limit_bytes: usize,
        received_bytes: usize,
    },
}

impl From<RuntimeError> for ResponseBodyReadFailure {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

pub(super) async fn read_response_body(
    response: reqwest::Response,
    context: &HttpCallContext<'_, '_>,
) -> Result<Vec<u8>> {
    read_response_body_classified(response, context)
        .await
        .map_err(|failure| match failure {
            ResponseBodyReadFailure::Runtime(error) => error,
            ResponseBodyReadFailure::LimitExceeded { limit_bytes, .. } => RuntimeError::Protocol {
                target: context.target().to_string(),
                message: format!("response body exceeds max size of {limit_bytes} bytes"),
            },
        })
}

pub(super) async fn read_response_body_classified(
    mut response: reqwest::Response,
    context: &HttpCallContext<'_, '_>,
) -> std::result::Result<Vec<u8>, ResponseBodyReadFailure> {
    let max_bytes = context.service_max_response_bytes();
    let mut body = Vec::new();
    let body_loop = || async {
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| map_reqwest_error_for(context.target(), error))?
        {
            let received_bytes = body.len().saturating_add(chunk.len());
            if received_bytes > max_bytes {
                return Err(ResponseBodyReadFailure::LimitExceeded {
                    limit_bytes: max_bytes,
                    received_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    };

    if !context.cancel_signals().is_empty() {
        tokio::select! {
            body = body_loop() => body,
            _ = wait_for_cancel_signals(context.cancel_signals()) => {
                Err(ResponseBodyReadFailure::Runtime(RuntimeError::cancelled()))
            },
        }
    } else {
        body_loop().await
    }
}

pub(super) fn response_value(parts: HttpResponseParts) -> Value {
    let mut response_object = Map::new();
    response_object.insert(
        "status".to_string(),
        Value::Number(parts.head().status().into()),
    );
    response_object.insert(
        "headers".to_string(),
        build_response_headers(parts.head().headers()),
    );
    response_object.insert("body".to_string(), bytes_value(parts.body()));

    Value::Object(response_object)
}

pub(super) fn build_response_headers(response_headers: &reqwest::header::HeaderMap) -> Value {
    let mut headers = Vec::new();

    for name in response_headers.keys() {
        for value in response_headers.get_all(name) {
            headers.push(json_header(
                name.as_str(),
                value
                    .to_str()
                    .map(ToString::to_string)
                    .unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).to_string()),
            ));
        }
    }

    Value::Array(headers)
}

fn json_header(name: &str, value: impl ToString) -> Value {
    let mut object = Map::new();
    object.insert("name".to_string(), Value::String(name.to_string()));
    object.insert("value".to_string(), Value::String(value.to_string()));
    Value::Object(object)
}

pub(super) fn response_event(head: &HttpResponseHead) -> Value {
    let mut object = Map::new();
    object.insert("tag".to_string(), Value::String("response".to_string()));
    object.insert("status".to_string(), Value::Number(head.status().into()));
    object.insert(
        "headers".to_string(),
        build_response_headers(head.headers()),
    );
    Value::Object(object)
}

pub(super) fn chunk_event(tag: &str, bytes: &[u8]) -> Value {
    let mut object = Map::new();
    object.insert("tag".to_string(), Value::String(tag.to_string()));
    object.insert("value".to_string(), bytes_value(bytes));
    Value::Object(object)
}
