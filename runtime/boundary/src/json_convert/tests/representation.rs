use serde_json::json;

use crate::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue},
};

use super::{
    super::{coerce_runtime_value, from_wire, to_wire},
    helpers::{array, named, nullable, record, representation},
};

#[test]
fn representation_wire_roundtrip_uses_erased_heap_payload() {
    let expected = representation("NameList", array(named("string")));
    let input = json!(["a", "b"]);
    let mut heap = RequestHeap::default();

    let value = from_wire(&input, &expected, &mut heap).expect("representation should decode");
    let RuntimeValue::Heap(handle) = value else {
        panic!("expected heap payload value");
    };
    assert!(matches!(
        heap.get(handle).expect("payload should resolve"),
        HeapNode::Array(_)
    ));

    let output = to_wire(&RuntimeValue::Heap(handle), &expected, &mut heap)
        .expect("representation should encode");
    assert_eq!(output, input);
}

#[test]
fn representation_wire_encodes_plain_payload_by_schema() {
    let expected = representation("Name", named("string"));
    let mut heap = RequestHeap::default();

    let output = to_wire(
        &RuntimeValue::String("Ada".to_string()),
        &expected,
        &mut heap,
    )
    .expect("plain payload should encode through representation schema");

    assert_eq!(output, json!("Ada"));
}

#[test]
fn representation_wire_encodes_erased_scalar_payload_for_unqualified_descriptor() {
    let expected = representation("Name", named("string"));
    let mut heap = RequestHeap::default();

    let output = to_wire(
        &RuntimeValue::String("Ada".to_string()),
        &expected,
        &mut heap,
    )
    .expect("erased representation should encode through descriptor");

    assert_eq!(output, json!("Ada"));
}

#[test]
fn duration_representation_wire_uses_integer_milliseconds_payload() {
    let expected = representation("std.time.Duration", named("integer"));
    let mut heap = RequestHeap::default();

    let value = from_wire(&json!(2_000), &expected, &mut heap)
        .expect("Duration should decode from integer milliseconds");
    assert_eq!(value, RuntimeValue::Number(2_000.0));

    let output = to_wire(&value, &expected, &mut heap)
        .expect("Duration should encode to integer milliseconds");
    assert_eq!(output, json!(2_000));
}

#[test]
fn record_coerce_accepts_erased_string_payload_for_string_field() {
    let expected = record(
        "chat.ChatDurableMailboxHint",
        vec![
            (
                "status",
                json!({ "kind": "literal", "value": { "kind": "string", "value": "reserved" } }),
            ),
            ("mailboxId", nullable(named("string"))),
            ("lastCursor", nullable(named("string"))),
        ],
    );
    let mut heap = RequestHeap::default();
    let mailbox_id = RuntimeValue::String("thread-1".to_string());
    let mut fields = RuntimeObjectFields::new();
    fields.insert(
        "status".to_string(),
        RuntimeValue::String("reserved".to_string()),
    );
    fields.insert("mailboxId".to_string(), mailbox_id);
    fields.insert("lastCursor".to_string(), RuntimeValue::Null);
    let value = RuntimeValue::Heap(
        heap.alloc_object(RuntimeObject::unshaped(fields))
            .expect("object alloc"),
    );

    let coerced = coerce_runtime_value(&value, &expected, &mut heap).expect("record should coerce");
    let output = to_wire(&coerced, &expected, &mut heap).expect("record should encode");

    assert_eq!(
        output,
        json!({
            "status": "reserved",
            "mailboxId": "thread-1",
            "lastCursor": null,
        })
    );
}
