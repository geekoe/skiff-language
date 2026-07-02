use serde_json::{json, Value};

use crate::error::RuntimeError;
use crate::host::http_runtime::input::{
    parse_headers, parse_input, parse_timeout_ms, redact_headers, ParsedRequest,
};

use super::helpers::{bytes_body, empty_body};

fn assert_http_error_contains(error: RuntimeError, expected: &str) {
    let payload = error.payload();
    assert_eq!(payload.code, "std.http.HttpError");
    assert!(
        payload.message.contains(expected),
        "expected {expected:?} in {:?}",
        payload.message
    );
}

#[test]
fn parse_input_rejects_unsupported_method() {
    let input = json!({
        "method": "TRACE",
        "url": "https://example.com",
        "headers": [],
        "body": empty_body(),
    });
    assert_http_error_contains(
        parse_input(&input).expect_err("TRACE should fail"),
        "one of",
    );
}

#[test]
fn parse_input_rejects_zero_timeout() {
    let input = json!({
        "method": "GET",
        "url": "https://example.com",
        "headers": [],
        "body": empty_body(),
        "timeoutMs": 0,
    });
    assert_http_error_contains(
        parse_timeout_ms(input.get("timeoutMs")).expect_err("zero timeout should fail"),
        "greater than zero",
    );
}

#[test]
fn parse_timeout_ms_accepts_missing_or_null() {
    assert!(matches!(parse_timeout_ms(None), Ok(None)));
    assert!(matches!(parse_timeout_ms(Some(&Value::Null)), Ok(None)));
}

#[test]
fn redact_headers_redacts_sensitive_names() {
    let source = vec![
        ("Authorization".to_string(), "token".to_string()),
        ("x-api-key".to_string(), "abc".to_string()),
        ("api-key".to_string(), "def".to_string()),
        ("x-other".to_string(), "ok".to_string()),
    ];
    let redacted = redact_headers(&source);
    assert_eq!(redacted[0].1, "<redacted>");
    assert_eq!(redacted[1].1, "<redacted>");
    assert_eq!(redacted[2].1, "<redacted>");
    assert_eq!(redacted[3].1, "ok");
}

#[test]
fn parse_headers_rejects_unknown_fields_and_reserved_legacy_metadata() {
    let headers = json!([
        {"name": "x-api-key", "value": "plain", "extra": "ignored"}
    ]);
    assert_http_error_contains(
        parse_headers(Some(&headers)).expect_err("unknown header field should fail"),
        "unknown field extra",
    );

    let headers = json!([
        {"name": "x-trace", "value": "abc", "__skiffType": "std.http.HttpHeader"}
    ]);
    assert!(parse_headers(Some(&headers))
        .expect_err("__skiffType is reserved legacy metadata")
        .to_string()
        .contains("unknown field __skiffType"));

    let headers = json!([
        {"name": "x-trace", "value": "abc", "$type": "std.http.HttpHeader"}
    ]);
    assert_eq!(
        parse_headers(Some(&headers)).expect("legacy metadata fields should be ignored"),
        vec![("x-trace".to_string(), "abc".to_string())]
    );
}

#[test]
fn parse_headers_rejects_missing_value() {
    let headers = json!([
        {"name": "x-api-key"}
    ]);

    assert_http_error_contains(
        parse_headers(Some(&headers)).expect_err("missing header value should fail"),
        "must set value",
    );
}

#[test]
fn parse_headers_rejects_name_and_value_type_errors() {
    let headers = json!([
        {"name": 42, "value": "plain"}
    ]);
    assert_http_error_contains(
        parse_headers(Some(&headers)).expect_err("header name type should fail"),
        "header.name must be a string",
    );

    let headers = json!([
        {"name": "x-api-key", "value": {"raw": "key"}}
    ]);
    assert_http_error_contains(
        parse_headers(Some(&headers)).expect_err("header value type should fail"),
        "header.value must be a string",
    );
}

#[test]
fn parse_headers_rejects_non_metadata_internal_payload_fields() {
    let headers = json!([
        {"name": "x-api-key", "value": "plain", "__skiffBytesBase64": "SGVsbG8="}
    ]);

    assert_http_error_contains(
        parse_headers(Some(&headers)).expect_err("internal payload field should fail"),
        "unknown field __skiffBytesBase64",
    );
}

#[test]
fn parse_input_accepts_bytes_body_wire_values() {
    let object_input = json!({
        "method": "POST",
        "url": "https://example.com",
        "headers": [],
        "body": bytes_body(b"hello"),
    });
    assert!(matches!(
        parse_input(&object_input),
        Ok(ParsedRequest { body, .. }) if body == b"hello".to_vec()
    ));

    let string_input = json!({
        "method": "POST",
        "url": "https://example.com",
        "headers": [],
        "body": "aGVsbG8=",
    });
    assert!(matches!(
        parse_input(&string_input),
        Ok(ParsedRequest { body, .. }) if body == b"hello".to_vec()
    ));
}

#[test]
fn parse_input_accepts_missing_or_null_body_as_empty() {
    let missing_body = json!({
        "method": "GET",
        "url": "https://example.com",
        "headers": [],
    });
    assert!(matches!(
        parse_input(&missing_body),
        Ok(ParsedRequest { body, .. }) if body.is_empty()
    ));

    let null_body = json!({
        "method": "GET",
        "url": "https://example.com",
        "headers": [],
        "body": null,
    });
    assert!(matches!(
        parse_input(&null_body),
        Ok(ParsedRequest { body, .. }) if body.is_empty()
    ));
}

#[test]
fn parse_input_rejects_legacy_tagged_body() {
    let input = json!({
        "method": "POST",
        "url": "https://example.com",
        "headers": [],
        "body": {"tag": "empty"},
    });

    assert_http_error_contains(
        parse_input(&input).expect_err("legacy tagged body should fail"),
        "body must be bytes",
    );
}
