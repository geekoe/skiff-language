use serde_json::json;

use crate::{
    error::RuntimeError,
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
    },
};

use super::{
    super::{coerce_runtime_value, from_wire, to_wire},
    helpers::{named, nullable, record},
};

#[test]
fn openapi_record_descriptor_roundtrips_and_coerces_without_materializing() {
    let expected = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "number" }
        },
        "required": ["name", "age"]
    });
    let input = json!({ "name": "Ada", "age": 36 });
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &expected, &mut heap).expect("record should decode");
    let RuntimeValue::Heap(handle) = value.clone() else {
        panic!("expected heap object");
    };
    assert!(matches!(
        heap.get(handle).expect("record should resolve"),
        HeapNode::Object(_)
    ));

    let coerced = coerce_runtime_value(&value, &expected, &mut heap)
        .expect("record runtime value should coerce");
    assert_eq!(coerced, value);
    assert_eq!(heap.stats().materialize_depth, 0);
    assert_eq!(heap.stats().materialize_output_bytes, 0);

    let output = to_wire(&value, &expected, &mut heap).expect("record should encode");
    assert_eq!(output, input);
}

#[test]
fn runtime_record_coerce_fills_missing_optional_fields_with_null() {
    let expected = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "number", "nullable": true }
        },
        "required": ["name"]
    });
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "name".to_string(),
            RuntimeValue::String("Ada".to_string()),
        )])))
        .expect("record should allocate");
    let value = RuntimeValue::Heap(handle);

    let coerced = coerce_runtime_value(&value, &expected, &mut heap)
        .expect("record runtime value should coerce");

    let RuntimeValue::Heap(coerced_handle) = coerced.clone() else {
        panic!("expected heap object");
    };
    let HeapNode::Object(object) = heap.get(coerced_handle).expect("record should resolve") else {
        panic!("expected object node");
    };
    assert_eq!(object.fields().get("age"), Some(&RuntimeValue::Null));

    let output = to_wire(&coerced, &expected, &mut heap).expect("record should encode");
    assert_eq!(output, json!({ "name": "Ada", "age": null }));
}

#[test]
fn runtime_record_coerce_materializes_map_source_as_object() {
    // A record-typed value sourced from a `HeapNode::Map` whose fields already
    // satisfy the plan must still be re-materialized as a `HeapNode::Object`.
    // Discriminated-union literals are evaluated as maps, and downstream object
    // accessors (e.g. the websocket connect adapter) require an object node.
    let expected = record(
        "User",
        vec![("name", named("string")), ("age", named("number"))],
    );
    let mut heap = RequestHeap::default();

    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("name"),
        RuntimeValue::String("Ada".to_string()),
    );
    map.insert(RuntimeValueKey::string("age"), RuntimeValue::Number(36.0));
    let handle = heap.alloc_map(map).expect("map should allocate");
    let value = RuntimeValue::Heap(handle);

    let coerced = coerce_runtime_value(&value, &expected, &mut heap)
        .expect("map runtime value should coerce to record");

    let RuntimeValue::Heap(coerced_handle) = coerced else {
        panic!("expected heap object");
    };
    let HeapNode::Object(object) = heap.get(coerced_handle).expect("record should resolve") else {
        panic!("record coerced from a map must be an object, not a map");
    };
    assert_eq!(
        object.fields().get("name"),
        Some(&RuntimeValue::String("Ada".to_string()))
    );
    assert_eq!(
        object.fields().get("age"),
        Some(&RuntimeValue::Number(36.0))
    );
}

#[test]
fn named_record_decode_rejects_missing_wrong_and_extra_fields() {
    let expected = record(
        "User",
        vec![
            ("name", named("string")),
            ("age", nullable(named("number"))),
        ],
    );

    for input in [
        json!({ "age": 1 }),
        json!({ "name": 123 }),
        json!({ "name": "alice", "age": null, "role": "admin" }),
    ] {
        let mut heap = RequestHeap::default();
        let error = from_wire(&input, &expected, &mut heap).unwrap_err();
        assert!(matches!(error, RuntimeError::Decode(_)));
    }
}

#[test]
fn runtime_rejects_record_type_source_without_structured_fields() {
    let expected = json!({ "kind": "RecordType", "source": "{ name: string }" });
    let mut heap = RequestHeap::default();

    let error = from_wire(&json!({ "name": "alice" }), &expected, &mut heap).unwrap_err();

    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
}
