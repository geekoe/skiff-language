use serde_json::json;

use super::*;
use crate::{
    runtime_value::{RuntimeObject, RuntimeObjectFields},
    type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt},
    value::bytes_value,
};

#[test]
fn direct_http_request_materializes_runtime_value() {
    let type_plan = RuntimeTypePlan::from_descriptor(&json!({
        "kind": "record",
        "fields": {
            "method": { "kind": "builtin", "name": "string", "args": [] },
            "url": { "kind": "builtin", "name": "string", "args": [] },
            "path": { "kind": "builtin", "name": "string", "args": [] },
            "query": {
                "kind": "builtin",
                "name": "Array",
                "args": [name_value_descriptor()]
            },
            "headers": {
                "kind": "builtin",
                "name": "Array",
                "args": [name_value_descriptor()]
            },
            "body": { "kind": "builtin", "name": "bytes", "args": [] }
        }
    }))
    .expect("request plan should build");
    let plan = direct_http_request_coerce_plan(type_plan);
    let mut heap = RequestHeap::default();
    let parts = HttpBoundaryRequestParts {
        method: "POST".to_string(),
        url: "https://example.test/users?id=1".to_string(),
        path: "/users".to_string(),
        query: vec![HttpBoundaryNameValue {
            name: "id".to_string(),
            value: "1".to_string(),
        }],
        headers: vec![HttpBoundaryNameValue {
            name: "content-type".to_string(),
            value: "application/json".to_string(),
        }],
        body: vec![1, 2, 3],
    };

    let value = direct_http_request_runtime_value(&parts, &plan, "test request", &mut heap)
        .expect("request should materialize");

    let fields = RuntimeValueGraph::new(&heap)
        .object_fields_or_error(&value, "request should be object")
        .expect("request should be object");
    assert_eq!(
        fields.get("method"),
        Some(&RuntimeValue::String("POST".to_string()))
    );
    let body = fields.get("body").expect("body field should exist");
    assert_eq!(
        RuntimeValueGraph::new(&heap)
            .bytes_or_error(body, "body should be bytes")
            .expect("body should be bytes"),
        &[1, 2, 3]
    );
}

#[test]
fn direct_http_response_reads_erased_payloads() {
    let type_plan = RuntimeTypePlan::from_descriptor(&json!({
        "kind": "record",
        "fields": {
            "status": { "kind": "builtin", "name": "integer", "args": [] },
            "headers": {
                "kind": "builtin",
                "name": "Array",
                "args": [name_value_descriptor()]
            },
            "body": { "kind": "builtin", "name": "bytes", "args": [] }
        }
    }))
    .expect("response plan should build");
    let plan = direct_http_response_coerce_plan(type_plan);
    let mut heap = RequestHeap::default();
    let header_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            (
                "name".to_string(),
                RuntimeValue::String("x-test".to_string()),
            ),
            ("value".to_string(), RuntimeValue::String("ok".to_string())),
        ])))
        .expect("header should allocate");
    let headers_handle = heap
        .alloc_array(vec![RuntimeValue::Heap(header_handle)])
        .expect("headers should allocate");
    let body_handle = heap
        .alloc_bytes(vec![4, 5, 6])
        .expect("body should allocate");
    let response_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("status".to_string(), RuntimeValue::Number(204.0)),
            ("headers".to_string(), RuntimeValue::Heap(headers_handle)),
            ("body".to_string(), RuntimeValue::Heap(body_handle)),
        ])))
        .expect("response should allocate");

    let response = direct_http_response_from_runtime_value(
        &RuntimeValue::Heap(response_handle),
        &plan,
        "test response",
        &mut heap,
    )
    .expect("response should extract");

    assert_eq!(
        response,
        HttpBoundaryResponseParts {
            status: 204,
            headers: vec![HttpBoundaryNameValue {
                name: "x-test".to_string(),
                value: "ok".to_string(),
            }],
            body: vec![4, 5, 6],
        }
    );
}

#[test]
fn http_response_stream_event_from_wire_reads_bytes() {
    let event = http_response_stream_event_from_wire(&json!({
        "tag": "chunk",
        "value": bytes_value(&[7, 8, 9]),
    }))
    .expect("chunk should parse");

    assert_eq!(event, HttpBoundaryResponseStreamEvent::Chunk(vec![7, 8, 9]));
}

#[test]
fn http_response_stream_event_rejects_malformed_or_extra_fields() {
    for malformed in [
        json!({"tag": "start", "status": 200}),
        json!({"tag": "chunk", "value": bytes_value(&[1]), "extra": true}),
        json!({"tag": "end", "value": bytes_value(&[])}),
        json!({
            "tag": "start",
            "status": 200,
            "headers": [{"name": "x", "value": "y", "extra": "z"}]
        }),
    ] {
        assert!(
            http_response_stream_event_from_wire(&malformed).is_err(),
            "malformed event must fail: {malformed}"
        );
    }
}

fn name_value_descriptor() -> Value {
    json!({
        "kind": "record",
        "fields": {
            "name": { "kind": "builtin", "name": "string", "args": [] },
            "value": { "kind": "builtin", "name": "string", "args": [] }
        }
    })
}
