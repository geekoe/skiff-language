use serde::{Deserialize, Serialize};
use serde_json::Value;

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
