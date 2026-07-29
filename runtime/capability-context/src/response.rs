use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_runtime_model::service_error::OpaqueServiceError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpNameValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponseMetadata {
    pub status: u16,
    pub headers: Vec<HttpNameValue>,
}

impl HttpResponseMetadata {
    pub fn new(status: u16, headers: Vec<HttpNameValue>) -> Self {
        Self { status, headers }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResponseError {
    pub code: String,
    pub message: String,
    pub status: Option<u16>,
    pub details: Option<Value>,
}

/// Projection accepted by response framing after cancellation terminals were excluded.
///
/// The producer owns its concrete error type. Transport consumes only this lower-layer response
/// contract and therefore never requires request or Host to implement a transport-owned trait.
pub trait OrdinaryResponseErrorSource {
    fn ordinary_response_error(&self) -> Option<ResponseError>;
}

/// Strict fixed service failure carried by `response.error`.
///
/// This is deliberately separate from [`ResponseError`]: generic response
/// metadata has no authority to classify a service error by code or message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedServiceResponseFailure {
    error: OpaqueServiceError,
}

impl FixedServiceResponseFailure {
    pub fn new(error: OpaqueServiceError) -> Self {
        Self { error }
    }

    pub fn error(&self) -> &OpaqueServiceError {
        &self.error
    }

    pub fn into_error(self) -> OpaqueServiceError {
        self.error
    }
}
