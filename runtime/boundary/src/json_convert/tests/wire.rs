use serde_json::json;

use crate::{
    error::RuntimeError,
    json::RuntimeBoundaryCodec,
    json_convert::{decode_wire_plan_impl, BoundaryStreamHandlePolicy},
    plan::BoundaryUse,
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueKey},
    type_descriptor::{
        RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan, RuntimeTypePlanDescriptorExt,
    },
    value::encode_base64,
};
use skiff_artifact_model::PackageSchemaTypeId;
use skiff_runtime_model::service_error::{
    CatchIdentity, NominalTypeIdentity, PackageSchemaTypeIdentity,
};

use super::{
    super::{from_wire, to_wire},
    helpers::{alias, array, generic, map, named, record, representation, union},
};

fn package_catch_identity(key: &str, type_id: &str) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::PackageSchema(
        PackageSchemaTypeIdentity::new("example.identity", key, PackageSchemaTypeId::new(type_id))
            .expect("test PackageSchema identity"),
    ))
}

#[test]
fn typed_wire_decode_preserves_nested_nominal_identity_sidecars() {
    let item_identity = package_catch_identity("Item", "schema:item");
    let mut item_plan =
        RuntimeTypePlan::synthetic_request_record(vec![RuntimeRecordFieldPlan::new(
            "name",
            RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new()),
            true,
        )]);
    item_plan.identity.catch_identity = Some(item_identity.clone());
    let outer_plan = RuntimeTypePlan::synthetic_request_record(vec![
        RuntimeRecordFieldPlan::new(
            "items",
            RuntimeTypePlan::synthetic_array(item_plan.clone()),
            true,
        ),
        RuntimeRecordFieldPlan::new(
            "lookup",
            RuntimeTypePlan::synthetic_map(
                RuntimeTypePlan::synthetic_named_builtin(
                    "string",
                    RuntimeTypeNode::String,
                    Vec::new(),
                ),
                item_plan,
            ),
            true,
        ),
    ]);
    let mut heap = RequestHeap::default();

    let decoded = decode_wire_plan_impl(
        &json!({
            "items": [{ "name": "Ada" }],
            "lookup": { "primary": { "name": "Grace" } }
        }),
        &outer_plan,
        &mut heap,
        BoundaryStreamHandlePolicy::ExternalBoundary,
    )
    .expect("typed decode");
    let RuntimeValue::Heap(outer_handle) = decoded else {
        panic!("outer record");
    };
    let items = heap
        .object_field_carrier(outer_handle, "items")
        .expect("items field")
        .expect("items carrier");
    let RuntimeValue::Heap(items_handle) = items.value() else {
        panic!("items array");
    };
    assert_eq!(
        heap.array_item_carrier(*items_handle, 0)
            .expect("items array")
            .expect("first item")
            .catch_identity(),
        Some(&item_identity)
    );
    let lookup = heap
        .object_field_carrier(outer_handle, "lookup")
        .expect("lookup field")
        .expect("lookup carrier");
    let RuntimeValue::Heap(lookup_handle) = lookup.value() else {
        panic!("lookup map");
    };
    assert_eq!(
        heap.map_entry_carrier(*lookup_handle, &RuntimeValueKey::string("primary"),)
            .expect("lookup map")
            .expect("primary item")
            .catch_identity(),
        Some(&item_identity)
    );
}

#[test]
fn task_ref_stays_opaque_on_external_json_boundaries() {
    let plan = RuntimeTypePlan::synthetic_named_builtin(
        "std.task.TaskRef",
        RuntimeTypeNode::TaskRef,
        Vec::new(),
    );
    let mut heap = RequestHeap::default();
    let codec = RuntimeBoundaryCodec::new(&plan, BoundaryUse::TypedJson, "external boundary");
    let canonical = "skiff-task-v1:b3duZXI.dGFzay0x";

    let decode_error = codec
        .from_wire_json(&json!(canonical), &mut heap)
        .expect_err("external JSON decode must refuse TaskRef");
    assert!(
        decode_error.to_string().contains("opaque handle"),
        "unexpected external decode error: {decode_error}"
    );

    let encode_error = codec
        .to_wire_json(&RuntimeValue::String(canonical.to_string()), &mut heap)
        .expect_err("external JSON encode must refuse TaskRef");
    assert!(
        encode_error.to_string().contains("opaque handle"),
        "unexpected external encode error: {encode_error}"
    );
}

#[test]
fn db_result_lane_roundtrips_task_ref_canonical_string() {
    let task_ref_plan = RuntimeTypePlan::synthetic_named_builtin(
        "std.task.TaskRef",
        RuntimeTypeNode::TaskRef,
        Vec::new(),
    );
    let record_plan = RuntimeTypePlan::synthetic_request_record(vec![
        RuntimeRecordFieldPlan::new("ref", task_ref_plan, true),
        RuntimeRecordFieldPlan::new(
            "title",
            RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new()),
            true,
        ),
    ]);
    let canonical = "skiff-task-v1:b3duZXI.dGFzay0x";
    let mut heap = RequestHeap::default();
    let codec = RuntimeBoundaryCodec::new(&record_plan, BoundaryUse::DbResultDecode, "db result");

    let decoded = codec
        .from_wire_json(
            &json!({ "ref": canonical, "title": "task entry" }),
            &mut heap,
        )
        .expect("DB result decode must accept canonical TaskRef strings");
    let RuntimeValue::Heap(handle) = decoded else {
        panic!("db result must decode to a record");
    };
    assert_eq!(
        heap.object_field_carrier(handle, "ref")
            .expect("ref carrier")
            .expect("ref present")
            .value(),
        &RuntimeValue::String(canonical.to_string())
    );

    let encoded = codec
        .to_wire_json(&decoded, &mut heap)
        .expect("DB result encode must render canonical TaskRef strings");
    assert_eq!(encoded, json!({ "ref": canonical, "title": "task entry" }));

    // The write-projection lane shares the same owner-internal allowance.
    let write_codec =
        RuntimeBoundaryCodec::new(&record_plan, BoundaryUse::DbWriteProjection, "db write");
    let encoded = write_codec
        .to_wire_json(&decoded, &mut heap)
        .expect("DB write projection encode must render canonical TaskRef strings");
    assert_eq!(encoded, json!({ "ref": canonical, "title": "task entry" }));
}

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
fn union_decode_fails_closed_when_multiple_exact_branches_match() {
    let expected = union(vec![named("string"), named("string")]);
    let mut heap = RequestHeap::default();

    let error = from_wire(&json!("ambiguous"), &expected, &mut heap)
        .expect_err("an ambiguous branch must not acquire the first branch identity");

    assert!(error.to_string().contains("ambiguously matched"));
    assert!(heap.is_empty());
}

#[test]
fn union_decode_probe_cost_does_not_scale_with_preexisting_heap() {
    let expected = union(vec![named("string"), named("number")]);
    let mut heap = RequestHeap::default();
    for _ in 0..100_000 {
        heap.alloc_array(Vec::new()).expect("preexisting heap node");
    }
    let initial_len = heap.len();
    let started = std::time::Instant::now();

    for _ in 0..1_000 {
        from_wire(&json!("matched"), &expected, &mut heap).expect("union decode");
    }

    assert_eq!(heap.len(), initial_len);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "union probing copied work proportional to the preexisting request heap: {:?}",
        started.elapsed()
    );
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
