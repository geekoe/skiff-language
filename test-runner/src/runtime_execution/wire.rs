use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{Map, Value};
use skiff_runtime_model::service_error::{OpaqueServiceError, ServiceErrorEnvelope};

use crate::canonical_fixture::CanonicalFixtureError;

const RUNTIME_FRAME_SCHEMA_VERSION: &str = "skiff-runtime-frame-v4";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TestDispatchOutcome {
    Passed,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControlErrorResponse {
    pub(super) code: String,
    pub(super) message: String,
}

/// The router's release pointer table projection carried by `/__router/health`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveAssemblyProjection {
    pub(super) profile: String,
    pub(super) release_count: u64,
    pub(super) build_ids: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HealthSnapshot {
    pub(super) active: ActiveAssemblyProjection,
}

pub(super) fn decode_test_dispatch_response(
    body: &str,
) -> Result<TestDispatchOutcome, CanonicalFixtureError> {
    decode_test_dispatch_response_inner(body)
        .map_err(|message| wire_error("runtime test dispatch response", message))
}

pub(super) fn decode_control_error_response(
    body: &str,
) -> Result<ControlErrorResponse, CanonicalFixtureError> {
    decode_control_error_response_inner(body)
        .map_err(|message| wire_error("control error response", message))
}

pub(super) fn decode_health_snapshot(body: &str) -> Result<HealthSnapshot, CanonicalFixtureError> {
    decode_health_snapshot_inner(body)
        .map_err(|message| wire_error("router health response", message))
}

fn decode_test_dispatch_response_inner(body: &str) -> Result<TestDispatchOutcome, String> {
    let value = decode_json(body, "runtime test dispatch response")?;
    let root = exact_object(
        &value,
        &["ok", "header", "payloadBase64"],
        &[],
        "runtime test dispatch response",
    )?;
    require_true(root, "ok", "runtime test dispatch response")?;
    let header_value = field(root, "header", "runtime test dispatch response")?;
    let header = header_value
        .as_object()
        .ok_or_else(|| "runtime test dispatch response.header must be an object".to_string())?;
    match string_field(header, "type", "runtime test dispatch response.header")? {
        "response.end" => decode_test_success(root, header_value),
        "response.error" => decode_test_failure(root, header_value),
        _ => Err(
            "runtime test dispatch response.header.type must be response.end or response.error"
                .to_string(),
        ),
    }
}

fn decode_test_success(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = exact_object(
        header_value,
        &[
            "schemaVersion",
            "type",
            "requestId",
            "payloadPresent",
            "httpResponse",
        ],
        &[],
        "runtime test dispatch response.header",
    )?;
    if string_field(
        header,
        "schemaVersion",
        "runtime test dispatch response.header",
    )? != RUNTIME_FRAME_SCHEMA_VERSION
    {
        return Err(format!(
            "header.schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        ));
    }
    if string_field(header, "type", "runtime test dispatch response.header")? != "response.end" {
        return Err("header.type must be response.end".to_string());
    }
    let request_id = string_field(header, "requestId", "runtime test dispatch response.header")?;
    if request_id.is_empty()
        || request_id.trim() != request_id
        || request_id
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err("header.requestId must be a non-empty canonical token".to_string());
    }
    if !bool_field(
        header,
        "payloadPresent",
        "runtime test dispatch response.header",
    )? {
        return Err("header.payloadPresent must be true for the null payload".to_string());
    }
    validate_dispatch_http_response(header)?;
    if string_field(root, "payloadBase64", "runtime test dispatch response")? != "bnVsbA==" {
        return Err(
            "payloadBase64 must be the canonical Base64 encoding of exact null".to_string(),
        );
    }
    Ok(TestDispatchOutcome::Passed)
}

fn decode_test_failure(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = header_value
        .as_object()
        .ok_or_else(|| "runtime test dispatch response.header must be an object".to_string())?;
    let error_kind = string_field(header, "errorKind", "runtime test dispatch response.header")?;
    match error_kind {
        "control" => decode_control_dispatch_failure(root, header_value),
        "fixedService" => decode_fixed_dispatch_failure(root, header_value),
        _ => Err(
            "runtime test dispatch response.header.errorKind must be control or fixedService"
                .to_string(),
        ),
    }
}

fn decode_control_dispatch_failure(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = exact_object(
        header_value,
        &["schemaVersion", "type", "requestId", "errorKind", "error"],
        &[],
        "runtime test dispatch response.header",
    )?;
    validate_error_header_prefix(header)?;
    let error = exact_object(
        field(header, "error", "runtime test dispatch response.header")?,
        &["code", "message"],
        &["status", "details"],
        "runtime test dispatch response.header.error",
    )?;
    let code =
        canonical_non_empty_string(error, "code", "runtime test dispatch response.header.error")?;
    let message = canonical_non_empty_string(
        error,
        "message",
        "runtime test dispatch response.header.error",
    )?;
    validate_optional_error_status(error, "runtime test dispatch response.header.error")?;
    if string_field(root, "payloadBase64", "runtime test dispatch response")? != "" {
        return Err("control response.error payloadBase64 must be empty".to_string());
    }
    Ok(TestDispatchOutcome::Failed(format!("{code}: {message}")))
}

fn decode_fixed_dispatch_failure(
    root: &Map<String, Value>,
    header_value: &Value,
) -> Result<TestDispatchOutcome, String> {
    let header = exact_object(
        header_value,
        &["schemaVersion", "type", "requestId", "errorKind"],
        &[],
        "runtime test dispatch response.header",
    )?;
    validate_error_header_prefix(header)?;
    let encoded = string_field(root, "payloadBase64", "runtime test dispatch response")?;
    if encoded.is_empty() {
        return Err("fixedService response.error payloadBase64 must be non-empty".to_string());
    }
    let bytes = BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("fixedService payloadBase64 is invalid: {error}"))?;
    if BASE64_STANDARD.encode(&bytes) != encoded {
        return Err("fixedService payloadBase64 must be canonical".to_string());
    }
    let error = OpaqueServiceError::decode(bytes)
        .map_err(|error| format!("fixedService payload is invalid: {error}"))?;
    let message = match error.envelope() {
        ServiceErrorEnvelope::PublicTypedError {
            package_id,
            stable_schema_key,
            ..
        } => format!("fixed service error {package_id}::{stable_schema_key}"),
        ServiceErrorEnvelope::InternalError { payload } => payload.message.clone(),
        ServiceErrorEnvelope::PlatformError {
            builtin_error_identity,
            ..
        } => format!("fixed service error {}", builtin_error_identity.symbol()),
    };
    Ok(TestDispatchOutcome::Failed(message))
}

fn validate_error_header_prefix(header: &Map<String, Value>) -> Result<(), String> {
    if string_field(
        header,
        "schemaVersion",
        "runtime test dispatch response.header",
    )? != RUNTIME_FRAME_SCHEMA_VERSION
    {
        return Err(format!(
            "header.schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        ));
    }
    if string_field(header, "type", "runtime test dispatch response.header")? != "response.error" {
        return Err("header.type must be response.error".to_string());
    }
    canonical_non_empty_string(header, "requestId", "runtime test dispatch response.header")?;
    Ok(())
}

fn decode_control_error_response_inner(body: &str) -> Result<ControlErrorResponse, String> {
    let value = decode_json(body, "control error response")?;
    let root = exact_object(&value, &["error"], &[], "control error response")?;
    let error = exact_object(
        field(root, "error", "control error response")?,
        &["code", "message"],
        &["details"],
        "control error response.error",
    )?;
    Ok(ControlErrorResponse {
        code: canonical_non_empty_string(error, "code", "control error response.error")?
            .to_string(),
        message: canonical_non_empty_string(error, "message", "control error response.error")?
            .to_string(),
    })
}

fn canonical_non_empty_string<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a str, String> {
    let value = string_field(object, name, context)?;
    if value.is_empty() || value.trim() != value {
        return Err(format!(
            "{context}.{name} must be a non-empty canonical string"
        ));
    }
    Ok(value)
}

fn validate_optional_error_status(error: &Map<String, Value>, context: &str) -> Result<(), String> {
    if let Some(status) = error.get("status") {
        let status = status
            .as_u64()
            .filter(|status| (400..=599).contains(status))
            .ok_or_else(|| format!("{context}.status must be an integer from 400 through 599"))?;
        u16::try_from(status)
            .map_err(|_| format!("{context}.status must fit the HTTP status range"))?;
    }
    Ok(())
}

fn validate_dispatch_http_response(header: &Map<String, Value>) -> Result<(), String> {
    let http_response = exact_object(
        field(
            header,
            "httpResponse",
            "runtime test dispatch response.header",
        )?,
        &["status", "headers"],
        &[],
        "runtime test dispatch response.header.httpResponse",
    )?;
    if u64_field(
        http_response,
        "status",
        "runtime test dispatch response.header.httpResponse",
    )? != 200
    {
        return Err("inner HTTP response status must be 200".to_string());
    }
    let headers = array(
        field(
            http_response,
            "headers",
            "runtime test dispatch response.header.httpResponse",
        )?,
        "runtime test dispatch response.header.httpResponse.headers",
    )?;
    let [content_type] = headers else {
        return Err(
            "inner HTTP response must have exactly one canonical content-type header".to_string(),
        );
    };
    let content_type = exact_object(
        content_type,
        &["name", "value"],
        &[],
        "runtime test dispatch response.header.httpResponse.headers[0]",
    )?;
    if string_field(
        content_type,
        "name",
        "runtime test dispatch response.header.httpResponse.headers[0]",
    )? != "content-type"
        || string_field(
            content_type,
            "value",
            "runtime test dispatch response.header.httpResponse.headers[0]",
        )? != "application/json; charset=utf-8"
    {
        return Err(
            "inner HTTP response content-type must be application/json; charset=utf-8".to_string(),
        );
    }
    Ok(())
}

fn decode_health_snapshot_inner(body: &str) -> Result<HealthSnapshot, String> {
    let value = decode_json(body, "router health")?;
    let root = value
        .as_object()
        .ok_or_else(|| "router health must be an object".to_string())?;
    require_true(root, "ok", "router health")?;
    let active = decode_active_projection(field(root, "activeAssembly", "router health")?)?;
    Ok(HealthSnapshot { active })
}

/// Decodes the router's release pointer table projection. The remaining
/// health body (capability connections, replica views, counters) is owned by
/// the router surface and carries no test-runner contract; unknown or
/// mutated fields there must not fail the readiness gate.
fn decode_active_projection(value: &Value) -> Result<ActiveAssemblyProjection, String> {
    let context = "router health activeAssembly";
    let active = exact_object(
        value,
        &["profile", "releaseCount", "buildIds"],
        &[],
        context,
    )?;
    let build_ids = array(
        field(active, "buildIds", context)?,
        &format!("{context}.buildIds"),
    )?
    .iter()
    .enumerate()
    .map(|(index, value)| {
        let build_id = value
            .as_str()
            .ok_or_else(|| format!("{context}.buildIds[{index}] must be a string"))?;
        if build_id.is_empty()
            || build_id.trim() != build_id
            || build_id
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(format!(
                "{context}.buildIds[{index}] must be a canonical token"
            ));
        }
        Ok(build_id.to_string())
    })
    .collect::<Result<Vec<_>, String>>()?;
    Ok(ActiveAssemblyProjection {
        profile: string_field(active, "profile", context)?.to_string(),
        release_count: u64_field(active, "releaseCount", context)?,
        build_ids: build_ids.into_iter().collect(),
    })
}

fn decode_json(body: &str, context: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|error| format!("{context} is not valid JSON: {error}"))
}

fn exact_object<'a>(
    value: &'a Value,
    required: &[&str],
    optional: &[&str],
    context: &str,
) -> Result<&'a Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))?;
    for name in required {
        if !object.contains_key(*name) {
            return Err(format!("{context} is missing {name}"));
        }
    }
    if let Some(name) = object
        .keys()
        .find(|name| !required.contains(&name.as_str()) && !optional.contains(&name.as_str()))
    {
        return Err(format!("{context} has unexpected field {name}"));
    }
    Ok(object)
}

fn field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a Value, String> {
    object
        .get(name)
        .ok_or_else(|| format!("{context} is missing {name}"))
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    context: &str,
) -> Result<&'a str, String> {
    field(object, name, context)?
        .as_str()
        .ok_or_else(|| format!("{context}.{name} must be a string"))
}

fn bool_field(object: &Map<String, Value>, name: &str, context: &str) -> Result<bool, String> {
    field(object, name, context)?
        .as_bool()
        .ok_or_else(|| format!("{context}.{name} must be a boolean"))
}

fn u64_field(object: &Map<String, Value>, name: &str, context: &str) -> Result<u64, String> {
    field(object, name, context)?
        .as_u64()
        .ok_or_else(|| format!("{context}.{name} must be a canonical unsigned integer"))
}

fn array<'a>(value: &'a Value, context: &str) -> Result<&'a [Value], String> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{context} must be an array"))
}

fn require_true(object: &Map<String, Value>, name: &str, context: &str) -> Result<(), String> {
    if bool_field(object, name, context)? {
        Ok(())
    } else {
        Err(format!("{context}.{name} must be true"))
    }
}

fn wire_error(context: impl Into<String>, message: impl Into<String>) -> CanonicalFixtureError {
    CanonicalFixtureError::Wire {
        context: context.into(),
        message: message.into(),
    }
}

#[cfg(test)]
#[path = "tests/wire.rs"]
mod tests;
