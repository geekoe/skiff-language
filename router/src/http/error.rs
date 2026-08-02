//! Platform HTTP errors (TS `GatewayError` parity for the assembly gateway).

use std::fmt;

use serde_json::{json, Value};

const FIXED_SERVICE_ERROR_MESSAGE: &str = "Service request failed";

/// A platform error projected to the HTTP client.
///
/// `json_body()` emits the canonical `{ "error": { code, message, details } }`
/// shape. `include_details` follows the TS detail policy per error class.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub details: Option<Box<Value>>,
    include_details: bool,
}

impl HttpError {
    pub fn platform(
        status: u16,
        code: impl Into<String>,
        message: impl Into<String>,
        details: Option<Value>,
    ) -> Self {
        // TS `GatewayError.toHttpBody`: status >= 500 hides details.
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: details.map(Box::new),
            include_details: status < 500,
        }
    }

    pub fn control_error(
        code: impl Into<String>,
        message: impl Into<String>,
        status: Option<u16>,
        details: Option<Value>,
    ) -> Self {
        let code = code.into();
        let message = message.into();
        let status = runtime_error_status(&code, status);
        Self {
            status,
            code: code.clone(),
            message,
            details: runtime_error_http_detail(&code, status, details).map(Box::new),
            include_details: true,
        }
    }

    pub fn fixed_service(trace_id: &str, error_id: &str) -> Self {
        Self {
            status: 500,
            code: "FixedServiceError".to_string(),
            message: FIXED_SERVICE_ERROR_MESSAGE.to_string(),
            details: Some(Box::new(
                json!({ "traceId": trace_id, "errorId": error_id }),
            )),
            include_details: true,
        }
    }

    pub fn timeout(timeout_ms: u64) -> Self {
        Self {
            status: 504,
            code: "TimeoutError".to_string(),
            message: format!("Runtime did not respond within {timeout_ms}ms"),
            details: Some(Box::new(json!({ "timeoutMs": timeout_ms }))),
            include_details: false,
        }
    }

    pub fn provider_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: 503,
            code: "std.service.ProviderUnavailableError".to_string(),
            message: message.into(),
            details: None,
            include_details: false,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: 500,
            code: "InternalGatewayError".to_string(),
            message: message.into(),
            details: None,
            include_details: false,
        }
    }

    pub fn json_body(&self) -> Value {
        let details = if self.include_details {
            self.details.as_deref().cloned()
        } else {
            None
        };
        let mut error = json!({
            "code": self.code,
            "message": self.message,
        });
        if let Some(details) = details {
            error["details"] = details;
        }
        json!({ "error": error })
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}: {}", self.status, self.code, self.message)
    }
}

impl std::error::Error for HttpError {}

fn runtime_error_status(code: &str, status: Option<u16>) -> u16 {
    if status.is_some_and(|status| (400..=599).contains(&status)) {
        return status.expect("checked");
    }
    match code {
        "std.bytes.DecodeError"
        | "std.number.DecodeError"
        | "std.json.DecodeError"
        | "std.db.DecodeError"
        | "std.file.FileError"
        | "std.time.DecodeError"
        | "config.DecodeError"
        | "std.http.HttpError"
        | "RequestDecodeError" => 400,
        "std.db.ConflictError" => 409,
        "std.service.ProviderUnavailableError" => 503,
        "TimeoutError" => 504,
        "std.service.ProtocolError" | "UnexpectedChunk" | "UnsupportedRuntimeTransport" => 502,
        _ => 500,
    }
}

fn runtime_error_http_detail(code: &str, status: u16, details: Option<Value>) -> Option<Value> {
    let decode_error = matches!(
        code,
        "HttpError"
            | "std.http.HttpError"
            | "std.bytes.DecodeError"
            | "std.number.DecodeError"
            | "std.json.DecodeError"
            | "std.db.ConflictError"
            | "std.db.DecodeError"
            | "std.file.FileError"
            | "std.time.DecodeError"
            | "config.DecodeError"
            | "RequestDecodeError"
    );
    if decode_error || (400..500).contains(&status) {
        details
    } else {
        None
    }
}
