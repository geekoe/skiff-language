use std::collections::BTreeMap;

use crate::{
    addr::{ExecutableAddr, FileAddr, TypeAddr, UnitAddr},
    error::RuntimeModelError,
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity,
        NominalTypeIdentity, RequestException,
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
    deep_clone_runtime_value_carrier_between_heaps, RequestHeap, RequestHeapLimits,
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
fn deep_clone_rejects_cycle() {
    let mut heap = RequestHeap::default();
    let root = heap.alloc_array(Vec::new()).expect("root should allocate");
    heap.push_array_item_without_cycle_check_for_test(root, RuntimeValue::Heap(root))
        .expect("cycle setup should update");

    let error = deep_clone_runtime_value(&mut heap, &RuntimeValue::Heap(root)).unwrap_err();
    assert!(error.to_string().contains("cyclic heap graph"));
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
