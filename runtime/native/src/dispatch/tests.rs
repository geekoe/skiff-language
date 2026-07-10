use std::collections::BTreeSet;

use serde_json::json;
use skiff_artifact_model::STD_NATIVE_SIGNATURES;
use skiff_runtime_capability_context::NativeCapabilityContexts;

use crate::{error::RuntimeError, runtime_value_facade::RuntimeValue};

use super::{
    http::{ensure_http_helper_none_capability_context, HTTP_REQUEST_HEADER_KEY},
    http_helpers::{cookie_value, forwardable_headers, name_values, sse_headers, NameMatch},
    json::json_codec_decode_error,
    runtime_shared_native_route,
    time::{clamp_sleep_millis, sleep_millis_from_runtime_value, TIME_SLEEP_MAX_MILLIS},
    RuntimeNativeRoute,
};

#[test]
fn native_signature_registry_shared_targets_are_runtime_reachable() {
    let mut missing = Vec::new();
    let mut routed = BTreeSet::new();

    for signature in STD_NATIVE_SIGNATURES {
        match runtime_shared_native_route(signature.binding_key) {
            Some(route) => {
                routed.insert(route);
            }
            None => missing.push(format!("{} ({})", signature.binding_key, signature.target)),
        }
    }

    assert!(
        missing.is_empty(),
        "STD_NATIVE_SIGNATURES names must be reachable by runtime native routing; missing: {}",
        missing.join(", ")
    );

    let expected_routes = BTreeSet::from([
        RuntimeNativeRoute::Actor,
        RuntimeNativeRoute::Bytes,
        RuntimeNativeRoute::File,
        RuntimeNativeRoute::Json,
        RuntimeNativeRoute::Time,
        RuntimeNativeRoute::Http,
        RuntimeNativeRoute::Websocket,
        RuntimeNativeRoute::Telemetry,
        RuntimeNativeRoute::Resource,
        RuntimeNativeRoute::NativeRegistry,
        RuntimeNativeRoute::ReceiverMethod,
    ]);
    assert_eq!(
        routed, expected_routes,
        "shared native signatures should cover every runtime shared native route"
    );
}

#[test]
fn std_time_sleep_millis_are_clamped() {
    assert_eq!(clamp_sleep_millis(-1.0), 0);
    assert_eq!(clamp_sleep_millis(0.0), 0);
    assert_eq!(clamp_sleep_millis(42.0), 42);
    assert_eq!(
        clamp_sleep_millis((TIME_SLEEP_MAX_MILLIS + 1) as f64),
        TIME_SLEEP_MAX_MILLIS
    );
}

#[test]
fn std_time_sleep_requires_safe_integer_milliseconds() {
    assert!(sleep_millis_from_runtime_value(&RuntimeValue::Number(42.0)).is_ok());

    let error = sleep_millis_from_runtime_value(&RuntimeValue::Number(9_007_199_254_740_992.0))
        .expect_err("unsafe integer payload should fail");
    assert!(
        error.to_string().contains("safe integer"),
        "unexpected error: {error}"
    );
}

#[test]
fn std_json_codec_decode_errors_use_public_decode_error_payload() {
    for expected_target in ["std.json.decode", "std.json.encode"] {
        let error = json_codec_decode_error(
            expected_target,
            RuntimeError::Decode("schema mismatch".to_string()),
        );

        assert!(
            matches!(
                error,
                RuntimeError::DecodeTarget {
                    ref target,
                    ref message,
                } if target == expected_target && message == "schema mismatch"
            ),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn http_request_helpers_read_headers_query_and_cookies() {
    let request = json!({
        "headers": [
            { "name": "X-Trace", "value": "a" },
            { "name": "x-trace", "value": "b" },
            { "name": "Cookie", "value": "sid=abc; theme = dark" }
        ],
        "query": [
            { "name": "q", "value": "first" },
            { "name": "Q", "value": "different" }
        ]
    });

    assert_eq!(
        name_values(
            &request,
            "headers",
            "x-trace",
            NameMatch::AsciiCaseInsensitive
        ),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(
        name_values(&request, "query", "q", NameMatch::Exact),
        vec!["first".to_string()]
    );
    assert_eq!(
        cookie_value(&["sid=abc; theme = dark".to_string()], "theme"),
        Some("dark".to_string())
    );
}

#[test]
fn http_forwardable_headers_drop_hop_by_hop_headers_and_connection_tokens() {
    let headers = vec![
        json!({ "name": "content-type", "value": "text/plain" }),
        json!({ "name": "connection", "value": "x-internal, Upgrade" }),
        json!({ "name": "x-internal", "value": "drop" }),
        json!({ "name": "upgrade", "value": "websocket" }),
        json!({ "name": "x-keep", "value": "yes" }),
    ];

    assert_eq!(
        forwardable_headers(&headers),
        json!([
            { "name": "content-type", "value": "text/plain" },
            { "name": "x-keep", "value": "yes" }
        ])
    );
}

#[test]
fn http_sse_headers_include_event_stream_defaults() {
    assert_eq!(
        sse_headers(),
        json!([
            { "name": "content-type", "value": "text/event-stream; charset=utf-8" },
            { "name": "cache-control", "value": "no-cache" },
            { "name": "connection", "value": "keep-alive" }
        ])
    );
}

#[test]
fn http_helper_none_capability_assertion_rejects_other_capabilities() {
    let no_capability = NativeCapabilityContexts::<(), (), (), (), (), (), (), ()>::None;
    assert!(
        ensure_http_helper_none_capability_context(HTTP_REQUEST_HEADER_KEY, &no_capability,)
            .is_ok()
    );

    let http_client_capability =
        NativeCapabilityContexts::<(), (), (), (), (), (), (), ()>::HttpClient(());
    let error = ensure_http_helper_none_capability_context(
        HTTP_REQUEST_HEADER_KEY,
        &http_client_capability,
    )
    .expect_err("HTTP request helper should reject non-None native capability context");
    let message = error.to_string();
    assert!(
        message.contains(HTTP_REQUEST_HEADER_KEY)
            && message.contains("HttpClient")
            && message.contains("None"),
        "unexpected error: {message}"
    );
}
