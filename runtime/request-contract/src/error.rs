use std::fmt;

use serde::Serialize;
use serde_json::Value;

use crate::service_error::CatchIdentity;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuntimeErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl fmt::Display for RuntimeErrorPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub trait WirePayload: std::error::Error + Send + Sync + 'static {
    fn payload(&self) -> RuntimeErrorPayload;

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any;
}

#[cfg(test)]
mod tests;
