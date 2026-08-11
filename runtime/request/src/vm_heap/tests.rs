use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    request_heap::RequestHeapLimits,
    service_error::{CatchIdentity, LocalExecutionTypeIdentity, NominalTypeIdentity},
    vm_heap::{VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapPathSegment, VmRecordField},
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};

use super::RequestVmHeap;

const TAG: CompactTypeTag = CompactTypeTag::new(17);
const FLAGS: ValueFlags = ValueFlags::new(1);

fn heap() -> RequestVmHeap {
    RequestVmHeap::with_domain(7, 0, RequestHeapLimits::default())
}

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

#[test]
fn array_alloc_read_mutate_and_release() {
    let mut heap = heap();
    let array = heap
        .allocate_array(&[ValueSlot::number(1.5), ValueSlot::integer(3)], TAG, FLAGS)
        .expect("array should allocate");

    assert_eq!(heap.array_len(&array), Ok(2));
    assert!(heap.array_get(&array, 0) == Ok(ValueSlot::number(1.5)));
    assert!(heap.array_get(&array, 1) == Ok(ValueSlot::integer(3)));

    heap.array_push_owned(&array, ValueSlot::bool(true))
        .expect("push should succeed");
    assert_eq!(heap.array_len(&array), Ok(3));
    assert!(heap.array_get(&array, 2) == Ok(ValueSlot::bool(true)));

    heap.set_writable_path(
        &array,
        &[VmHeapPathSegment::ArrayIndex],
        &[ValueSlot::integer(0)],
        ValueSlot::number(9.0),
    )
    .expect("writable array path should succeed");
    assert!(heap.array_get(&array, 0) == Ok(ValueSlot::number(9.0)));

    let snapshot = heap.snapshot_share(&array).expect("snapshot should share");
    assert!(snapshot == array);
    heap.release_snapshot(&snapshot).expect("release snapshot");
    assert_eq!(heap.validate_live(&array), Ok(()));
    heap.release_snapshot(&array).expect("release final owner");
    assert!(matches!(
        heap.validate_live(&array),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
}

#[test]
fn map_get_put_and_entry_at_roundtrip() {
    let mut heap = heap();
    let key = heap.alloc_string("a").expect("string key should allocate");
    let map = heap
        .allocate_map(&[], TAG, FLAGS)
        .expect("map should allocate");

    assert_eq!(heap.map_len(&map), Ok(0));
    assert!(!heap
        .map_put_owned(&map, key, ValueSlot::number(1.0))
        .expect("put should succeed"));
    assert_eq!(heap.map_len(&map), Ok(1));
    assert!(heap.map_get(&map, &key) == Ok(ValueSlot::number(1.0)));
    let entry = heap.map_entry_at(&map, 0).expect("entry should resolve");
    assert!(entry.key == key && entry.value == ValueSlot::number(1.0));

    assert!(heap
        .map_put_owned(&map, key, ValueSlot::bool(true))
        .expect("second put should succeed"));
    assert_eq!(heap.map_len(&map), Ok(1));
    assert!(heap.map_get(&map, &key) == Ok(ValueSlot::bool(true)));
}

#[test]
fn record_field_writable_path_and_representation() {
    let mut heap = heap();
    let record = heap
        .allocate_record(
            &[VmRecordField {
                name: "count".to_string(),
                value: ValueSlot::integer(3),
            }],
            TAG,
            FLAGS,
        )
        .expect("record should allocate");

    assert!(heap.record_field(&record, "count") == Ok(ValueSlot::integer(3)));
    assert!(heap.get_dense_field(&record, 0) == Ok(ValueSlot::integer(3)));

    heap.set_writable_path(
        &record,
        &[VmHeapPathSegment::DenseField {
            field: "count".to_string(),
        }],
        &[],
        ValueSlot::bool(true),
    )
    .expect("record path should succeed");
    assert!(heap.record_field(&record, "count") == Ok(ValueSlot::bool(true)));

    let payload = heap.alloc_string("payload").expect("payload string");
    let representation = heap
        .allocate_representation(&payload, local_identity(4), TAG, FLAGS)
        .expect("representation should allocate");
    assert!(heap.representation_payload(&representation) == Ok(payload));
}

#[test]
fn foreign_and_stale_request_heap_refs_fail_closed() {
    let mut first = RequestVmHeap::with_domain(1, 0, RequestHeapLimits::default());
    let second = RequestVmHeap::with_domain(2, 0, RequestHeapLimits::default());
    let array = first
        .allocate_array(&[ValueSlot::number(1.0)], TAG, FLAGS)
        .expect("array should allocate");

    assert!(matches!(
        second.validate_live(&array),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::WrongDomain,
            ..
        })
    ));
    assert!(matches!(
        second.array_len(&array),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::WrongDomain,
            ..
        })
    ));

    first
        .release_snapshot(&array)
        .expect("release should succeed");
    assert!(matches!(
        first.array_len(&array),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
}

#[test]
fn allocation_resource_limits_fail_closed() {
    let limits = RequestHeapLimits {
        max_nodes: 1,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestVmHeap::with_domain(3, 0, limits);
    heap.allocate_array(&[ValueSlot::bool(true)], TAG, FLAGS)
        .expect("first array should allocate");

    assert!(matches!(
        heap.allocate_map(&[], TAG, FLAGS),
        Err(VmHeapError::ResourceLimitExceeded {
            operation: skiff_runtime_model::vm_heap::VmHeapOperation::AllocateMap,
            ..
        })
    ));
}
