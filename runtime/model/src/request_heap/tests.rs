use std::collections::BTreeMap;

use crate::{
    addr::{ExecutableAddr, FileAddr, TypeAddr, UnitAddr},
    error::RuntimeModelError,
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, InternalErrorPayload,
        LocalExecutionTypeIdentity, NominalTypeIdentity, OpaqueServiceError,
        PlatformBuiltinErrorIdentity, RequestException, RequestExceptionCause,
        ServiceErrorEnvelope,
    },
    value::{
        CallbackCapabilityCarrier, HeapHandle, HeapNode, InterfaceCarrier, InterfaceMethodSlot,
        InterfaceMethodTable, InterfaceMethodTarget, InterfaceReceiverCallAbi, InterfaceValue,
        RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueCarrier,
        RuntimeValueKey,
    },
};
use skiff_artifact_model::{InstructionSourceSite, SourcePosition, SourceSpanRef};

use super::{
    deep_clone_runtime_value, deep_clone_runtime_value_between_heaps,
    deep_clone_runtime_value_carrier_between_heaps,
    deep_clone_runtime_value_carriers_between_heaps, deep_clone_runtime_values_between_heaps,
    RequestHeap, RequestHeapLimits, RequestHeapRollbackRebaseError,
};

fn local_identity(type_index: usize) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index,
            },
            type_arguments: Vec::new(),
        },
    ))
}

fn source_site() -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 1,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    }
}

#[test]
fn alloc_array_and_get_roundtrip() {
    let mut heap = RequestHeap::default();

    let handle = heap
        .alloc_array(vec![RuntimeValue::Bool(true), RuntimeValue::from("item")])
        .expect("array should allocate");

    assert_eq!(handle.index(), 0);
    assert_eq!(handle.generation(), 0);
    assert_eq!(heap.stats().node_count, 1);
    assert!(heap.stats().estimated_bytes > 0);
    assert_eq!(
        heap.get(handle).expect("handle should resolve"),
        &HeapNode::Array(vec![RuntimeValue::Bool(true), RuntimeValue::from("item")])
    );
}

#[test]
fn carrier_sidecars_survive_container_projection_mutation_and_cross_heap_clone() {
    let first = local_identity(10);
    let second = local_identity(11);
    let first_value = RuntimeValueCarrier::identified(RuntimeValue::from("first"), first.clone());
    let second_value = RuntimeValueCarrier::identified(RuntimeValue::Number(2.0), second.clone());
    let mut heap = RequestHeap::default();

    let object = heap
        .alloc_object_carriers(BTreeMap::from([("field".to_string(), first_value.clone())]))
        .unwrap();
    let array = heap
        .alloc_array_carriers(vec![first_value.clone()])
        .unwrap();
    let map = heap
        .alloc_map_carriers(BTreeMap::from([(
            RuntimeValueKey::string("key"),
            second_value.clone(),
        )]))
        .unwrap();

    assert_eq!(
        heap.object_field_carrier(object, "field")
            .unwrap()
            .unwrap()
            .catch_identity(),
        Some(&first)
    );
    assert_eq!(
        heap.array_item_carrier(array, 0)
            .unwrap()
            .unwrap()
            .catch_identity(),
        Some(&first)
    );
    assert_eq!(
        heap.map_entry_carrier(map, &RuntimeValueKey::string("key"))
            .unwrap()
            .unwrap()
            .catch_identity(),
        Some(&second)
    );

    heap.set_array_item_carrier(array, 0, second_value.clone())
        .unwrap();
    heap.set_object_field_carrier(object, "field".to_string(), second_value.clone())
        .unwrap();
    heap.set_map_entry_carrier(map, RuntimeValueKey::string("key"), first_value.clone())
        .unwrap();
    assert_eq!(
        heap.pop_array_item_carrier(array).unwrap().catch_identity(),
        Some(&second)
    );

    let root = RuntimeValueCarrier::identified(RuntimeValue::Heap(object), first.clone());
    let mut destination = RequestHeap::default();
    let cloned =
        deep_clone_runtime_value_carrier_between_heaps(&heap, &mut destination, &root).unwrap();
    assert_eq!(cloned.catch_identity(), Some(&first));
    let RuntimeValue::Heap(cloned_object) = cloned.value() else {
        panic!("cloned object must remain a heap value");
    };
    assert_eq!(
        destination
            .object_field_carrier(*cloned_object, "field")
            .unwrap()
            .unwrap()
            .catch_identity(),
        Some(&second)
    );
}

#[test]
fn local_carrier_cell_survives_cross_heap_clone_without_becoming_an_array_projection() {
    let identity = local_identity(12);
    let mut source = RequestHeap::default();
    let cell = source
        .alloc_local_carrier_cell(RuntimeValueCarrier::identified(
            RuntimeValue::from("stream-item"),
            identity.clone(),
        ))
        .unwrap();
    assert_eq!(
        source
            .local_carrier_cell(cell)
            .unwrap()
            .unwrap()
            .catch_identity(),
        Some(&identity)
    );
    assert!(source.array_item_carrier(cell, 0).unwrap().is_none());

    let mut destination = RequestHeap::default();
    let cloned = deep_clone_runtime_value_between_heaps(
        &source,
        &mut destination,
        &RuntimeValue::Heap(cell),
    )
    .unwrap();
    let RuntimeValue::Heap(cloned) = cloned else {
        panic!("carrier cell clone must remain a heap handle");
    };
    assert_eq!(
        destination
            .local_carrier_cell(cloned)
            .unwrap()
            .unwrap()
            .catch_identity(),
        Some(&identity)
    );
}

#[test]
fn exception_heap_clone_preserves_exact_local_cause_and_metadata() {
    let identity = local_identity(20);
    let site = source_site();
    let exception = RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::from("private"), identity.clone()),
        site.clone(),
        vec![ExceptionStackFrame::Local { site: site.clone() }],
        ErrorCorrelation {
            trace_id: "trace-1".to_string(),
            error_id: "error-1".to_string(),
        },
    )
    .unwrap();
    let mut source = RequestHeap::default();
    let handle = source.alloc_exception(exception.clone()).unwrap();
    let mut destination = RequestHeap::default();
    let cloned = deep_clone_runtime_value_between_heaps(
        &source,
        &mut destination,
        &RuntimeValue::Heap(handle),
    )
    .unwrap();
    let RuntimeValue::Heap(handle) = cloned else {
        panic!("cloned exception must remain a heap value");
    };
    let HeapNode::Exception(cloned) = destination.get(handle).unwrap() else {
        panic!("cloned node must remain an Exception");
    };
    assert_eq!(cloned, &exception);
    assert_eq!(cloned.local_catch_identity(), Some(&identity));
}

#[test]
fn imported_exception_heap_edges_are_reachable_and_remapped_by_both_clone_modes() {
    let identity = local_identity(21);
    let envelope = ServiceErrorEnvelope::PlatformError {
        builtin_error_identity: PlatformBuiltinErrorIdentity::DbConflict,
        encoded_payload: br#"{"retryable":true}"#.to_vec(),
        trace_id: "trace-imported".to_string(),
        error_id: "error-imported".to_string(),
    };
    let opaque = OpaqueServiceError::decode(
        serde_json::to_vec(&envelope).expect("test service envelope should encode"),
    )
    .expect("test service envelope should decode strictly");
    let mut source = RequestHeap::default();
    let payload = source
        .alloc_array(vec![RuntimeValue::String("imported-local".to_string())])
        .expect("imported local payload should allocate");
    let imported = RequestException::imported(
        opaque.clone(),
        Some(RuntimeValueCarrier::identified(
            RuntimeValue::Heap(payload),
            identity.clone(),
        )),
        source_site(),
        Vec::new(),
    )
    .expect("imported exception should retain its exact local projection");
    let exception = source
        .alloc_exception(imported)
        .expect("imported exception node should allocate");
    assert!(
        source.is_reachable(exception, payload).unwrap(),
        "an imported local projection is a real exception-graph edge"
    );
    assert!(
        !source
            .can_insert_without_cycle(payload, &RuntimeValue::Heap(exception))
            .unwrap(),
        "cycle checks must traverse an imported local projection"
    );

    let same_heap = deep_clone_runtime_value(&mut source, &RuntimeValue::Heap(exception))
        .expect("same-heap clone should preserve imported local projection");
    let RuntimeValue::Heap(same_heap) = same_heap else {
        panic!("same-heap clone should remain heap-backed")
    };
    let same_heap_payload =
        assert_imported_exception_payload(&source, same_heap, &identity, &opaque);
    assert_ne!(
        same_heap_payload, payload,
        "same-heap deep clone must not retain the original imported payload handle"
    );

    let mut destination = RequestHeap::default();
    let collision = destination
        .alloc_array(vec![RuntimeValue::String(
            "destination-collision".to_string(),
        )])
        .expect("destination collision sentinel should allocate");
    assert_eq!(collision, payload, "test requires a valid handle collision");
    let cross_heap = deep_clone_runtime_value_between_heaps(
        &source,
        &mut destination,
        &RuntimeValue::Heap(exception),
    )
    .expect("cross-heap clone should remap imported local projection");
    let RuntimeValue::Heap(cross_heap) = cross_heap else {
        panic!("cross-heap clone should remain heap-backed")
    };
    let cross_heap_payload =
        assert_imported_exception_payload(&destination, cross_heap, &identity, &opaque);
    assert_ne!(
        cross_heap_payload, collision,
        "cross-heap clone must not retain a colliding source handle"
    );
    assert!(matches!(
        destination.get(collision),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("destination-collision".to_string())]
    ));
}

fn assert_imported_exception_payload(
    heap: &RequestHeap,
    exception: crate::value::HeapHandle,
    expected_identity: &CatchIdentity,
    expected_opaque: &OpaqueServiceError,
) -> HeapHandle {
    let HeapNode::Exception(exception) = heap
        .get(exception)
        .expect("cloned exception handle should resolve")
    else {
        panic!("cloned node should remain an exception")
    };
    let RequestExceptionCause::OpaqueService {
        error,
        local_value: Some(local_value),
    } = exception.cause()
    else {
        panic!("cloned exception should preserve its imported cause")
    };
    assert_eq!(error, expected_opaque);
    assert_eq!(local_value.catch_identity(), Some(expected_identity));
    let RuntimeValue::Heap(payload) = local_value.value() else {
        panic!("cloned imported projection should remain heap-backed")
    };
    assert!(matches!(
        heap.get(*payload),
        Ok(HeapNode::Array(items))
            if items == &[RuntimeValue::String("imported-local".to_string())]
    ));
    *payload
}

#[test]
fn alloc_bytes_stores_request_local_heap_node() {
    let mut heap = RequestHeap::default();

    let handle = heap
        .alloc_bytes(b"hello".as_slice())
        .expect("bytes should allocate");

    assert_eq!(heap.stats().node_count, 1);
    let HeapNode::Bytes(bytes) = heap.get(handle).expect("bytes should resolve") else {
        panic!("expected bytes node");
    };
    assert_eq!(bytes.as_slice(), b"hello");
}

#[test]
fn alloc_interface_stores_wrapper_and_tracks_estimated_bytes() {
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_interface(test_interface(RuntimeValue::from("payload")))
        .expect("interface should allocate");

    assert_eq!(heap.stats().node_count, 1);
    assert!(heap.stats().estimated_bytes > 0);
    assert_eq!(heap.recompute_stats(), heap.stats());
    let HeapNode::Interface(value) = heap.get(handle).expect("interface should resolve") else {
        panic!("expected interface node");
    };
    assert_eq!(value.interface(), "pkg.Reader");
    assert_eq!(value.diagnostic_label(), "any interface pkg.Reader (local)");
}

#[test]
fn callback_capability_cross_heap_clone_keeps_only_opaque_route() {
    let mut source = RequestHeap::default();
    let source_handle = source
        .alloc_interface(InterfaceValue::new(
            "contract:reader".to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-a",
                "activation-a",
                7,
                "contract:reader",
                "capability-1",
            )),
        ))
        .expect("callback capability should allocate");
    assert_eq!(source.recompute_stats(), source.stats());

    let same_heap = deep_clone_runtime_value(&mut source, &RuntimeValue::Heap(source_handle))
        .expect("same-heap clone should retain only the opaque callback route");
    let RuntimeValue::Heap(same_heap_handle) = same_heap else {
        panic!("same-heap callback clone should remain a heap value");
    };
    assert_ne!(same_heap_handle, source_handle);
    let HeapNode::Interface(same_heap_value) = source.get(same_heap_handle).unwrap() else {
        panic!("same-heap callback clone should remain an interface wrapper");
    };
    assert!(matches!(
        same_heap_value.carrier(),
        InterfaceCarrier::CallbackCapability(_)
    ));

    let mut destination = RequestHeap::default();
    destination
        .alloc_array(Vec::new())
        .expect("destination fixture should reserve a distinct local handle");
    let cloned = deep_clone_runtime_value_between_heaps(
        &source,
        &mut destination,
        &RuntimeValue::Heap(source_handle),
    )
    .expect("opaque callback route should clone without rebuilding behavior");
    let RuntimeValue::Heap(cloned_handle) = cloned else {
        panic!("callback wrapper should remain a heap value");
    };
    assert_ne!(source_handle, cloned_handle);
    let HeapNode::Interface(cloned) = destination
        .get(cloned_handle)
        .expect("cloned callback should resolve")
    else {
        panic!("cloned callback should remain an interface wrapper");
    };
    let InterfaceCarrier::CallbackCapability(carrier) = cloned.carrier() else {
        panic!("callback clone must not become local or remote behavior");
    };
    assert_eq!(carrier.owner_runtime_replica_id(), "runtime-a");
    assert_eq!(carrier.owner_activation_id(), "activation-a");
    assert_eq!(carrier.request_generation(), 7);
    assert_eq!(carrier.opaque_capability_id(), "capability-1");
    assert_eq!(destination.recompute_stats(), destination.stats());
}

#[test]
fn array_mutation_keeps_estimated_bytes_stats_current() {
    let mut heap = RequestHeap::default();
    let handle = heap.alloc_array(Vec::new()).expect("array should allocate");
    let before = heap.stats().estimated_bytes;

    heap.push_array_item(handle, RuntimeValue::String("larger item".to_string()))
        .expect("push should pass");

    assert!(heap.stats().estimated_bytes > before);

    let recomputed = heap.recompute_stats();
    assert_eq!(recomputed, heap.stats());
}

#[test]
fn get_rejects_out_of_bounds_handle() {
    let heap = RequestHeap::default();
    let error = heap.get(HeapHandle::new(10, 0)).unwrap_err();

    assert!(
        error.to_string().contains("index is out of bounds"),
        "unexpected error: {error}"
    );
}

#[test]
fn get_rejects_stale_generation() {
    let mut heap = RequestHeap::default();
    let handle = heap.alloc_array(Vec::new()).expect("array should allocate");
    let stale = HeapHandle::new(handle.index(), handle.generation() + 1);

    let error = heap.get(stale).unwrap_err();

    assert!(
        error.to_string().contains("generation does not match"),
        "unexpected error: {error}"
    );
}

#[test]
fn new_with_epoch_stamps_allocated_handles() {
    let mut heap = RequestHeap::new_with_epoch(7, RequestHeapLimits::default());
    assert_eq!(heap.epoch(), 7);

    let handle = heap
        .alloc_array(vec![RuntimeValue::from("epoch-stamped")])
        .expect("array should allocate");

    assert_eq!(handle.epoch(), 7);
    assert_eq!(handle.index(), 0);
    assert_eq!(handle.generation(), 0);
    assert_eq!(
        heap.get(handle)
            .expect("epoch-stamped handle should resolve"),
        &HeapNode::Array(vec![RuntimeValue::from("epoch-stamped")])
    );
}

#[test]
fn get_rejects_stale_epoch_handle() {
    let mut heap = RequestHeap::new_with_epoch(3, RequestHeapLimits::default());
    let handle = heap.alloc_array(Vec::new()).expect("array should allocate");

    let stale = HeapHandle::new_with_epoch(handle.index(), handle.generation(), 0);
    let error = heap.get(stale).unwrap_err();
    assert!(
        error.to_string().contains("epoch does not match heap slot"),
        "unexpected error: {error}"
    );

    let stale = HeapHandle::new_with_epoch(handle.index(), handle.generation(), 4);
    let error = heap.get(stale).unwrap_err();
    assert!(
        error.to_string().contains("epoch does not match heap slot"),
        "unexpected error: {error}"
    );

    assert!(heap.get(handle).is_ok());
}

#[test]
fn mutation_rejects_stale_epoch_handle() {
    let mut heap = RequestHeap::new_with_epoch(2, RequestHeapLimits::default());
    let handle = heap.alloc_array(Vec::new()).expect("array should allocate");

    let stale = HeapHandle::new_with_epoch(handle.index(), handle.generation(), 1);
    assert!(matches!(
        heap.push_array_item(stale, RuntimeValue::from("late")),
        Err(RuntimeModelError::Decode(_))
    ));
    assert_eq!(heap.stats().node_count, 1);
}

#[test]
fn rollback_rebase_preserves_heap_epoch() {
    let mut heap = RequestHeap::new_with_epoch(5, RequestHeapLimits::default());
    let inner = heap
        .alloc_array(vec![RuntimeValue::from("retained")])
        .expect("inner prefix node should allocate");
    let prefix = heap
        .alloc_array(vec![RuntimeValue::Heap(inner)])
        .expect("prefix should allocate");
    let checkpoint = heap.checkpoint();
    let discarded = heap
        .alloc_array(vec![RuntimeValue::from("discarded")])
        .expect("discarded suffix should allocate");

    let prepared = heap
        .prepare_rollback_rebase(checkpoint, &[RuntimeValue::Heap(prefix)])
        .expect("rollback rebase should prepare");
    heap.commit_prepared_rollback_rebase(prepared);

    assert_eq!(heap.epoch(), 5);
    assert_eq!(
        heap.get(prefix).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::Heap(inner)])
    );
    assert!(heap.get(discarded).is_err());
    let HeapNode::Array(prefix_items) = heap.get(prefix).unwrap() else {
        panic!("prefix must remain an array");
    };
    let RuntimeValue::Heap(retained) = prefix_items[0] else {
        panic!("prefix array item is a heap handle");
    };
    assert_eq!(retained.epoch(), 5);
    assert!(heap.get(retained).is_ok());
}

#[test]
fn cross_heap_clone_stamps_destination_epoch() {
    let mut source = RequestHeap::new_with_epoch(1, RequestHeapLimits::default());
    let root = source
        .alloc_array(vec![RuntimeValue::from("source")])
        .expect("source root should allocate");

    let mut dest = RequestHeap::new_with_epoch(2, RequestHeapLimits::default());
    let cloned =
        deep_clone_runtime_value_between_heaps(&source, &mut dest, &RuntimeValue::Heap(root))
            .expect("cross-heap clone should succeed");
    let RuntimeValue::Heap(cloned_handle) = cloned else {
        panic!("cloned root must remain a heap value")
    };

    assert_eq!(cloned_handle.epoch(), 2);
    assert_eq!(dest.epoch(), 2);
    assert!(dest.get(cloned_handle).is_ok());
    assert!(
        dest.get(root).is_err(),
        "source-epoch handle must fail closed"
    );
}

#[test]
fn alloc_map_uses_plain_string_key_identity() {
    let mut heap = RequestHeap::default();
    let mut map = BTreeMap::new();
    map.insert(RuntimeValueKey::string("u1"), RuntimeValue::Number(3.0));
    map.insert(RuntimeValueKey::string("u1"), RuntimeValue::Number(1.0));

    let handle = heap.alloc_map(map).expect("map should allocate");
    let HeapNode::Map(map) = heap.get(handle).expect("handle should resolve") else {
        panic!("expected map node");
    };

    assert_eq!(
        map.keys().cloned().collect::<Vec<_>>(),
        vec![RuntimeValueKey::string("u1")]
    );
    assert_eq!(
        map.get(&RuntimeValueKey::string("u1")),
        Some(&RuntimeValue::Number(1.0))
    );
}

#[test]
fn max_nodes_limit_rejects_allocation_without_mutating_stats() {
    let limits = RequestHeapLimits {
        max_nodes: 1,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    heap.alloc_array(Vec::new())
        .expect("first node should allocate");

    let before = heap.stats();
    let error = heap.alloc_array(Vec::new()).unwrap_err();

    let RuntimeModelError::ResourceLimitExceeded {
        resource,
        reason,
        limit,
        current,
        requested_delta,
    } = error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(resource, "requestHeap");
    assert_eq!(reason, "max heap nodes");
    assert_eq!(limit, 1);
    assert_eq!(current, 1);
    assert_eq!(requested_delta, 1);
    assert_eq!(heap.stats(), before);
    assert_eq!(heap.len(), 1);
}

#[test]
fn array_push_rejects_estimated_bytes_limit_without_mutating_node() {
    let limits = RequestHeapLimits {
        max_estimated_bytes: 48,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let handle = heap.alloc_array(Vec::new()).expect("array should allocate");

    let before = heap.stats();
    let error = heap
        .push_array_item(
            handle,
            RuntimeValue::String("this string is too large".to_string()),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        RuntimeModelError::ResourceLimitExceeded { .. }
    ));
    assert_eq!(heap.stats(), before);
    assert_eq!(
        heap.get(handle).expect("handle should resolve"),
        &HeapNode::Array(Vec::new())
    );
}

#[test]
fn array_set_rejects_estimated_bytes_limit_without_partial_mutation() {
    let limits = RequestHeapLimits {
        max_estimated_bytes: 48,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let handle = heap
        .alloc_array(vec![RuntimeValue::String("x".to_string())])
        .expect("array should allocate");

    let before = heap.stats();
    let error = heap
        .set_array_item(
            handle,
            0,
            RuntimeValue::String("this string is too large".to_string()),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        RuntimeModelError::ResourceLimitExceeded { .. }
    ));
    assert_eq!(heap.stats(), before);
    assert_eq!(
        heap.get(handle).expect("handle should resolve"),
        &HeapNode::Array(vec![RuntimeValue::String("x".to_string())])
    );
}

#[test]
fn map_set_and_object_field_set_reject_estimated_bytes_limit_without_partial_mutation() {
    let limits = RequestHeapLimits {
        max_estimated_bytes: 128,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let mut map = BTreeMap::new();
    map.insert(
        RuntimeValueKey::string("k"),
        RuntimeValue::String("v".to_string()),
    );
    let map_handle = heap.alloc_map(map).expect("map should allocate");
    let mut object_fields = RuntimeObjectFields::new();
    object_fields.insert("name".to_string(), RuntimeValue::String("a".to_string()));
    let object_handle = heap
        .alloc_object(RuntimeObject::unshaped(object_fields))
        .expect("object should allocate");

    let before = heap.stats();
    let map_error = heap
        .set_map_entry(
            map_handle,
            RuntimeValueKey::string("k"),
            RuntimeValue::String("this string is too large".to_string()),
        )
        .unwrap_err();
    assert!(matches!(
        map_error,
        RuntimeModelError::ResourceLimitExceeded { .. }
    ));
    assert_eq!(heap.stats(), before);
    let HeapNode::Map(map) = heap.get(map_handle).expect("map should resolve") else {
        panic!("expected map");
    };
    assert_eq!(
        map.get(&RuntimeValueKey::string("k")),
        Some(&RuntimeValue::String("v".to_string()))
    );

    let object_error = heap
        .set_object_field(
            object_handle,
            "name".to_string(),
            RuntimeValue::String("this string is too large".to_string()),
        )
        .unwrap_err();
    assert!(matches!(
        object_error,
        RuntimeModelError::ResourceLimitExceeded { .. }
    ));
    assert_eq!(heap.stats(), before);
    let HeapNode::Object(object) = heap.get(object_handle).expect("object should resolve") else {
        panic!("expected object");
    };
    assert_eq!(
        object.fields().get("name"),
        Some(&RuntimeValue::String("a".to_string()))
    );
}

#[test]
fn cycle_check_rejects_inserting_ancestor_into_descendant() {
    let mut heap = RequestHeap::default();
    let child = heap.alloc_array(Vec::new()).expect("child should allocate");
    let parent = heap
        .alloc_array(vec![RuntimeValue::Heap(child)])
        .expect("parent should allocate");

    assert!(!heap
        .can_insert_without_cycle(child, &RuntimeValue::Heap(parent))
        .expect("cycle check should evaluate"));
    assert!(heap
        .check_insert_without_cycle(child, &RuntimeValue::Heap(parent))
        .unwrap_err()
        .to_string()
        .contains("cycle"));
    assert!(heap
        .can_insert_without_cycle(parent, &RuntimeValue::from("ok"))
        .expect("scalar insert should pass"));
}

#[test]
fn deep_clone_preserves_aliases_inside_cloned_graph() {
    let mut heap = RequestHeap::default();
    let shared = heap
        .alloc_array(vec![RuntimeValue::from("shared")])
        .expect("shared should allocate");
    let root = heap
        .alloc_array(vec![RuntimeValue::Heap(shared), RuntimeValue::Heap(shared)])
        .expect("root should allocate");

    let cloned = deep_clone_runtime_value(&mut heap, &RuntimeValue::Heap(root))
        .expect("clone should succeed");
    let RuntimeValue::Heap(cloned_root) = cloned else {
        panic!("expected heap clone");
    };
    assert_ne!(cloned_root, root);

    let HeapNode::Array(items) = heap.get(cloned_root).expect("clone should resolve") else {
        panic!("expected cloned array");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].as_heap_handle(), items[1].as_heap_handle());
    assert_ne!(items[0].as_heap_handle(), Some(shared));
    assert!(heap.stats().clone_depth >= 1);
}

#[test]
fn deep_clone_bytes_clones_heap_handle_without_copying_buffer() {
    let mut heap = RequestHeap::default();
    let original = heap
        .alloc_bytes(vec![1, 2, 3, 4])
        .expect("bytes should allocate");
    let original_ptr = match heap.get(original).expect("bytes should resolve") {
        HeapNode::Bytes(bytes) => bytes.as_slice().as_ptr(),
        _ => panic!("expected bytes node"),
    };

    let cloned = deep_clone_runtime_value(&mut heap, &RuntimeValue::Heap(original))
        .expect("clone should succeed");
    let RuntimeValue::Heap(cloned) = cloned else {
        panic!("expected heap clone");
    };
    let cloned_ptr = match heap.get(cloned).expect("clone should resolve") {
        HeapNode::Bytes(bytes) => bytes.as_slice().as_ptr(),
        _ => panic!("expected bytes node"),
    };

    assert_ne!(cloned, original);
    assert_eq!(cloned_ptr, original_ptr);
}

#[test]
fn deep_clone_clones_nested_array_payload() {
    let mut heap = RequestHeap::default();
    let payload_handle = heap
        .alloc_array(vec![RuntimeValue::from("item")])
        .expect("payload should allocate");

    let cloned = deep_clone_runtime_value(&mut heap, &RuntimeValue::Heap(payload_handle))
        .expect("clone should succeed");
    let RuntimeValue::Heap(cloned_payload) = cloned else {
        panic!("expected heap clone");
    };

    assert_ne!(cloned_payload, payload_handle);
    assert_eq!(
        heap.get(cloned_payload).expect("clone should resolve"),
        &HeapNode::Array(vec![RuntimeValue::from("item")])
    );
}

#[test]
fn deep_clone_interface_preserves_wrapper_and_clones_local_payload() {
    let mut heap = RequestHeap::default();
    let original_payload = heap
        .alloc_array(vec![RuntimeValue::from("item")])
        .expect("payload should allocate");
    let interface = heap
        .alloc_interface(test_interface(RuntimeValue::Heap(original_payload)))
        .expect("interface should allocate");

    let cloned = deep_clone_runtime_value(&mut heap, &RuntimeValue::Heap(interface))
        .expect("interface clone should succeed");
    let RuntimeValue::Heap(cloned_interface) = cloned else {
        panic!("expected heap clone");
    };
    assert_ne!(cloned_interface, interface);

    let HeapNode::Interface(value) = heap
        .get(cloned_interface)
        .expect("cloned interface should resolve")
    else {
        panic!("expected interface clone");
    };
    let InterfaceCarrier::Local { payload, .. } = value.carrier() else {
        panic!("expected local interface carrier");
    };
    let cloned_payload = payload
        .as_heap_handle()
        .expect("interface payload should remain heap-backed");
    assert_ne!(cloned_payload, original_payload);
    assert_eq!(
        heap.get(cloned_payload)
            .expect("cloned payload should resolve"),
        &HeapNode::Array(vec![RuntimeValue::from("item")])
    );
}

#[test]
fn deep_clone_between_heaps_clones_nested_object_graph() {
    let mut source = RequestHeap::default();
    let mut dest = RequestHeap::default();
    let numbers = source
        .alloc_array(vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)])
        .expect("array should allocate");
    let label = RuntimeValue::from("primary");
    let mut fields = RuntimeObjectFields::new();
    fields.insert("numbers".to_string(), RuntimeValue::Heap(numbers));
    fields.insert("label".to_string(), label.clone());
    let root = source
        .alloc_object(RuntimeObject::unshaped(fields))
        .expect("root should allocate");

    let cloned =
        deep_clone_runtime_value_between_heaps(&source, &mut dest, &RuntimeValue::Heap(root))
            .expect("cross heap clone should succeed");
    let RuntimeValue::Heap(cloned_root) = cloned else {
        panic!("expected heap clone");
    };

    let HeapNode::Object(object) = dest.get(cloned_root).expect("clone should resolve") else {
        panic!("expected object clone");
    };
    let cloned_numbers = object
        .fields()
        .get("numbers")
        .and_then(RuntimeValue::as_heap_handle)
        .expect("numbers should be heap value");
    let cloned_label = object
        .fields()
        .get("label")
        .expect("label should be present");

    assert_eq!(
        dest.get(cloned_numbers).expect("numbers should resolve"),
        &HeapNode::Array(vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)])
    );
    assert_eq!(cloned_label, &RuntimeValue::String("primary".to_string()));
    assert_eq!(
        source.get(root).expect("source root should remain intact"),
        &HeapNode::Object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("label".to_string(), label),
            ("numbers".to_string(), RuntimeValue::Heap(numbers)),
        ])))
    );
}

#[test]
fn deep_clone_between_heaps_interface_preserves_wrapper_and_clones_payload() {
    let mut source = RequestHeap::default();
    let mut dest = RequestHeap::default();
    let payload = source
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "name".to_string(),
            RuntimeValue::from("Ada"),
        )])))
        .expect("payload should allocate");
    let interface = source
        .alloc_interface(test_interface(RuntimeValue::Heap(payload)))
        .expect("interface should allocate");

    let cloned =
        deep_clone_runtime_value_between_heaps(&source, &mut dest, &RuntimeValue::Heap(interface))
            .expect("cross heap interface clone should succeed");
    let RuntimeValue::Heap(cloned_interface) = cloned else {
        panic!("expected heap clone");
    };
    let HeapNode::Interface(value) = dest
        .get(cloned_interface)
        .expect("cloned interface should resolve")
    else {
        panic!("expected interface clone");
    };
    let InterfaceCarrier::Local { payload, .. } = value.carrier() else {
        panic!("expected local interface carrier");
    };
    let cloned_payload = payload
        .as_heap_handle()
        .expect("interface payload should remain heap-backed");
    let HeapNode::Object(object) = dest.get(cloned_payload).expect("payload should resolve") else {
        panic!("expected cloned object payload");
    };
    assert_eq!(
        object.fields().get("name"),
        Some(&RuntimeValue::from("Ada"))
    );
}

#[test]
fn deep_clone_between_heaps_preserves_shared_references() {
    let mut source = RequestHeap::default();
    let mut dest = RequestHeap::default();
    let shared = source
        .alloc_map(RuntimeMap::from([(
            RuntimeValueKey::string("id"),
            RuntimeValue::from("shared"),
        )]))
        .expect("shared map should allocate");
    let root = source
        .alloc_array(vec![RuntimeValue::Heap(shared), RuntimeValue::Heap(shared)])
        .expect("root should allocate");

    let cloned =
        deep_clone_runtime_value_between_heaps(&source, &mut dest, &RuntimeValue::Heap(root))
            .expect("cross heap clone should succeed");
    let RuntimeValue::Heap(cloned_root) = cloned else {
        panic!("expected heap clone");
    };
    let HeapNode::Array(items) = dest.get(cloned_root).expect("clone should resolve") else {
        panic!("expected array clone");
    };

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].as_heap_handle(), items[1].as_heap_handle());
    let cloned_shared = items[0]
        .as_heap_handle()
        .expect("cloned shared map should be heap-backed");
    assert_eq!(
        dest.get(cloned_shared)
            .expect("cloned shared map should resolve in dest"),
        &HeapNode::Map(RuntimeMap::from([(
            RuntimeValueKey::string("id"),
            RuntimeValue::from("shared"),
        )]))
    );
    assert_eq!(
        source
            .get(shared)
            .expect("source shared map should remain intact"),
        &HeapNode::Map(RuntimeMap::from([(
            RuntimeValueKey::string("id"),
            RuntimeValue::from("shared"),
        )]))
    );
}

#[test]
fn cross_heap_multi_root_clone_preserves_aliases_despite_handle_collision() {
    let mut source = RequestHeap::default();
    let shared = source
        .alloc_map(RuntimeMap::from([(
            RuntimeValueKey::string("id"),
            RuntimeValue::from("source"),
        )]))
        .expect("shared source map");
    let roots = vec![
        RuntimeValue::Heap(shared).into(),
        RuntimeValue::Heap(shared).into(),
    ];

    let mut destination = RequestHeap::default();
    let collision = destination
        .alloc_array(vec![RuntimeValue::from("destination")])
        .expect("destination collision node");
    assert_eq!(
        collision.index(),
        shared.index(),
        "the fixture must collide on raw handle index"
    );

    let cloned = deep_clone_runtime_value_carriers_between_heaps(&source, &mut destination, &roots)
        .expect("multi-root clone");
    let first = cloned[0]
        .value()
        .as_heap_handle()
        .expect("first cloned root");
    let second = cloned[1]
        .value()
        .as_heap_handle()
        .expect("second cloned root");

    assert_eq!(first, second, "root-to-root alias must remain shared");
    assert_ne!(
        first, collision,
        "source handle must not resolve by raw collision"
    );
    assert_eq!(
        destination.get(collision).expect("collision remains valid"),
        &HeapNode::Array(vec![RuntimeValue::from("destination")])
    );
    assert_eq!(
        destination.get(first).expect("cloned source map"),
        &HeapNode::Map(RuntimeMap::from([(
            RuntimeValueKey::string("id"),
            RuntimeValue::from("source"),
        )]))
    );
}

#[test]
fn cross_heap_multi_root_clone_rolls_back_all_roots_on_limit_error() {
    let mut source = RequestHeap::default();
    let first = source.alloc_array(Vec::new()).expect("first source root");
    let second = source.alloc_array(Vec::new()).expect("second source root");
    let roots = vec![
        RuntimeValue::Heap(first).into(),
        RuntimeValue::Heap(second).into(),
    ];
    let limits = RequestHeapLimits {
        max_nodes: 2,
        ..RequestHeapLimits::default()
    };
    let mut destination = RequestHeap::new(limits);
    let collision = destination
        .alloc_array(vec![RuntimeValue::from("keep")])
        .expect("preexisting destination node");
    let checkpoint = destination.stats();

    let error = deep_clone_runtime_value_carriers_between_heaps(&source, &mut destination, &roots)
        .expect_err("second root must exceed destination node limit");

    assert!(matches!(
        error,
        RuntimeModelError::ResourceLimitExceeded { reason, .. } if reason == "max heap nodes"
    ));
    assert_eq!(destination.stats(), checkpoint);
    assert_eq!(destination.len(), 1);
    assert_eq!(
        destination
            .get(collision)
            .expect("preexisting node survives rollback"),
        &HeapNode::Array(vec![RuntimeValue::from("keep")])
    );
}

#[test]
fn multi_root_cross_heap_clone_preserves_aliases_and_excludes_dead_nodes() {
    let mut source = RequestHeap::default();
    let shared = source
        .alloc_array(vec![RuntimeValue::from("shared")])
        .expect("shared value should allocate");
    let left = source
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "shared".to_string(),
            RuntimeValue::Heap(shared),
        )])))
        .unwrap();
    let right = source
        .alloc_array(vec![RuntimeValue::Heap(shared)])
        .unwrap();
    source
        .alloc_array(vec![RuntimeValue::from("dead")])
        .expect("unreachable value should allocate");
    let mut dest = RequestHeap::new(source.limits().clone());

    let cloned = deep_clone_runtime_values_between_heaps(
        &source,
        &mut dest,
        &[RuntimeValue::Heap(left), RuntimeValue::Heap(right)],
    )
    .expect("multi-root graph clone should succeed");

    let left = cloned[0].as_heap_handle().unwrap();
    let right = cloned[1].as_heap_handle().unwrap();
    assert_ne!(left, right, "the field roots remain distinct");
    let HeapNode::Object(left) = dest.get(left).unwrap() else {
        panic!("left root must remain an object");
    };
    let HeapNode::Array(right) = dest.get(right).unwrap() else {
        panic!("right root must remain an array");
    };
    assert_eq!(
        left.fields()["shared"].as_heap_handle(),
        right[0].as_heap_handle(),
        "an alias shared by distinct roots must remain shared"
    );
    assert_eq!(dest.len(), 3, "only the three reachable nodes are copied");
    assert_eq!(dest.limits(), source.limits());
}

#[test]
fn multi_root_cross_heap_clone_rejects_a_corrupt_cycle() {
    let mut source = RequestHeap::default();
    let root = source
        .alloc_array(Vec::new())
        .expect("root should allocate");
    source
        .push_array_item_without_cycle_check_for_test(root, RuntimeValue::Heap(root))
        .expect("cycle fixture should update");
    let mut dest = RequestHeap::new(source.limits().clone());

    let error = deep_clone_runtime_values_between_heaps(
        &source,
        &mut dest,
        &[RuntimeValue::Heap(root), RuntimeValue::Heap(root)],
    )
    .expect_err("persistent graph copy must fail closed on a corrupt cycle");

    assert!(error.to_string().contains("cyclic heap graph"));
}

#[test]
fn multi_root_cross_heap_clone_rolls_back_all_roots_and_stats_on_failure() {
    let mut source = RequestHeap::default();
    let valid = source
        .alloc_array(vec![RuntimeValue::from("reachable")])
        .unwrap();
    let invalid = HeapHandle::new(u32::MAX, 0);
    let limits = RequestHeapLimits {
        max_nodes: 2,
        ..RequestHeapLimits::default()
    };
    let mut dest = RequestHeap::new(limits);
    let sentinel = dest
        .alloc_array(vec![RuntimeValue::from("existing")])
        .unwrap();
    let before_len = dest.len();
    let before_stats = dest.stats();

    deep_clone_runtime_values_between_heaps(
        &source,
        &mut dest,
        &[RuntimeValue::Heap(valid), RuntimeValue::Heap(invalid)],
    )
    .expect_err("a later invalid root must roll back an earlier cloned root");

    assert_eq!(dest.len(), before_len);
    assert_eq!(dest.stats(), before_stats);
    assert_eq!(
        dest.get(sentinel).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::from("existing")])
    );

    let too_large = source.alloc_array(Vec::new()).unwrap();
    let error = deep_clone_runtime_values_between_heaps(
        &source,
        &mut dest,
        &[RuntimeValue::Heap(valid), RuntimeValue::Heap(too_large)],
    )
    .expect_err("destination node limit must roll back the whole multi-root clone");
    assert!(matches!(
        error,
        RuntimeModelError::ResourceLimitExceeded { reason, .. } if reason == "max heap nodes"
    ));
    assert_eq!(dest.len(), before_len);
    assert_eq!(dest.stats(), before_stats);
    assert_eq!(
        dest.get(sentinel).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::from("existing")])
    );
}

#[test]
fn deep_clone_rejects_cycle() {
    let mut heap = RequestHeap::default();
    let root = heap.alloc_array(Vec::new()).expect("root should allocate");
    heap.push_array_item_without_cycle_check_for_test(root, RuntimeValue::Heap(root))
        .expect("cycle setup should update");

    let error = deep_clone_runtime_value(&mut heap, &RuntimeValue::Heap(root)).unwrap_err();
    assert!(error.to_string().contains("cyclic heap graph"));
}

#[test]
fn rollback_rebase_pins_prefix_and_compacts_reachable_suffix_without_handle_collision() {
    let mut heap = RequestHeap::default();
    let prefix = heap.alloc_array(Vec::new()).unwrap();
    let checkpoint = heap.checkpoint();
    let discarded = heap
        .alloc_array(vec![RuntimeValue::from("discarded")])
        .unwrap();
    let retained = heap
        .alloc_array(vec![RuntimeValue::from("retained")])
        .unwrap();
    heap.push_array_item(prefix, RuntimeValue::Heap(retained))
        .unwrap();
    let before_prepare = heap.clone();

    let prepared = heap.prepare_rollback_rebase(checkpoint, &[]).unwrap();
    assert_eq!(heap.stats(), before_prepare.stats());
    assert_eq!(heap.len(), before_prepare.len());
    assert_eq!(
        heap.get(prefix).unwrap(),
        before_prepare.get(prefix).unwrap()
    );

    let roots = prepared.rebased_roots().to_vec();
    heap.commit_prepared_rollback_rebase(prepared);
    assert!(roots.is_empty());
    assert_eq!(heap.len(), 2);
    assert_eq!(prefix.index(), 0);
    assert_eq!(prefix.generation(), 0);
    let HeapNode::Array(prefix_items) = heap.get(prefix).unwrap() else {
        panic!("prefix must remain an array");
    };
    let rebased_retained = prefix_items[0].as_heap_handle().unwrap();
    assert_eq!(rebased_retained.index(), discarded.index());
    assert_ne!(rebased_retained, discarded);
    assert!(heap.get(discarded).is_err());
    assert_eq!(
        heap.get(rebased_retained).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::from("retained")])
    );
    let stats = heap.stats();
    assert_eq!(stats.node_count, 2);
    assert_eq!(heap.recompute_stats(), stats);
}

#[test]
fn rollback_rebase_preserves_aliases_across_prefix_and_multiple_explicit_roots() {
    let mut heap = RequestHeap::default();
    let prefix = heap.alloc_array(Vec::new()).unwrap();
    let checkpoint = heap.checkpoint();
    let shared = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "name".to_string(),
            RuntimeValue::from("shared"),
        )])))
        .unwrap();
    let array_root = heap
        .alloc_array(vec![RuntimeValue::Heap(shared), RuntimeValue::Heap(shared)])
        .unwrap();
    let opaque = OpaqueServiceError::decode(
        serde_json::to_vec(&ServiceErrorEnvelope::InternalError {
            payload: InternalErrorPayload {
                message: "sanitized".to_string(),
                trace_id: "trace-rebase".to_string(),
                error_id: "error-rebase".to_string(),
            },
        })
        .unwrap(),
    )
    .unwrap();
    let exception = RequestException::imported(
        opaque,
        Some(RuntimeValueCarrier::identified(
            RuntimeValue::Heap(shared),
            local_identity(31),
        )),
        source_site(),
        Vec::new(),
    )
    .unwrap();
    let exception_root = heap.alloc_exception(exception).unwrap();
    heap.push_array_item(prefix, RuntimeValue::Heap(shared))
        .unwrap();

    let prepared = heap
        .prepare_rollback_rebase(
            checkpoint,
            &[
                RuntimeValue::Heap(array_root),
                RuntimeValue::Heap(exception_root),
                RuntimeValue::Heap(shared),
            ],
        )
        .unwrap();
    let roots = prepared.rebased_roots().to_vec();
    heap.commit_prepared_rollback_rebase(prepared);
    let direct_shared = roots[2].as_heap_handle().unwrap();

    let HeapNode::Array(prefix_items) = heap.get(prefix).unwrap() else {
        panic!("prefix must remain an array");
    };
    assert_eq!(prefix_items[0].as_heap_handle(), Some(direct_shared));

    let array_root = roots[0].as_heap_handle().unwrap();
    let HeapNode::Array(array_items) = heap.get(array_root).unwrap() else {
        panic!("explicit array root must remain an array");
    };
    assert_eq!(array_items[0].as_heap_handle(), Some(direct_shared));
    assert_eq!(array_items[1].as_heap_handle(), Some(direct_shared));

    let exception_root = roots[1].as_heap_handle().unwrap();
    let HeapNode::Exception(exception) = heap.get(exception_root).unwrap() else {
        panic!("explicit exception root must remain an exception");
    };
    assert_eq!(
        exception.local_value().unwrap().value().as_heap_handle(),
        Some(direct_shared)
    );
    assert_eq!(exception.local_catch_identity(), Some(&local_identity(31)));
}

#[test]
fn rollback_rebase_supports_cycles_and_rewrites_every_cycle_edge() {
    let mut heap = RequestHeap::default();
    let checkpoint = heap.checkpoint();
    let first = heap.alloc_array(Vec::new()).unwrap();
    let second = heap.alloc_array(Vec::new()).unwrap();
    heap.push_array_item_without_cycle_check_for_test(first, RuntimeValue::Heap(second))
        .unwrap();
    heap.push_array_item_without_cycle_check_for_test(second, RuntimeValue::Heap(first))
        .unwrap();
    let before_stats = heap.stats();

    let prepared = heap
        .prepare_rollback_rebase(checkpoint, &[RuntimeValue::Heap(first)])
        .unwrap();
    assert_eq!(heap.stats(), before_stats);
    assert_eq!(heap.len(), 2);
    let roots = prepared.rebased_roots().to_vec();
    heap.commit_prepared_rollback_rebase(prepared);
    let rebased_first = roots[0].as_heap_handle().unwrap();
    let HeapNode::Array(first_items) = heap.get(rebased_first).unwrap() else {
        panic!("first cycle node must remain an array");
    };
    let rebased_second = first_items[0].as_heap_handle().unwrap();
    let HeapNode::Array(second_items) = heap.get(rebased_second).unwrap() else {
        panic!("second cycle node must remain an array");
    };
    assert_eq!(second_items[0].as_heap_handle(), Some(rebased_first));
    assert!(heap.get(first).is_err());
    assert!(heap.get(second).is_err());
}

#[test]
fn rollback_rebase_corrupt_handle_failure_does_not_mutate_heap_or_stats() {
    let mut heap = RequestHeap::default();
    let prefix = heap.alloc_array(Vec::new()).unwrap();
    let checkpoint = heap.checkpoint();
    let corrupt = HeapHandle::new(10_000, 0);
    heap.push_array_item_without_cycle_check_for_test(prefix, RuntimeValue::Heap(corrupt))
        .unwrap();
    let before = heap.clone();

    let error = heap.prepare_rollback_rebase(checkpoint, &[]).unwrap_err();
    assert!(!error.is_skippable());
    assert!(error.to_string().contains("index is out of bounds"));
    assert_eq!(heap.stats(), before.stats());
    assert_eq!(heap.len(), before.len());
    assert_eq!(heap.get(prefix).unwrap(), before.get(prefix).unwrap());
}

#[test]
fn rollback_rebase_depth_limit_failure_does_not_mutate_heap_or_stats() {
    let limits = RequestHeapLimits {
        max_clone_depth: 1,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let checkpoint = heap.checkpoint();
    let leaf = heap.alloc_array(Vec::new()).unwrap();
    let middle = heap.alloc_array(vec![RuntimeValue::Heap(leaf)]).unwrap();
    let root = heap.alloc_array(vec![RuntimeValue::Heap(middle)]).unwrap();
    let before = heap.clone();

    let error = heap
        .prepare_rollback_rebase(checkpoint, &[RuntimeValue::Heap(root)])
        .unwrap_err();
    assert!(error.is_skippable());
    assert!(matches!(
        error,
        RequestHeapRollbackRebaseError::ResourceLimit(
            RuntimeModelError::ResourceLimitExceeded { reason, .. }
        ) if reason == "max clone depth"
    ));
    assert_eq!(heap.stats(), before.stats());
    assert_eq!(heap.len(), before.len());
    assert_eq!(heap.get(root).unwrap(), before.get(root).unwrap());
}

#[test]
fn rollback_rebase_generation_exhaustion_is_skippable_and_does_not_mutate_heap() {
    let mut heap = RequestHeap::default();
    let checkpoint = heap.checkpoint();
    heap.alloc_array(vec![RuntimeValue::from("valid")]).unwrap();
    heap.nodes[0].generation = u32::MAX;
    let root = HeapHandle::new(0, u32::MAX);
    assert!(heap.get(root).is_ok());
    let before = heap.clone();

    let error = heap
        .prepare_rollback_rebase(checkpoint, &[RuntimeValue::Heap(root)])
        .unwrap_err();
    assert!(error.is_skippable());
    assert!(matches!(
        error,
        RequestHeapRollbackRebaseError::ResourceLimit(
            RuntimeModelError::ResourceLimitExceeded { reason, .. }
        ) if reason == "heap slot generation space"
    ));
    assert_eq!(heap.stats(), before.stats());
    assert_eq!(heap.get(root).unwrap(), before.get(root).unwrap());
}

#[test]
fn clone_depth_limit_is_enforced() {
    let limits = RequestHeapLimits {
        max_clone_depth: 0,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let child = heap.alloc_array(Vec::new()).expect("child should allocate");
    let root = heap
        .alloc_array(vec![RuntimeValue::Heap(child)])
        .expect("root should allocate");

    let error = deep_clone_runtime_value(&mut heap, &RuntimeValue::Heap(root)).unwrap_err();
    assert!(matches!(
        error,
        RuntimeModelError::ResourceLimitExceeded { reason, .. } if reason == "max clone depth"
    ));
}

#[test]
fn materialize_stats_helpers_enforce_depth_and_bytes_limits() {
    let limits = RequestHeapLimits {
        max_materialize_depth: 1,
        max_materialize_output_bytes: 8,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);

    let depth_error = heap.record_materialize_stats(2, 1).unwrap_err();
    assert!(matches!(
        depth_error,
        RuntimeModelError::ResourceLimitExceeded { reason, .. } if reason == "max materialize depth"
    ));

    let bytes_error = heap
        .record_materialized_value(1, &serde_json::json!({"larger": true}))
        .unwrap_err();
    assert!(matches!(
        bytes_error,
        RuntimeModelError::ResourceLimitExceeded { reason, .. } if reason == "max materialize output bytes"
    ));
}

fn test_interface(payload: RuntimeValue) -> InterfaceValue {
    InterfaceValue::new(
        "pkg.Reader".to_string(),
        InterfaceCarrier::Local {
            concrete_type: "pkg.FileReader".to_string(),
            method_table: InterfaceMethodTable::new(
                "table:pkg.Reader:pkg.FileReader".to_string(),
                "pkg.Reader".to_string(),
                vec![InterfaceMethodSlot::new(
                    0,
                    "pkg.Reader.read".to_string(),
                    InterfaceMethodTarget::LocalExecutable {
                        executable: ExecutableAddr::service(0, 1),
                        receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                    },
                )],
            ),
            payload,
        },
    )
}
