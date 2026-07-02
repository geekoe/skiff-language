use serde_json::json;

use crate::{
    error::RuntimeError,
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

use super::{
    super::{from_wire, to_wire},
    helpers::{array, named},
};

#[test]
fn materialize_rejects_cycles_and_depth_limits() {
    let limits = RequestHeapLimits {
        max_materialize_depth: 1,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let child = heap.alloc_array(Vec::new()).expect("child should allocate");
    let root = heap
        .alloc_array(vec![RuntimeValue::Heap(child)])
        .expect("root should allocate");

    let error = to_wire(&RuntimeValue::Heap(root), &array(named("Json")), &mut heap).unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded { reason, .. } if reason == "max materialize depth"
    ));

    let mut cycle_heap = RequestHeap::default();
    let cycle = cycle_heap
        .alloc_array(Vec::new())
        .expect("cycle should allocate");
    cycle_heap
        .push_array_item_without_cycle_check_for_test(cycle, RuntimeValue::Heap(cycle))
        .expect("cycle setup should update");

    let error = to_wire(
        &RuntimeValue::Heap(cycle),
        &array(named("Json")),
        &mut cycle_heap,
    )
    .unwrap_err();
    assert!(error.to_string().contains("cyclic heap graph"));
}

#[test]
fn materialize_output_bytes_limit_is_enforced() {
    let limits = RequestHeapLimits {
        max_materialize_output_bytes: 8,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let value =
        from_wire(&json!("too long"), &named("string"), &mut heap).expect("string should decode");

    let error = to_wire(&value, &named("string"), &mut heap).unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded { reason, .. } if reason == "max materialize output bytes"
    ));
}
