use serde_json::json;

use crate::{
    error::RuntimeError,
    json::RuntimeBoundaryCodec,
    plan::BoundaryUse,
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueKey},
    type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt},
    value::encode_base64,
};

use super::{
    super::{from_wire, to_wire},
    helpers::{
        alias, array, generic, map, named, record, representation, union,
        websocket_connection_message_descriptor,
    },
};

#[test]
fn wire_roundtrip_supports_scalars_bytes_arrays_maps_and_representation_keys() {
    let expected = map(
        representation("UserId", named("string")),
        array(named("bytes")),
    );
    let encoded = encode_base64(b"hello");
    let input = json!({
        "u1": [{ "__skiffBytesBase64": encoded }],
        "u2": [{ "__skiffBytesBase64": encode_base64(b"bye") }]
    });
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &expected, &mut heap).expect("from_wire should decode");
    let RuntimeValue::Heap(handle) = value else {
        panic!("expected heap map");
    };
    let HeapNode::Map(map) = heap.get(handle).expect("map should resolve") else {
        panic!("expected map");
    };
    assert_eq!(
        map.keys().cloned().collect::<Vec<_>>(),
        vec![RuntimeValueKey::string("u1"), RuntimeValueKey::string("u2"),]
    );

    let output =
        to_wire(&RuntimeValue::Heap(handle), &expected, &mut heap).expect("to_wire should encode");
    assert_eq!(output, input);
    assert!(heap.stats().materialize_depth > 0);
    assert!(heap.stats().materialize_output_bytes > 0);
}

#[test]
fn wire_roundtrip_supports_named_representation_map_keys() {
    let expected = map(named("UserId"), named("string"));
    let input = json!({ "u1": "Ada" });
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &expected, &mut heap).expect("from_wire should decode");
    let RuntimeValue::Heap(handle) = value else {
        panic!("expected heap map");
    };
    let HeapNode::Map(map) = heap.get(handle).expect("map should resolve") else {
        panic!("expected map");
    };
    assert_eq!(
        map.get(&RuntimeValueKey::string("u1")),
        Some(&RuntimeValue::String("Ada".to_string()))
    );

    let output =
        to_wire(&RuntimeValue::Heap(handle), &expected, &mut heap).expect("to_wire should encode");
    assert_eq!(output, input);
}

#[test]
fn wire_roundtrip_supports_date_boundaries() {
    let expected = record("Event", vec![("at", named("Date"))]);
    let input = json!({ "at": "2026-06-04T23:12:03.456+08:00" });
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &expected, &mut heap).expect("Date should decode");
    let RuntimeValue::Heap(handle) = value else {
        panic!("expected heap object");
    };
    let HeapNode::Object(object) = heap.get(handle).expect("object should resolve") else {
        panic!("expected object");
    };
    assert!(matches!(
        object.fields().get("at"),
        Some(RuntimeValue::Date(_))
    ));

    let output =
        to_wire(&RuntimeValue::Heap(handle), &expected, &mut heap).expect("Date should encode");
    assert_eq!(output, json!({ "at": "2026-06-04T15:12:03.456Z" }));

    let error = from_wire(
        &json!({ "at": "2016-12-31T23:59:60Z" }),
        &expected,
        &mut RequestHeap::default(),
    )
    .expect_err("leap seconds should be rejected");
    assert!(
        error.to_string().contains("leap seconds"),
        "unexpected error: {error}"
    );
}

#[test]
fn std_http_client_stream_handle_preserves_body_stream_handle() {
    let expected = RuntimeTypePlan::from_descriptor(&named("std.http.HttpClientStreamHandle"))
        .expect("std handle plan should build");
    let input = json!({
        "status": 200,
        "headers": [],
        "body": { "__skiffStreamId": "stream-1" },
    });
    let mut heap = RequestHeap::default();

    let codec = RuntimeBoundaryCodec::new(&expected, BoundaryUse::NativeReturn, "std handle test");
    let value = codec
        .from_wire_json_internal_handle(&input, &mut heap)
        .expect("std runtime-owned stream handle record should decode");
    let output = codec
        .to_wire_json_internal_handle(&value, &mut heap)
        .expect("std runtime-owned stream handle should encode");

    assert_eq!(output, input);
}

#[test]
fn default_typed_boundary_rejects_std_handle_record_stream_field() {
    let expected = named("std.http.HttpClientStreamHandle");
    let input = json!({
        "status": 200,
        "headers": [],
        "body": { "__skiffStreamId": "stream-1" },
    });
    let mut heap = RequestHeap::default();

    let error = from_wire(&input, &expected, &mut heap)
        .expect_err("default typed boundaries must not decode internal stream handle fields");

    assert!(
        error
            .to_string()
            .contains("Stream handles are only allowed"),
        "unexpected error: {error}"
    );
}

#[test]
fn typed_json_use_case_rejects_internal_handle_materialization_entry() {
    let expected = RuntimeTypePlan::from_descriptor(&named("std.http.HttpClientStreamHandle"))
        .expect("std handle plan should build");
    let input = json!({
        "status": 200,
        "headers": [],
        "body": { "__skiffStreamId": "stream-1" },
    });
    let mut heap = RequestHeap::default();

    let error = RuntimeBoundaryCodec::new(&expected, BoundaryUse::TypedJson, "typed json")
        .from_wire_json_internal_handle(&input, &mut heap)
        .expect_err("TypedJson must not materialize internal handles");

    assert!(
        error
            .to_string()
            .contains("boundary use does not allow runtime-owned internal handles"),
        "unexpected error: {error}"
    );
}

#[test]
fn internal_handle_boundary_still_rejects_ordinary_record_stream_field() {
    let expected = RuntimeTypePlan::from_descriptor(&record(
        "UserEnvelope",
        vec![("body", generic("Stream", vec![named("bytes")]))],
    ))
    .expect("ordinary record plan should build");
    let input = json!({
        "body": { "__skiffStreamId": "stream-1" },
    });
    let mut heap = RequestHeap::default();

    let error = RuntimeBoundaryCodec::new(&expected, BoundaryUse::NativeReturn, "record test")
        .from_wire_json_internal_handle(&input, &mut heap)
        .expect_err("ordinary records must not decode stream handle fields");

    assert!(
        error
            .to_string()
            .contains("Stream handles are only allowed"),
        "unexpected error: {error}"
    );
}

#[test]
fn ordinary_record_stream_field_is_not_a_request_local_handle_boundary() {
    let expected = record(
        "UserEnvelope",
        vec![("body", generic("Stream", vec![named("bytes")]))],
    );
    let input = json!({
        "body": { "__skiffStreamId": "stream-1" },
    });
    let mut heap = RequestHeap::default();

    let error = from_wire(&input, &expected, &mut heap)
        .expect_err("ordinary records must not decode stream handle fields");

    assert!(
        error
            .to_string()
            .contains("Stream handles are only allowed"),
        "unexpected error: {error}"
    );
}

#[test]
fn websocket_connection_message_descriptor_decodes_and_encodes_text_message() {
    let expected = websocket_connection_message_descriptor();
    let input = json!({ "tag": "text", "text": "{\"type\":\"hello\"}" });
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &expected, &mut heap).expect("text message should decode");
    let RuntimeValue::Heap(handle) = value else {
        panic!("expected heap value");
    };
    let HeapNode::Object(object) = heap.get(handle).expect("message should resolve") else {
        panic!("expected object payload");
    };
    assert_eq!(
        object.fields().get("tag"),
        Some(&RuntimeValue::String("text".to_string()))
    );

    let output =
        to_wire(&RuntimeValue::Heap(handle), &expected, &mut heap).expect("message should encode");
    assert_eq!(output, input);
}

#[test]
fn websocket_connect_result_builtin_descriptors_decode_and_encode() {
    let context = record(
        "api.example.ConnectionContext",
        vec![("userId", named("string"))],
    );
    let cases = vec![
        (
            generic(
                "std.websocket.WebSocketConnectResult",
                vec![context.clone()],
            ),
            json!({
                "tag": "accept",
                "context": { "userId": "user-1" },
                "businessIdentity": "user-1",
                "connectionPolicy": {
                    "maxConnections": 1,
                    "overflow": "close-oldest",
                    "closeCode": 4009,
                    "closeReason": "host connection replaced",
                },
            }),
        ),
        (
            generic(
                "std.websocket.WebSocketConnectResult",
                vec![context.clone()],
            ),
            json!({
                "tag": "accept",
                "context": { "userId": "user-1" },
                "businessIdentity": "user-1",
                "connectionPolicy": {
                    "maxConnections": 1,
                    "overflow": "reject-new",
                    "closeCode": null,
                    "closeReason": null,
                },
            }),
        ),
        (
            generic("WebSocketConnectResult", vec![context.clone()]),
            json!({
                "tag": "reject",
                "code": 1008,
                "reason": "policy",
            }),
        ),
        (
            generic("std.websocket.WebSocketConnectResult", vec![context]),
            json!({
                "tag": "reject",
                "code": 1013,
                "reason": "try-again",
            }),
        ),
    ];

    for (expected, input) in cases {
        let mut heap = RequestHeap::default();
        let value = from_wire(&input, &expected, &mut heap)
            .expect("WebSocketConnectResult should decode from builtin descriptor");
        let output = to_wire(&value, &expected, &mut heap)
            .expect("WebSocketConnectResult should encode from builtin descriptor");
        assert_eq!(output, input);
    }
}

#[test]
fn websocket_event_builtin_descriptors_decode_and_encode() {
    let context = record(
        "api.example.ConnectionContext",
        vec![("userId", named("string"))],
    );
    let connection = json!({
        "id": "connection-1",
        "businessIdentity": "user-1",
        "context": { "userId": "user-1" },
    });
    let cases = vec![(
        generic("std.websocket.WebSocketReceiveEvent", vec![context.clone()]),
        json!({
            "connection": connection.clone(),
            "message": { "tag": "text", "text": "{\"type\":\"hello\"}" },
        }),
    )];

    for (expected, input) in cases {
        let mut heap = RequestHeap::default();
        let value = from_wire(&input, &expected, &mut heap)
            .expect("websocket event should decode from builtin descriptor");
        let output = to_wire(&value, &expected, &mut heap)
            .expect("websocket event should encode from builtin descriptor");
        assert_eq!(output, input);
    }
}

#[test]
fn record_and_json_object_same_wire_shape_decode_to_different_node_kinds() {
    let record = json!({
        "kind": "record",
        "name": "User",
        "fields": {
            "name": named("string")
        }
    });
    let object = json!({ "name": "alice" });
    let mut record_heap = RequestHeap::default();
    let mut json_heap = RequestHeap::default();

    let record_value = from_wire(&object, &record, &mut record_heap).expect("record should decode");
    let json_object_value =
        from_wire(&object, &named("JsonObject"), &mut json_heap).expect("JsonObject should decode");

    let RuntimeValue::Heap(record_handle) = record_value else {
        panic!("expected record heap value");
    };
    let RuntimeValue::Heap(json_object_handle) = json_object_value else {
        panic!("expected JsonObject heap value");
    };

    assert!(matches!(
        record_heap
            .get(record_handle)
            .expect("record should resolve"),
        HeapNode::Object(_)
    ));
    assert!(matches!(
        json_heap
            .get(json_object_handle)
            .expect("JsonObject should resolve"),
        HeapNode::Map(_)
    ));
}

#[test]
fn json_fallback_treats_representation_like_fields_as_plain_json() {
    let input = json!({
        "id": {
            "__skiffRepresentationType": "UserId",
            "value": "u1"
        },
        "avatar": {
            "__skiffBytesBase64": encode_base64(b"abc")
        }
    });
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &named("Json"), &mut heap).expect("Json should decode");
    let output = to_wire(&value, &named("Json"), &mut heap).expect("Json should encode");

    assert_eq!(output, input);
}

#[test]
fn union_decode_rejects_object_when_string_or_number_branches_fail() {
    let expected = union(vec![named("string"), named("number")]);
    let mut heap = RequestHeap::default();

    let error = from_wire(&json!({ "value": true }), &expected, &mut heap).unwrap_err();

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(heap.is_empty());
}

#[test]
fn tagged_union_decode_rejects_unknown_or_mismatched_object() {
    let success = record(
        "Success",
        vec![
            (
                "tag",
                json!({ "kind": "literal", "value": { "kind": "string", "value": "success" } }),
            ),
            ("value", named("string")),
        ],
    );
    let failure = record(
        "Failure",
        vec![
            (
                "tag",
                json!({ "kind": "literal", "value": { "kind": "string", "value": "failure" } }),
            ),
            ("code", named("number")),
        ],
    );
    let expected = union(vec![success, failure]);
    let mut unknown_tag_heap = RequestHeap::default();
    let mut mismatch_heap = RequestHeap::default();

    let unknown_tag = from_wire(
        &json!({ "tag": "pending", "value": "x" }),
        &expected,
        &mut unknown_tag_heap,
    )
    .unwrap_err();
    let mismatch = from_wire(
        &json!({ "tag": "success", "code": 500 }),
        &expected,
        &mut mismatch_heap,
    )
    .unwrap_err();

    assert!(matches!(unknown_tag, RuntimeError::Decode(_)));
    assert!(matches!(mismatch, RuntimeError::Decode(_)));
    assert!(unknown_tag_heap.is_empty());
    assert!(mismatch_heap.is_empty());
}

#[test]
fn failed_union_branches_do_not_pollute_heap() {
    let expected = union(vec![
        array(record("User", vec![("name", named("string"))])),
        named("string"),
    ]);
    let mut heap = RequestHeap::default();

    let error = from_wire(&json!([{ "name": 123 }]), &expected, &mut heap).unwrap_err();

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert_eq!(heap.len(), 0);
}

#[test]
fn alias_array_of_named_records_validates_element_fields() {
    let user = record("User", vec![("name", named("string"))]);
    let expected = alias("UserList", array(user));
    let mut heap = RequestHeap::default();

    let error = from_wire(
        &json!([{ "name": "alice" }, { "name": 123 }]),
        &expected,
        &mut heap,
    )
    .unwrap_err();

    assert!(matches!(error, RuntimeError::Decode(_)));
}

#[test]
fn arbitrary_json_requires_explicit_json_descriptor() {
    let input = json!({ "nested": ["anything", 1, true] });
    let mut string_heap = RequestHeap::default();
    let mut json_heap = RequestHeap::default();

    let string_error = from_wire(&input, &named("string"), &mut string_heap).unwrap_err();
    let value = from_wire(&input, &named("Json"), &mut json_heap).expect("Json should decode");
    let output = to_wire(&value, &named("Json"), &mut json_heap).expect("Json should encode");

    assert!(matches!(string_error, RuntimeError::Decode(_)));
    assert_eq!(output, input);
}
