use std::collections::BTreeMap;

use super::*;
use skiff_artifact_model::builtin_receiver_op_by_name;
use skiff_runtime_model::runtime_value::HeapNode;

fn receiver_op(root: &str, method: &str) -> BuiltinReceiverOp {
    builtin_receiver_op_by_name(root, method).expect("receiver op must exist")
}

#[test]
fn array_push_mutates_the_same_heap_array_and_returns_null() {
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_array(vec![RuntimeValue::Number(1.0)])
        .expect("array allocation should succeed");
    let receiver = RuntimeValue::Heap(handle);

    let result = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(
            &receiver_op("Array", "push"),
            receiver.clone(),
            vec![RuntimeValue::Number(2.0)],
        )
        .expect("Array.push should dispatch");

    assert_eq!(result, RuntimeValue::Null, "Array.push return must be null");
    assert_eq!(
        receiver,
        RuntimeValue::Heap(handle),
        "Array.push must preserve receiver identity"
    );
    let HeapNode::Array(items) = heap.get(handle).expect("array handle should remain live") else {
        panic!("Array.push receiver must remain an array");
    };
    assert_eq!(
        items,
        &vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)],
        "Array.push must mutate the caller-reachable receiver"
    );
}

#[test]
fn map_get_preserves_nested_identity_and_optional_missing_behavior() {
    let mut heap = RequestHeap::default();
    let nested = heap
        .alloc_array(vec![RuntimeValue::String("nested".to_string())])
        .expect("nested array allocation should succeed");
    let map = BTreeMap::from([
        (RuntimeValueKey::string("scalar"), RuntimeValue::Number(7.0)),
        (
            RuntimeValueKey::string("nested"),
            RuntimeValue::Heap(nested),
        ),
    ]);
    let receiver = RuntimeValue::Heap(
        heap.alloc_map(map)
            .expect("Map receiver allocation should succeed"),
    );

    for (key, expected) in [
        ("scalar", RuntimeValue::Number(7.0)),
        ("nested", RuntimeValue::Heap(nested)),
        ("missing", RuntimeValue::Null),
    ] {
        let result = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(
                &receiver_op("Map", "get"),
                receiver.clone(),
                vec![RuntimeValue::String(key.to_string())],
            )
            .expect("Map.get should dispatch");
        assert_eq!(result, expected, "unexpected Map.get result for {key}");
    }
}

#[test]
fn map_has_and_set_preserve_identity_and_nested_values() {
    let mut heap = RequestHeap::default();
    let old_nested = heap
        .alloc_array(vec![RuntimeValue::String("old".to_string())])
        .expect("old nested allocation should succeed");
    let new_nested = heap
        .alloc_array(vec![RuntimeValue::String("new".to_string())])
        .expect("new nested allocation should succeed");
    let map = BTreeMap::from([(
        RuntimeValueKey::string("present"),
        RuntimeValue::Heap(old_nested),
    )]);
    let handle = heap
        .alloc_map(map)
        .expect("Map receiver allocation should succeed");
    let receiver = RuntimeValue::Heap(handle);

    for (key, expected) in [("present", true), ("missing", false)] {
        let result = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(
                &receiver_op("Map", "has"),
                receiver.clone(),
                vec![RuntimeValue::String(key.to_string())],
            )
            .expect("Map.has should dispatch");
        assert_eq!(result, RuntimeValue::Bool(expected));
    }

    for key in ["present", "inserted"] {
        let result = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(
                &receiver_op("Map", "set"),
                receiver.clone(),
                vec![
                    RuntimeValue::String(key.to_string()),
                    RuntimeValue::Heap(new_nested),
                ],
            )
            .expect("Map.set should dispatch");
        assert_eq!(result, RuntimeValue::Null);
        assert_eq!(receiver, RuntimeValue::Heap(handle));
    }

    let HeapNode::Map(map) = heap.get(handle).expect("Map handle should remain live") else {
        panic!("Map.set receiver must remain a map");
    };
    assert_eq!(
        map.get(&RuntimeValueKey::string("present")),
        Some(&RuntimeValue::Heap(new_nested))
    );
    assert_eq!(
        map.get(&RuntimeValueKey::string("inserted")),
        Some(&RuntimeValue::Heap(new_nested))
    );
}

#[test]
fn map_has_and_set_reject_malformed_arity() {
    let mut heap = RequestHeap::default();
    let receiver = RuntimeValue::Heap(
        heap.alloc_map(BTreeMap::new())
            .expect("Map allocation should succeed"),
    );
    for (method, args) in [
        ("has", vec![]),
        (
            "has",
            vec![
                RuntimeValue::String("one".to_string()),
                RuntimeValue::String("two".to_string()),
            ],
        ),
        ("set", vec![RuntimeValue::String("key".to_string())]),
    ] {
        ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(&receiver_op("Map", method), receiver.clone(), args)
            .expect_err("malformed Map receiver call must fail closed");
    }
}

#[test]
fn string_replace_all_receiver_method_dispatches() {
    let mut heap = RequestHeap::default();
    let mut dispatch = ReceiverMethodDispatch::new(&mut heap);

    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("string", "replaceAll"),
                RuntimeValue::String("a-b-a".to_string()),
                vec![
                    RuntimeValue::String("a".to_string()),
                    RuntimeValue::String("z".to_string())
                ],
            )
            .expect("string.replaceAll should dispatch"),
        RuntimeValue::String("z-b-z".to_string())
    );
}

#[test]
fn date_receiver_methods_dispatch() {
    let mut heap = RequestHeap::default();
    let mut dispatch = ReceiverMethodDispatch::new(&mut heap);

    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Date", "toEpochMilliseconds"),
                RuntimeValue::Date(1_000),
                vec![]
            )
            .expect("toEpochMilliseconds should dispatch"),
        runtime_integer_value(1_000)
    );
    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Date", "toISOString"),
                RuntimeValue::Date(1_000),
                vec![]
            )
            .expect("toISOString should dispatch"),
        RuntimeValue::String("1970-01-01T00:00:01.000Z".to_string())
    );
    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Date", "addMilliseconds"),
                RuntimeValue::Date(1_000),
                vec![RuntimeValue::Number(500.0)],
            )
            .expect("addMilliseconds should dispatch"),
        RuntimeValue::Date(1_500)
    );
    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Date", "diffMilliseconds"),
                RuntimeValue::Date(1_500),
                vec![RuntimeValue::Date(1_000)],
            )
            .expect("diffMilliseconds should dispatch"),
        runtime_integer_value(500)
    );
    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Date", "compare"),
                RuntimeValue::Date(1_000),
                vec![RuntimeValue::Date(1_500)],
            )
            .expect("compare should dispatch"),
        runtime_integer_value(-1)
    );
    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Date", "isBefore"),
                RuntimeValue::Date(1_000),
                vec![RuntimeValue::Date(1_500)],
            )
            .expect("isBefore should dispatch"),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Date", "isAfter"),
                RuntimeValue::Date(1_500),
                vec![RuntimeValue::Date(1_000)],
            )
            .expect("isAfter should dispatch"),
        RuntimeValue::Bool(true)
    );
}

#[test]
fn date_add_milliseconds_preserves_range_and_typed_errors() {
    let mut heap = RequestHeap::default();
    let op = receiver_op("Date", "addMilliseconds");

    for (receiver, delta, expected) in [
        (1_000, 500.0, 1_500),
        (1_000, -500.0, 500),
        (
            date_value::MIN_EPOCH_MILLIS,
            0.0,
            date_value::MIN_EPOCH_MILLIS,
        ),
        (
            date_value::MAX_EPOCH_MILLIS,
            0.0,
            date_value::MAX_EPOCH_MILLIS,
        ),
    ] {
        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &op,
                    RuntimeValue::Date(receiver),
                    vec![RuntimeValue::Number(delta)],
                )
                .expect("canonical Date.addMilliseconds should dispatch"),
            RuntimeValue::Date(expected)
        );
    }

    for (receiver, delta) in [
        (date_value::MAX_EPOCH_MILLIS, 1.0),
        (date_value::MIN_EPOCH_MILLIS, -1.0),
    ] {
        let error = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(
                &op,
                RuntimeValue::Date(receiver),
                vec![RuntimeValue::Number(delta)],
            )
            .expect_err("Date.addMilliseconds must reject an out-of-range Date");
        assert!(matches!(
            error,
            RuntimeError::DecodeTarget { ref target, .. }
                if target == "Date.addMilliseconds"
        ));
    }

    for argument in [
        RuntimeValue::Number(0.5),
        RuntimeValue::Number(f64::INFINITY),
        RuntimeValue::Number(9_007_199_254_740_992.0),
        RuntimeValue::String("1".to_string()),
    ] {
        let error = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(&op, RuntimeValue::Date(0), vec![argument])
            .expect_err("Date.addMilliseconds must require an integer argument");
        assert!(matches!(error, RuntimeError::Decode(_)));
    }
}

#[test]
fn duration_receiver_methods_dispatch_erased_milliseconds() {
    let mut heap = RequestHeap::default();
    let mut dispatch = ReceiverMethodDispatch::new(&mut heap);

    assert_eq!(
        dispatch
            .dispatch_op(
                &receiver_op("Duration", "toMilliseconds"),
                RuntimeValue::Number(2_000.0),
                vec![]
            )
            .expect("Duration.toMilliseconds should dispatch"),
        runtime_integer_value(2_000)
    );
}

#[test]
fn number_ceil_preserves_numeric_boundaries_and_rejects_malformed_calls() {
    let mut heap = RequestHeap::default();
    let op = receiver_op("number", "ceil");

    for (input, expected) in [
        (1.25, RuntimeValue::Number(2.0)),
        (-1.25, RuntimeValue::Number(-1.0)),
        (
            9_007_199_254_740_991.0,
            RuntimeValue::Number(9_007_199_254_740_991.0),
        ),
        (f64::MAX, RuntimeValue::Number(f64::MAX)),
        (f64::INFINITY, RuntimeValue::Null),
    ] {
        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(&op, RuntimeValue::Number(input), vec![])
                .expect("canonical number.ceil should dispatch"),
            expected,
            "{input}"
        );
    }

    let wrong_receiver = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(&op, RuntimeValue::String("1.25".to_string()), vec![])
        .expect_err("number.ceil must reject a non-number receiver");
    assert!(matches!(wrong_receiver, RuntimeError::Decode(_)));

    let wrong_arity = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(
            &op,
            RuntimeValue::Number(1.25),
            vec![RuntimeValue::Number(0.0)],
        )
        .expect_err("number.ceil must reject arguments");
    assert!(matches!(wrong_arity, RuntimeError::Decode(_)));
}

#[test]
fn bytes_to_hex_dispatches_exact_receiver_and_rejects_malformed_calls() {
    let mut heap = RequestHeap::default();
    let op = receiver_op("bytes", "toHex");
    let receiver = RuntimeValue::Heap(
        heap.alloc_bytes(vec![0x00, 0x0f, 0xa5, 0xff])
            .expect("bytes fixture should allocate"),
    );

    assert_eq!(
        ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(&op, receiver.clone(), vec![])
            .expect("canonical bytes.toHex should dispatch"),
        RuntimeValue::String("000fa5ff".to_string())
    );

    let wrong_receiver = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(&op, RuntimeValue::String("00".to_string()), vec![])
        .expect_err("bytes.toHex must reject a non-bytes receiver");
    assert!(matches!(wrong_receiver, RuntimeError::Decode(_)));

    let wrong_arity = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(&op, receiver, vec![RuntimeValue::Number(0.0)])
        .expect_err("bytes.toHex must reject arguments");
    assert!(matches!(wrong_arity, RuntimeError::Decode(_)));
}
