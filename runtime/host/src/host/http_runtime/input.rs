use reqwest::{
    header::{HeaderName, HeaderValue},
    Method, Url,
};
use serde_json::Value;
use skiff_runtime_boundary::value::{
    bytes_payload, decode_base64, is_internal_metadata_key, BYTES_BASE64_KEY,
};

use crate::error::{Result, RuntimeError};

const SUPPORTED_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

#[cfg(test)]
const REDACTED_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "cookie",
    "set-cookie",
];

#[derive(Debug)]
pub(super) struct ParsedRequest {
    pub(super) method: Method,
    pub(super) url: Url,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) timeout_ms: Option<u64>,
}

fn http_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::http_error(message.into(), None)
}

pub(super) fn parse_input(input: &Value) -> Result<ParsedRequest> {
    let object = input
        .as_object()
        .ok_or_else(|| http_error("std.http.request input must be an object".to_string()))?;
    validate_request_fields(object)?;

    let method = normalize_method(as_str_array_value(
        object
            .get("method")
            .ok_or_else(|| http_error("std.http.request missing method".to_string()))?,
        "method",
    )?)?;

    let raw_url = as_str_array_value(
        object
            .get("url")
            .ok_or_else(|| http_error("std.http.request missing url".to_string()))?,
        "url",
    )?;
    let url = Url::parse(raw_url)
        .map_err(|_| http_error("std.http.request.url is invalid".to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(http_error(format!(
            "std.http.request.url must use http or https scheme, got {}",
            url.scheme()
        )));
    }
    if url.host().is_none() {
        return Err(http_error(
            "std.http.request.url must be absolute".to_string(),
        ));
    }

    let headers = parse_headers(object.get("headers"))?;
    let body = parse_body(object.get("body"))?;
    let timeout_ms = parse_timeout_ms(object.get("timeoutMs"))?;

    Ok(ParsedRequest {
        method,
        url,
        headers,
        body,
        timeout_ms,
    })
}

fn validate_request_fields(object: &serde_json::Map<String, Value>) -> Result<()> {
    for key in object.keys() {
        if matches!(
            key.as_str(),
            "method" | "url" | "headers" | "body" | "timeoutMs"
        ) || is_internal_metadata_key(key)
        {
            continue;
        }
        // Legacy request-level response cap. Response size limits are enforced
        // by runtime config now; accept and ignore the field so old callers
        // keep working.
        if key == "maxResponseBytes" {
            continue;
        }
        return Err(http_error(format!(
            "std.http.request has unknown field {key}"
        )));
    }
    Ok(())
}

fn normalize_method(raw: &str) -> Result<Method> {
    let normalized = raw.trim().to_ascii_uppercase();
    if normalized.trim().is_empty() {
        return Err(http_error(
            "std.http.request.method must be a non-empty string".to_string(),
        ));
    }

    if !SUPPORTED_METHODS
        .iter()
        .any(|method| method == &normalized.as_str())
    {
        return Err(http_error(format!(
            "std.http.request.method must be one of {}",
            SUPPORTED_METHODS.join(", ")
        )));
    }

    Method::from_bytes(normalized.as_bytes())
        .map_err(|error| http_error(format!("std.http.request.method is invalid: {error}")))
}

fn as_str_array_value<'a>(value: &'a Value, field: &'static str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| http_error(format!("std.http.request {field} must be a string")))
}

pub(super) fn parse_timeout_ms(value: Option<&Value>) -> Result<Option<u64>> {
    match value {
        None => Ok(None),
        Some(value) if value.is_null() => Ok(None),
        Some(value) => {
            let value = value
                .as_u64()
                .or_else(|| {
                    value.as_f64().and_then(|value| {
                        if !value.is_finite() || value.fract() != 0.0 || value < 0.0 {
                            return None;
                        }
                        Some(value as u64)
                    })
                })
                .ok_or_else(|| {
                    http_error("std.http.request.timeoutMs must be a positive integer".to_string())
                })?;

            if value == 0 {
                return Err(http_error(
                    "std.http.request.timeoutMs must be greater than zero".to_string(),
                ));
            }

            Ok(Some(value))
        }
    }
}

pub(super) fn parse_headers(value: Option<&Value>) -> Result<Vec<(String, String)>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    let array = value
        .as_array()
        .ok_or_else(|| http_error("std.http.request.headers must be an array".to_string()))?;
    let mut headers = Vec::with_capacity(array.len());

    for item in array {
        let object = item
            .as_object()
            .ok_or_else(|| http_error("std.http.request header must be object".to_string()))?;

        for key in object.keys() {
            if key != "name" && key != "value" && !is_internal_metadata_key(key) {
                return Err(http_error(format!(
                    "std.http.request header has unknown field {key}"
                )));
            }
        }

        let name = as_str_array_value(
            object
                .get("name")
                .ok_or_else(|| http_error("std.http.request header missing name".to_string()))?,
            "header.name",
        )?
        .trim();
        let plain_value = object.get("value").filter(|value| !value.is_null());
        let Some(value) = plain_value else {
            return Err(http_error(
                "std.http.request header must set value".to_string(),
            ));
        };
        let value = as_str_array_value(value, "header.value")?
            .trim()
            .to_string();
        validate_request_header_name(name)?;

        value.parse::<HeaderValue>().map_err(|error| {
            http_error(format!(
                "std.http.request header value is invalid for {name}: {error}"
            ))
        })?;

        headers.push((name.to_string(), value));
    }

    Ok(headers)
}

fn validate_request_header_name(name: &str) -> Result<()> {
    if !name.as_bytes().is_ascii() {
        return Err(http_error(format!(
            "std.http.request header name contains non ASCII characters: {name}"
        )));
    }
    if name.is_empty() {
        return Err(http_error(
            "std.http.request header name must not be empty".to_string(),
        ));
    }
    if name.eq_ignore_ascii_case("proxy-authorization") {
        return Err(http_error(
            "std.http.request headers must not include Proxy-Authorization; runtime proxy authentication is runtime-owned"
                .to_string(),
        ));
    }
    name.parse::<HeaderName>().map_err(|error| {
        http_error(format!(
            "std.http.request header name is invalid for {name}: {error}"
        ))
    })?;
    Ok(())
}

fn parse_body(value: Option<&Value>) -> Result<Vec<u8>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(bytes) = bytes_payload(value) {
        return Ok(bytes);
    }
    if let Some(encoded) = value.as_str() {
        return decode_body_base64(encoded);
    }
    if value
        .as_object()
        .is_some_and(|object| object.contains_key(BYTES_BASE64_KEY))
    {
        return Err(http_error(
            "std.http.request.body __skiffBytesBase64 must be valid base64".to_string(),
        ));
    }
    Err(http_error(
        "std.http.request.body must be bytes (__skiffBytesBase64 object or base64 string)"
            .to_string(),
    ))
}

fn decode_body_base64(encoded: &str) -> Result<Vec<u8>> {
    decode_base64(encoded)
        .map_err(|error| http_error(format!("std.http.request.body base64 is invalid: {error}")))
}

#[cfg(test)]
fn redact_single_header_name(name: &str) -> bool {
    REDACTED_HEADER_NAMES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

#[cfg(test)]
pub(super) fn redact_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                if redact_single_header_name(name) {
                    "<redacted>".to_string()
                } else {
                    value.clone()
                },
            )
        })
        .collect()
}
