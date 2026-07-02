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
    helpers::{array, map, named, record, union},
};

fn assert_reserved_legacy_metadata_error(error: RuntimeError) {
    let message = error.to_string();
    assert!(
        message.contains("reserved Skiff metadata field __skiffType")
            && message.contains("JSON boundary"),
        "unexpected error: {message}"
    );
}

#[test]
fn openapi_map_descriptor_roundtrips_and_coerces_without_materializing() {
    let expected = json!({
        "type": "object",
        "additionalProperties": { "type": "string" }
    });
    let input = json!({ "first": "Ada", "last": "Lovelace" });
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &expected, &mut heap).expect("map should decode");
    let RuntimeValue::Heap(handle) = value.clone() else {
        panic!("expected heap map");
    };
    assert!(matches!(
        heap.get(handle).expect("map should resolve"),
        HeapNode::Map(_)
    ));

    let coerced = coerce_runtime_value(&value, &expected, &mut heap)
        .expect("map runtime value should coerce");
    assert_eq!(coerced, value);
    assert_eq!(heap.stats().materialize_depth, 0);
    assert_eq!(heap.stats().materialize_output_bytes, 0);

    let output = to_wire(&value, &expected, &mut heap).expect("map should encode");
    assert_eq!(output, input);
}

#[test]
fn runtime_coerce_preserves_heap_identity_without_materializing_wire_json() {
    let expected = array(named("string"));
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_array(vec![RuntimeValue::String("Ada".to_string())])
        .expect("array should allocate");
    let value = RuntimeValue::Heap(handle);

    let output =
        coerce_runtime_value(&value, &expected, &mut heap).expect("array should coerce in place");

    assert_eq!(output, value);
    assert_eq!(heap.stats().materialize_depth, 0);
    assert_eq!(heap.stats().materialize_output_bytes, 0);
}

#[test]
fn runtime_coerce_converts_object_to_map_without_materializing_wire_json() {
    let expected = map(named("string"), named("string"));
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeObjectFields::new();
    fields.insert("name".to_string(), RuntimeValue::String("Ada".to_string()));
    let handle = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("object should allocate");
    let value = RuntimeValue::Heap(handle);

    let output =
        coerce_runtime_value(&value, &expected, &mut heap).expect("object should coerce to map");

    let RuntimeValue::Heap(handle) = output else {
        panic!("expected heap map");
    };
    let HeapNode::Map(map) = heap.get(handle).expect("map should resolve") else {
        panic!("expected map node");
    };
    assert_eq!(
        map.get(&RuntimeValueKey::string("name")),
        Some(&RuntimeValue::String("Ada".to_string()))
    );
    assert_eq!(heap.stats().materialize_depth, 0);
    assert_eq!(heap.stats().materialize_output_bytes, 0);
}

#[test]
fn runtime_coerce_typed_map_rejects_reserved_legacy_metadata_object_field() {
    let expected = map(named("string"), named("string"));
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "__skiffType".to_string(),
        RuntimeValue::String("User".to_string()),
    );
    fields.insert("name".to_string(), RuntimeValue::String("Ada".to_string()));
    let handle = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("object should allocate");

    let error = coerce_runtime_value(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect_err("typed Map coercion should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn runtime_coerce_typed_map_rejects_reserved_legacy_metadata_existing_map_key() {
    let expected = map(named("string"), named("string"));
    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("__skiffType"),
        RuntimeValue::String("User".to_string()),
    );
    map.insert(
        RuntimeValueKey::string("name"),
        RuntimeValue::String("Ada".to_string()),
    );
    let handle = heap.alloc_map(map).expect("map should allocate");

    let error = coerce_runtime_value(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect_err("typed Map coercion should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn runtime_coerce_json_object_rejects_reserved_legacy_metadata_object_field() {
    let expected = named("JsonObject");
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "__skiffType".to_string(),
        RuntimeValue::String("User".to_string()),
    );
    fields.insert("id".to_string(), RuntimeValue::String("u1".to_string()));
    let handle = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("object should allocate");

    let error = coerce_runtime_value(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect_err("JsonObject coercion should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn runtime_coerce_json_object_rejects_reserved_legacy_metadata_map_key() {
    let expected = named("JsonObject");
    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("__skiffType"),
        RuntimeValue::String("User".to_string()),
    );
    map.insert(
        RuntimeValueKey::string("id"),
        RuntimeValue::String("u1".to_string()),
    );
    let handle = heap.alloc_map(map).expect("map should allocate");

    let error = coerce_runtime_value(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect_err("JsonObject coercion should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn runtime_coerce_json_rejects_reserved_legacy_metadata_direct_path() {
    let expected = named("Json");
    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("__skiffType"),
        RuntimeValue::String("User".to_string()),
    );
    map.insert(
        RuntimeValueKey::string("id"),
        RuntimeValue::String("u1".to_string()),
    );
    let handle = heap.alloc_map(map).expect("map should allocate");

    let error = coerce_runtime_value(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect_err("Json coercion should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn runtime_coerce_record_rejects_reserved_legacy_metadata_source_field() {
    let expected = record("User", vec![("name", named("string"))]);
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "__skiffType".to_string(),
        RuntimeValue::String("User".to_string()),
    );
    fields.insert("name".to_string(), RuntimeValue::String("Ada".to_string()));
    let handle = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("object should allocate");

    let error = coerce_runtime_value(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect_err("typed Record coercion should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn runtime_coerce_record_rejects_declared_reserved_legacy_metadata_field() {
    let expected = record(
        "User",
        vec![("__skiffType", named("string")), ("name", named("string"))],
    );
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "__skiffType".to_string(),
        RuntimeValue::String("User".to_string()),
    );
    fields.insert("name".to_string(), RuntimeValue::String("Ada".to_string()));
    let handle = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("object should allocate");

    let error = coerce_runtime_value(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect_err("typed Record coercion should reject declared reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn failed_runtime_union_branches_roll_back_new_nodes() {
    let expected = union(vec![
        array(record("User", vec![("name", named("string"))])),
        named("string"),
    ]);
    let mut heap = RequestHeap::default();

    let mut valid = RuntimeMap::new();
    valid.insert(
        RuntimeValueKey::string("name"),
        RuntimeValue::String("Ada".to_string()),
    );
    let valid = heap.alloc_map(valid).expect("valid map should allocate");

    let mut invalid = RuntimeMap::new();
    invalid.insert(RuntimeValueKey::string("name"), RuntimeValue::Number(123.0));
    let invalid = heap
        .alloc_map(invalid)
        .expect("invalid map should allocate");

    let input = heap
        .alloc_array(vec![RuntimeValue::Heap(valid), RuntimeValue::Heap(invalid)])
        .expect("array should allocate");
    let original_len = heap.len();

    let error = coerce_runtime_value(&RuntimeValue::Heap(input), &expected, &mut heap).unwrap_err();

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert_eq!(heap.len(), original_len);
    assert_eq!(heap.stats().node_count, original_len);
}
