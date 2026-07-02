use crate::{
    error::RuntimeError,
    json::RuntimeBoundaryCodec,
    plan::BoundaryUse,
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, InterfaceCarrier, InterfaceMethodTable, InterfaceValue, RuntimeMap,
        RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
    },
    type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt},
};

use super::{
    super::{decode_json_text_runtime_value, encode_json_runtime_value, to_json_runtime_value},
    helpers::{alias, named, record},
};

fn assert_reserved_legacy_metadata_error(error: RuntimeError) {
    let message = error.to_string();
    assert!(
        message.contains("reserved Skiff metadata field __skiffType")
            && message.contains("JSON boundary"),
        "unexpected error: {message}"
    );
}

fn assert_interface_boundary_error(error: RuntimeError) {
    let message = error.to_string();
    assert!(
        message.contains("any interface pkg.Reader")
            && (message.contains("cannot") || message.contains("is not")),
        "unexpected error: {message}"
    );
}

fn interface_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::Local {
                concrete_type: "pkg.FileReader".to_string(),
                method_table: InterfaceMethodTable::new(
                    "table:pkg.Reader:pkg.FileReader".to_string(),
                    "pkg.Reader".to_string(),
                    Vec::new(),
                ),
                payload: RuntimeValue::Null,
            },
        ))
        .expect("interface should allocate"),
    )
}

#[test]
fn runtime_json_from_rejects_reserved_legacy_metadata_object_field() {
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "__skiffType".to_string(),
        RuntimeValue::String("User".to_string()),
    );
    fields.insert("id".to_string(), RuntimeValue::String("u1".to_string()));
    let object = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("object should allocate");
    let value = RuntimeValue::Heap(object);

    let error = to_json_runtime_value(&value, None, &mut heap)
        .expect_err("reserved legacy metadata should not become Json runtime");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn runtime_json_from_rejects_reserved_legacy_metadata_map_key() {
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
    let value = RuntimeValue::Heap(heap.alloc_map(map).expect("map should allocate"));

    let error = to_json_runtime_value(&value, None, &mut heap)
        .expect_err("reserved legacy metadata should not become Json runtime");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn ordinary_json_materialization_rejects_interface_wrapper() {
    let mut heap = RequestHeap::default();
    let value = interface_runtime_value(&mut heap);

    let error = super::super::to_wire(&value, &named("Json"), &mut heap)
        .expect_err("ordinary JSON materialization should reject interface wrapper");

    assert_interface_boundary_error(error);
}

#[test]
fn json_encode_rejects_interface_wrapper() {
    let mut heap = RequestHeap::default();
    let value = interface_runtime_value(&mut heap);

    let json_error = to_json_runtime_value(&value, None, &mut heap)
        .expect_err("std.json runtime value projection should reject interface wrapper");
    assert_interface_boundary_error(json_error);

    let encode_error = encode_json_runtime_value(&value, Some(&named("Json")), &mut heap)
        .expect_err("std.json text encode should reject interface wrapper");
    assert_interface_boundary_error(encode_error);
}

#[test]
fn db_write_projection_rejects_interface_wrapper() {
    let mut heap = RequestHeap::default();
    let value = interface_runtime_value(&mut heap);
    let plan = RuntimeTypePlan::from_descriptor(&named("Json")).expect("Json plan should build");

    let error = RuntimeBoundaryCodec::new(&plan, BoundaryUse::DbWriteProjection, "db write")
        .to_wire_json(&value, &mut heap)
        .expect_err("DB write projection should reject interface wrapper");

    assert_interface_boundary_error(error);
}

#[test]
fn json_encode_json_rejects_reserved_legacy_metadata_object_field() {
    let mut heap = RequestHeap::default();
    let message_id = RuntimeValue::String("message-1".to_string());
    let role = RuntimeValue::String("assistant".to_string());
    let mut output_fields = RuntimeObjectFields::new();
    output_fields.insert(
        "__skiffType".to_string(),
        RuntimeValue::String("chat.ChatMessageStartOutput".to_string()),
    );
    output_fields.insert("messageId".to_string(), message_id);
    output_fields.insert("role".to_string(), role);
    output_fields.insert(
        "delta".to_string(),
        RuntimeValue::String("hello".to_string()),
    );
    let output = heap
        .alloc_object(RuntimeObject::unshaped(output_fields))
        .expect("runtime object should allocate");

    let error =
        encode_json_runtime_value(&RuntimeValue::Heap(output), Some(&named("Json")), &mut heap)
            .expect_err("Json encode should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn json_encode_json_object_rejects_reserved_legacy_metadata_map_key() {
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeMap::new();
    fields.insert(
        RuntimeValueKey::string("__skiffType"),
        RuntimeValue::String("chat.ChatMessageDeltaOutput".to_string()),
    );
    fields.insert(
        RuntimeValueKey::string("threadId"),
        RuntimeValue::String("thread-1".to_string()),
    );
    fields.insert(
        RuntimeValueKey::string("delta"),
        RuntimeValue::String("hi".to_string()),
    );
    let value = heap.alloc_map(fields).expect("JsonObject should allocate");

    let error = encode_json_runtime_value(
        &RuntimeValue::Heap(value),
        Some(&named("JsonObject")),
        &mut heap,
    )
    .expect_err("JsonObject encode should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
}

#[test]
fn json_encode_plan_rejects_reserved_legacy_metadata_for_direct_and_alias_json() {
    let mut heap = RequestHeap::default();
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "__skiffType".to_string(),
        RuntimeValue::String("User".to_string()),
    );
    fields.insert("id".to_string(), RuntimeValue::String("u1".to_string()));
    let value = heap
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("runtime object should allocate");

    let direct_json_plan =
        RuntimeTypePlan::from_descriptor(&named("Json")).expect("Json plan should build");
    let direct_error = RuntimeBoundaryCodec::new(
        &direct_json_plan,
        BoundaryUse::JsonValueProjection,
        "JSON boundary",
    )
    .encode_json_text_value(&RuntimeValue::Heap(value), &mut heap)
    .expect_err("direct Json encode should reject reserved legacy metadata");
    assert_reserved_legacy_metadata_error(direct_error);

    let alias_json_plan = RuntimeTypePlan::from_descriptor(&alias("JsonAlias", named("Json")))
        .expect("alias plan should build");
    let alias_error = RuntimeBoundaryCodec::new(
        &alias_json_plan,
        BoundaryUse::JsonValueProjection,
        "JSON boundary",
    )
    .encode_json_text_value(&RuntimeValue::Heap(value), &mut heap)
    .expect_err("alias Json encode should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(alias_error);
}

#[test]
fn runtime_json_decode_text_coerces_json_to_record() {
    let expected = record("User", vec![("name", named("string"))]);
    let mut heap = RequestHeap::default();

    let output = decode_json_text_runtime_value("{\"name\":\"Ada\"}", &expected, &mut heap)
        .expect("Json should decode to record");

    let RuntimeValue::Heap(handle) = output else {
        panic!("expected heap object");
    };
    let HeapNode::Object(object) = heap.get(handle).expect("object should resolve") else {
        panic!("expected object node");
    };
    assert_eq!(
        object.fields().get("name"),
        Some(&RuntimeValue::String("Ada".to_string()))
    );
    assert_eq!(heap.stats().materialize_depth, 0);
    assert_eq!(heap.stats().materialize_output_bytes, 0);
}

#[test]
fn runtime_json_decode_text_rejects_reserved_legacy_metadata_on_typed_record() {
    let expected = record("User", vec![("name", named("string"))]);
    let mut heap = RequestHeap::default();

    let error = decode_json_text_runtime_value(
        "{\"__skiffType\":\"User\",\"name\":\"Ada\"}",
        &expected,
        &mut heap,
    )
    .expect_err("typed record decode should reject reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
    assert!(heap.is_empty());
}

#[test]
fn runtime_json_decode_text_rejects_declared_reserved_legacy_metadata_record_field() {
    let expected = record(
        "User",
        vec![("__skiffType", named("string")), ("name", named("string"))],
    );
    let mut heap = RequestHeap::default();

    let error = decode_json_text_runtime_value(
        "{\"__skiffType\":\"User\",\"name\":\"Ada\"}",
        &expected,
        &mut heap,
    )
    .expect_err("typed record decode should reject declared reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
    assert!(heap.is_empty());
}

#[test]
fn runtime_json_decode_text_rejects_reserved_legacy_metadata_inside_json_value() {
    let mut heap = RequestHeap::default();

    let error = decode_json_text_runtime_value(
        "{\"profile\":{\"__skiffType\":\"User\",\"name\":\"Ada\"}}",
        &named("Json"),
        &mut heap,
    )
    .expect_err("Json decode should reject nested reserved legacy metadata");

    assert_reserved_legacy_metadata_error(error);
    assert!(heap.is_empty());
}
