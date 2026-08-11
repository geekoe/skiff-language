pub(crate) use super::*;
use std::collections::BTreeSet;

use serde_json::json;
use skiff_artifact_model::STD_NATIVE_SIGNATURES;
use skiff_runtime_capability_context::NativeCapabilityContexts;

use crate::{
    error::RuntimeError,
    runtime_value_facade::{RequestHeap, RuntimeValue},
};

use super::{
    http::{
        ensure_http_helper_none_capability_context, http_status_arg, HTTP_REQUEST_HEADER_KEY,
        HTTP_STREAM_CHUNK_KEY, HTTP_STREAM_END_KEY, HTTP_STREAM_START_KEY,
    },
    http_helpers::{cookie_value, forwardable_headers, name_values, sse_headers, NameMatch},
    json::{json_codec_decode_error, JsonNativeDispatch},
    runtime_shared_native_route,
    time::{clamp_sleep_millis, sleep_millis_from_runtime_value, TIME_SLEEP_MAX_MILLIS},
    RuntimeNativeInvocation, RuntimeNativeRoute,
};
use skiff_runtime_boundary::json::decode_untyped_wire_json;
use skiff_runtime_model::{
    request_heap::RequestHeap as ModelRequestHeap,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};
use skiff_runtime_native_contract::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

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
        RuntimeNativeRoute::TaskControl,
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
fn std_json_field_access_reads_shared_values_without_serialization() {
    let mut heap = ModelRequestHeap::default();
    let chunk = decode_untyped_wire_json(
        &json!({
            "id": "chatcmpl-1",
            "model": 42,
            "ok": true,
            "enabled": "not-a-bool",
            "choices": [{ "index": 0 }, "nope"],
            "meta": { "depth": 1 },
        }),
        &mut heap,
    )
    .expect("chunk should decode");

    let json_plan = RuntimeTypePlan::json_value_plan();
    let string_plan = RuntimeTypePlan::new("string", None, RuntimeTypeNode::String);
    let nullable_json = RuntimeTypePlan::synthetic_nullable(json_plan.clone());
    let nullable_string = RuntimeTypePlan::synthetic_nullable(string_plan.clone());
    let nullable_number = RuntimeTypePlan::synthetic_nullable(RuntimeTypePlan::new(
        "number",
        None,
        RuntimeTypeNode::Number,
    ));
    let nullable_bool = RuntimeTypePlan::synthetic_nullable(RuntimeTypePlan::new(
        "bool",
        None,
        RuntimeTypeNode::Bool,
    ));
    let nullable_array =
        RuntimeTypePlan::synthetic_nullable(RuntimeTypePlan::synthetic_array(json_plan.clone()));

    fn invocation(
        binding_key: &'static str,
        arg_plans: Vec<RuntimeTypePlan>,
        return_plan: RuntimeTypePlan,
    ) -> RuntimeNativeInvocation {
        RuntimeNativeInvocation::new(
            binding_key.to_string(),
            binding_key,
            Some(NativeCallPlan::new(
                NativeBindingKey::from_static(binding_key),
                arg_plans,
                return_plan,
                NativeRequiredContext::None,
            )),
            None,
            None,
        )
    }

    let mut dispatch =
        |key: &'static str, field: &str, plans: Vec<RuntimeTypePlan>, ret: RuntimeTypePlan| {
            JsonNativeDispatch::dispatch(
                &invocation(key, plans, ret),
                key,
                vec![chunk.clone(), RuntimeValue::String(field.to_string())],
                &mut heap,
            )
        };

    assert_eq!(
        dispatch(
            "std.json.get",
            "id",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_json.clone(),
        )
        .expect("get should read the string field"),
        RuntimeValue::String("chatcmpl-1".to_string())
    );
    assert_eq!(
        dispatch(
            "std.json.get",
            "missing",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_json.clone(),
        )
        .expect("get should yield null for a missing field"),
        RuntimeValue::Null
    );
    assert_eq!(
        dispatch(
            "std.json.getString",
            "id",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_string.clone(),
        )
        .expect("getString should read the string field"),
        RuntimeValue::String("chatcmpl-1".to_string())
    );
    assert_eq!(
        dispatch(
            "std.json.getString",
            "model",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_string.clone(),
        )
        .expect("getString should yield null for a non-string field"),
        RuntimeValue::Null
    );
    assert_eq!(
        dispatch(
            "std.json.getNumber",
            "model",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_number.clone(),
        )
        .expect("getNumber should read the number field"),
        RuntimeValue::Number(42.0)
    );
    assert_eq!(
        dispatch(
            "std.json.getNumber",
            "id",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_number.clone(),
        )
        .expect("getNumber should yield null for a non-number field"),
        RuntimeValue::Null
    );
    assert_eq!(
        dispatch(
            "std.json.getBool",
            "ok",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_bool.clone(),
        )
        .expect("getBool should read the bool field"),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        dispatch(
            "std.json.getBool",
            "enabled",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_bool.clone(),
        )
        .expect("getBool should yield null for a non-bool field"),
        RuntimeValue::Null
    );
    let array_field = dispatch(
        "std.json.getArray",
        "choices",
        vec![json_plan.clone(), string_plan.clone()],
        nullable_array.clone(),
    )
    .expect("getArray should read the array field");
    let RuntimeValue::Heap(array_handle) = array_field else {
        panic!("getArray should return a heap array");
    };
    let skiff_runtime_model::runtime_value::HeapNode::Array(items) = heap
        .get(array_handle)
        .expect("array field handle should resolve")
    else {
        panic!("field should be a heap array");
    };
    assert_eq!(items.len(), 2);

    let error = JsonNativeDispatch::dispatch(
        &invocation(
            "std.json.getString",
            vec![json_plan.clone(), string_plan.clone()],
            nullable_string.clone(),
        ),
        "std.json.getString",
        vec![chunk.clone()],
        &mut heap,
    )
    .expect_err("getters should require the declared argument count");
    assert!(
        error.to_string().contains("expects 2 argument(s)"),
        "unexpected arity diagnostic: {error}"
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
fn plan_free_json_encode_is_dynamic_but_decode_remains_strict() {
    let mut heap = RequestHeap::default();
    let encode = RuntimeNativeInvocation::new(
        "std.json.encode".to_string(),
        "std.json.encode",
        None,
        None,
        None,
    );
    assert_eq!(
        JsonNativeDispatch::dispatch(
            &encode,
            "std.json.encode",
            vec![RuntimeValue::String("deepseek".to_string())],
            &mut heap,
        )
        .expect("plan-free JSON encode should use its admitted dynamic codec"),
        RuntimeValue::String("\"deepseek\"".to_string())
    );

    let decode = RuntimeNativeInvocation::new(
        "std.json.decode".to_string(),
        "std.json.decode",
        None,
        None,
        None,
    );
    let error = JsonNativeDispatch::dispatch(
        &decode,
        "std.json.decode",
        vec![RuntimeValue::String("\"deepseek\"".to_string())],
        &mut heap,
    )
    .expect_err("JSON decode cannot infer its return type without a plan");
    assert!(
        error
            .to_string()
            .contains("unsupported native target std.json.decode"),
        "unexpected strict decode diagnostic: {error}"
    );
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

#[test]
fn http_stream_event_constructors_require_none_capability_context() {
    let no_capability = NativeCapabilityContexts::<(), (), (), (), (), (), (), ()>::None;
    let response_stream_capability =
        NativeCapabilityContexts::<(), (), (), (), (), (), (), ()>::HttpResponseStream(());

    for binding_key in [
        HTTP_STREAM_START_KEY,
        HTTP_STREAM_CHUNK_KEY,
        HTTP_STREAM_END_KEY,
    ] {
        ensure_http_helper_none_capability_context(binding_key, &no_capability)
            .unwrap_or_else(|error| panic!("{binding_key} should accept None context: {error}"));
        let error =
            ensure_http_helper_none_capability_context(binding_key, &response_stream_capability)
                .expect_err("constructor should reject response-stream capability context");
        let message = error.to_string();
        assert!(
            message.contains(binding_key)
                && message.contains("HttpResponseStream")
                && message.contains("None"),
            "unexpected error: {message}"
        );
    }
}

#[test]
fn http_stream_start_status_accepts_only_integer_100_through_599() {
    for (value, expected) in [(100.0, 100), (200.0, 200), (599.0, 599)] {
        assert_eq!(
            http_status_arg(Some(&RuntimeValue::Number(value)), HTTP_STREAM_START_KEY)
                .expect("status should be valid"),
            expected
        );
    }

    for value in [99.0, 600.0, 200.5, f64::NAN] {
        let error = http_status_arg(Some(&RuntimeValue::Number(value)), HTTP_STREAM_START_KEY)
            .expect_err("invalid status must fail");
        assert!(
            error.to_string().contains("integer between 100 and 599"),
            "unexpected error: {error}"
        );
    }

    let missing =
        http_status_arg(None, HTTP_STREAM_START_KEY).expect_err("missing status must fail");
    assert!(missing.to_string().contains("requires status"));
    let wrong = http_status_arg(
        Some(&RuntimeValue::String("200".to_string())),
        HTTP_STREAM_START_KEY,
    )
    .expect_err("non-number status must fail");
    assert!(wrong.to_string().contains("status must be an integer"));
}
mod prepared;
