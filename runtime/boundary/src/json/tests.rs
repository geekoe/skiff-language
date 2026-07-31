use serde_json::json;

use crate::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{
        ActorRef, CallbackCapabilityCarrier, HeapHandle, HeapNode, InterfaceCarrier,
        InterfaceValue, RuntimeMap, RuntimeValue, RuntimeValueKey,
    },
};

use super::*;

#[test]
fn untyped_wire_decodes_bytes_arrays_and_objects() {
    let mut heap = RequestHeap::default();
    let value = decode_untyped_wire_json(
        &json!({
            "bytes": { "__skiffBytesBase64": "aGk=" },
            "items": [true, 2]
        }),
        &mut heap,
    )
    .expect("untyped wire should decode");

    let RuntimeValue::Heap(handle) = value else {
        panic!("root should be a heap object");
    };
    let HeapNode::Object(object) = heap.get(handle).expect("object handle") else {
        panic!("root handle should point to object");
    };
    assert_eq!(
        object.fields().get("items"),
        Some(&RuntimeValue::Heap(HeapHandle::new(1, 0)))
    );
    let RuntimeValue::Heap(bytes_handle) = object.fields().get("bytes").unwrap() else {
        panic!("bytes field should be a heap value");
    };
    let HeapNode::Bytes(bytes) = heap.get(*bytes_handle).expect("bytes handle") else {
        panic!("bytes handle should point to bytes");
    };
    assert_eq!(bytes.as_slice(), b"hi");
}

#[test]
fn untyped_wire_encodes_dates_maps_and_heap_values() {
    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("when"),
        RuntimeValue::Date(1_609_459_200_000),
    );
    map.insert(RuntimeValueKey::string("count"), RuntimeValue::Number(2.0));
    let handle = heap.alloc_map(map).expect("map alloc");

    let encoded = encode_untyped_wire_json(&RuntimeValue::Heap(handle), &heap)
        .expect("untyped wire should encode");

    assert_eq!(
        encoded,
        json!({
            "count": 2,
            "when": "2021-01-01T00:00:00.000Z"
        })
    );
}

#[test]
fn untyped_wire_encode_enforces_materialize_depth_limit() {
    let mut heap = RequestHeap::new(RequestHeapLimits {
        max_materialize_depth: 0,
        ..RequestHeapLimits::default()
    });
    let handle = heap
        .alloc_array(vec![RuntimeValue::String("too deep".to_string())])
        .expect("array alloc");

    let error = encode_untyped_wire_json(&RuntimeValue::Heap(handle), &heap)
        .expect_err("nested item should exceed materialize depth");

    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded { reason, .. } if reason == "max materialize depth"
    ));
}

#[test]
fn untyped_wire_rejects_actor_and_interface_values() {
    let heap = RequestHeap::default();
    let actor = RuntimeValue::ActorRef(ActorRef::new(
        "svc",
        "actor-type",
        "id-type",
        "v1",
        Vec::new(),
        "hash",
        None,
    ));
    let error = encode_untyped_wire_json(&actor, &heap).expect_err("actor should reject");
    assert!(error
        .to_string()
        .contains("actor ref actor-type cannot be encoded as wire value"));

    let mut heap = RequestHeap::default();
    let interface = InterfaceValue::new(
        "iface".to_string(),
        InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
            "runtime-a",
            "activation-a",
            1,
            "iface",
            "capability-a",
        )),
    );
    let handle = heap.alloc_interface(interface).expect("interface alloc");
    let error = encode_untyped_wire_json(&RuntimeValue::Heap(handle), &heap)
        .expect_err("interface should reject");
    assert!(error
        .to_string()
        .contains("any interface iface (callback capability) cannot be encoded as wire value"));
}
