use std::collections::BTreeMap;

use crate::request_heap::RequestHeap;

use super::{
    runtime_map_has, runtime_values_equal, HeapHandle, InterfaceCarrier, InterfaceValue,
    RemoteOperationSlot, RemoteOperationTable, RuntimeMap, RuntimeObject, RuntimeObjectFields,
    RuntimeValue, RuntimeValueKey,
};

#[test]
fn heap_runtime_value_equality_uses_handle_identity() {
    let first = RuntimeValue::Heap(HeapHandle::new(0, 0));
    let same = RuntimeValue::Heap(HeapHandle::new(0, 0));
    let different_index = RuntimeValue::Heap(HeapHandle::new(1, 0));
    let different_generation = RuntimeValue::Heap(HeapHandle::new(0, 1));

    assert_eq!(first, same);
    assert_ne!(first, different_index);
    assert_ne!(first, different_generation);
}

#[test]
fn runtime_value_key_orders_by_plain_string_payload() {
    let mut keys = BTreeMap::new();
    keys.insert(RuntimeValueKey::string("u2"), 4);
    keys.insert(RuntimeValueKey::string("u1"), 3);
    keys.insert(RuntimeValueKey::string("u1"), 1);

    let ordered = keys.into_keys().collect::<Vec<_>>();

    assert_eq!(
        ordered,
        vec![RuntimeValueKey::string("u1"), RuntimeValueKey::string("u2"),]
    );
}

#[test]
fn runtime_equality_compares_erased_payload_values() {
    let heap = RequestHeap::default();

    assert!(
        runtime_values_equal(&heap, &RuntimeValue::from("u1"), &RuntimeValue::from("u1"))
            .expect("equality should evaluate")
    );
}

#[test]
fn runtime_equality_compares_distinct_heap_bytes_by_value() {
    let mut heap = RequestHeap::default();
    let first = heap.alloc_bytes(vec![1, 2, 3]).expect("bytes allocate");
    let same = heap.alloc_bytes(vec![1, 2, 3]).expect("bytes allocate");
    let different = heap.alloc_bytes(vec![1, 2]).expect("bytes allocate");

    assert!(
        runtime_values_equal(&heap, &RuntimeValue::Heap(first), &RuntimeValue::Heap(same))
            .expect("equality should evaluate")
    );
    assert!(!runtime_values_equal(
        &heap,
        &RuntimeValue::Heap(first),
        &RuntimeValue::Heap(different)
    )
    .expect("equality should evaluate"));
}

#[test]
fn runtime_equality_uses_heap_identity_for_non_bytes_nodes() {
    let mut heap = RequestHeap::default();
    let first_array = heap.alloc_array(Vec::new()).expect("array should allocate");
    let second_array = heap.alloc_array(Vec::new()).expect("array should allocate");

    assert!(runtime_values_equal(
        &heap,
        &RuntimeValue::Heap(first_array),
        &RuntimeValue::Heap(first_array)
    )
    .expect("equality should evaluate"));
    assert!(!runtime_values_equal(
        &heap,
        &RuntimeValue::Heap(first_array),
        &RuntimeValue::Heap(second_array)
    )
    .expect("equality should evaluate"));
}

#[test]
fn runtime_equality_rejects_interface_wrapper_before_heap_identity_shortcut() {
    let mut heap = RequestHeap::default();
    let interface = heap
        .alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::Remote {
                dependency_ref: "reader-service".to_string(),
                public_instance_key: "readers/default".to_string(),
                operations: RemoteOperationTable::new(
                    "remote:reader".to_string(),
                    "pkg.Reader".to_string(),
                    vec![RemoteOperationSlot::new(
                        0,
                        "method:pkg.Reader:read".to_string(),
                        "operation:reader:read".to_string(),
                    )],
                ),
            },
        ))
        .expect("interface should allocate");
    let value = RuntimeValue::Heap(interface);

    let self_error = runtime_values_equal(&heap, &value, &value)
        .expect_err("interface self equality must fail closed");
    assert!(
        self_error
            .to_string()
            .contains("interface value any interface pkg.Reader"),
        "unexpected error: {self_error}"
    );

    let scalar_error = runtime_values_equal(&heap, &value, &RuntimeValue::from("reader"))
        .expect_err("interface equality against scalar must fail closed");
    assert!(
        scalar_error
            .to_string()
            .contains("does not define equality"),
        "unexpected error: {scalar_error}"
    );
}

#[test]
fn runtime_map_has_matches_map_entries() {
    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("present"),
        RuntimeValue::Number(1.0),
    );
    let receiver = RuntimeValue::Heap(heap.alloc_map(map).expect("map should allocate"));

    assert!(
        runtime_map_has(&heap, &receiver, &RuntimeValue::from("present"))
            .expect("map has should evaluate")
    );
    assert!(
        !runtime_map_has(&heap, &receiver, &RuntimeValue::from("missing"))
            .expect("map has should evaluate")
    );
}

#[test]
fn runtime_map_has_matches_object_fields() {
    let mut heap = RequestHeap::default();
    let fields = RuntimeObjectFields::from([(
        "present".to_string(),
        RuntimeValue::String("value".to_string()),
    )]);
    let receiver = RuntimeValue::Heap(
        heap.alloc_object(RuntimeObject::unshaped(fields))
            .expect("object should allocate"),
    );

    assert!(
        runtime_map_has(&heap, &receiver, &RuntimeValue::from("present"))
            .expect("object has should evaluate")
    );
    assert!(
        !runtime_map_has(&heap, &receiver, &RuntimeValue::from("missing"))
            .expect("object has should evaluate")
    );
}

#[test]
fn runtime_map_has_returns_false_for_non_map_like_values() {
    let mut heap = RequestHeap::default();
    let array = RuntimeValue::Heap(heap.alloc_array(Vec::new()).expect("array should allocate"));

    assert!(!runtime_map_has(
        &heap,
        &RuntimeValue::Number(1.0),
        &RuntimeValue::from("key")
    )
    .expect("scalar has should evaluate"));
    assert!(!runtime_map_has(&heap, &array, &RuntimeValue::from("key"))
        .expect("array has should evaluate"));
}

#[test]
fn runtime_map_has_fails_closed_for_interface_heap_values() {
    let mut heap = RequestHeap::default();
    let interface = heap
        .alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::Remote {
                dependency_ref: "reader-service".to_string(),
                public_instance_key: "readers/default".to_string(),
                operations: RemoteOperationTable::new(
                    "remote:reader".to_string(),
                    "pkg.Reader".to_string(),
                    vec![RemoteOperationSlot::new(
                        0,
                        "method:pkg.Reader:read".to_string(),
                        "operation:reader:read".to_string(),
                    )],
                ),
            },
        ))
        .expect("interface should allocate");
    let receiver = RuntimeValue::Heap(interface);

    let error = runtime_map_has(&heap, &receiver, &RuntimeValue::from("key"))
        .expect_err("interface has must fail closed");
    assert!(
        error
            .to_string()
            .contains("any interface pkg.Reader (remote) is not map-like"),
        "unexpected error: {error}"
    );
}

#[test]
fn runtime_map_has_rejects_non_string_object_key() {
    let mut heap = RequestHeap::default();
    let receiver = RuntimeValue::Heap(
        heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::new()))
            .expect("object should allocate"),
    );

    let error = runtime_map_has(&heap, &receiver, &RuntimeValue::Number(1.0))
        .expect_err("non-string object key must fail");
    assert_eq!(
        error.to_string(),
        "Map key must be string or representation-over-string"
    );
}
