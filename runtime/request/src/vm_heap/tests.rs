use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    request_heap::RequestHeapLimits,
    service_error::{CatchIdentity, LocalExecutionTypeIdentity, NominalTypeIdentity},
    vm_heap::{
        VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapOperation, VmHeapPathSegment,
        VmRecordField,
    },
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};

use super::RequestVmHeap;

const TAG: CompactTypeTag = CompactTypeTag::new(17);
const FLAGS: ValueFlags = ValueFlags::new(1);
const RESOURCE_TAG: CompactTypeTag = CompactTypeTag::new(27);
const RESOURCE_FLAGS: ValueFlags = ValueFlags::new(2);

fn heap() -> RequestVmHeap {
    RequestVmHeap::with_domain(7, 0, RequestHeapLimits::default())
}

fn resource_ref(handle: u64) -> ValueSlot {
    ValueSlot::resource_ref(
        skiff_runtime_model::vm_value::VmHandle::new(handle),
        RESOURCE_TAG,
        RESOURCE_FLAGS,
    )
}

#[test]
fn take_dense_field_physically_detaches_field_and_consumes_record_remainder() {
    let mut heap = heap();
    let taken_child = heap
        .allocate_record(
            &[VmRecordField {
                name: "value".to_string(),
                value: ValueSlot::integer(7),
            }],
            TAG,
            FLAGS,
        )
        .expect("taken child");
    let remainder = heap
        .allocate_record(
            &[VmRecordField {
                name: "value".to_string(),
                value: ValueSlot::integer(8),
            }],
            TAG,
            FLAGS,
        )
        .expect("remainder child");
    let record = heap
        .allocate_record(
            &[
                VmRecordField {
                    name: "body".to_string(),
                    value: taken_child,
                },
                VmRecordField {
                    name: "headers".to_string(),
                    value: remainder,
                },
                VmRecordField {
                    name: "status".to_string(),
                    value: ValueSlot::integer(200),
                },
            ],
            TAG,
            FLAGS,
        )
        .expect("stream-handle-shaped record");
    let physical = heap.live_entry(&record).expect("live record").heap_handle;

    assert_eq!(heap.take_dense_field(&record, 0), Ok(taken_child));
    assert_eq!(heap.validate_live(&taken_child), Ok(()));
    assert!(matches!(
        heap.validate_live(&record),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
    assert!(matches!(
        heap.validate_live(&remainder),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
    assert_eq!(
        heap.request_heap()
            .object_field_carrier(physical, "body")
            .expect("physical record remains inspectable"),
        None,
        "the selected field must be absent from the physical object"
    );
    assert!(matches!(
        heap.take_dense_field(&record, 0),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));

    heap.release_snapshot(&taken_child)
        .expect("returned field remains the caller-owned value");
}

#[test]
fn take_dense_field_rejects_alias_and_bad_ordinal_without_mutation() {
    let mut heap = heap();
    let record = heap
        .allocate_record(
            &[
                VmRecordField {
                    name: "body".to_string(),
                    value: ValueSlot::integer(1),
                },
                VmRecordField {
                    name: "status".to_string(),
                    value: ValueSlot::integer(200),
                },
            ],
            TAG,
            FLAGS,
        )
        .expect("record");
    let alias = heap.snapshot_share(&record).expect("shared alias");

    assert!(matches!(
        heap.take_dense_field(&record, 0),
        Err(VmHeapError::OwnershipViolation { .. })
    ));
    assert_eq!(heap.get_dense_field(&record, 0), Ok(ValueSlot::integer(1)));
    heap.release_snapshot(&alias).expect("release alias");

    assert!(matches!(
        heap.take_dense_field(&record, 7),
        Err(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::TakeDenseField,
            ..
        })
    ));
    assert_eq!(heap.get_dense_field(&record, 0), Ok(ValueSlot::integer(1)));
    assert_eq!(heap.take_dense_field(&record, 0), Ok(ValueSlot::integer(1)));
}

#[test]
fn take_dense_field_rejects_invalid_remainder_before_physical_detach() {
    let mut heap = heap();
    let child = heap
        .allocate_record(
            &[VmRecordField {
                name: "value".to_string(),
                value: ValueSlot::integer(7),
            }],
            TAG,
            FLAGS,
        )
        .expect("child");
    // A verified VM cannot construct two owning edges from one unshared
    // owner. The heap port still rejects such a forged graph atomically.
    let record = heap
        .allocate_record(
            &[
                VmRecordField {
                    name: "body".to_string(),
                    value: ValueSlot::integer(1),
                },
                VmRecordField {
                    name: "left".to_string(),
                    value: child,
                },
                VmRecordField {
                    name: "right".to_string(),
                    value: child,
                },
            ],
            TAG,
            FLAGS,
        )
        .expect("forged duplicate-owner record");
    let physical = heap.live_entry(&record).expect("live record").heap_handle;

    assert!(matches!(
        heap.take_dense_field(&record, 0),
        Err(VmHeapError::OwnershipViolation { .. })
    ));
    assert_eq!(heap.validate_live(&record), Ok(()));
    assert_eq!(heap.get_dense_field(&record, 0), Ok(ValueSlot::integer(1)));
    assert!(
        heap.request_heap()
            .object_field_carrier(physical, "body")
            .expect("physical record")
            .is_some(),
        "preflight rejection must leave the physical field attached"
    );
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

    let prepared = heap
        .prepare_writable_path(
            &array,
            &[VmHeapPathSegment::ArrayIndex],
            &[ValueSlot::integer(0)],
        )
        .expect("prepare should pin");
    let replacement = heap
        .commit_writable_path(prepared, ValueSlot::number(9.0))
        .expect("commit should apply");
    assert!(replacement == array, "exclusive commit writes in place");
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
fn shared_array_push_fails_closed_without_alias_mutation() {
    let mut heap = heap();
    let array = heap
        .allocate_array(&[ValueSlot::number(1.0)], TAG, FLAGS)
        .expect("array should allocate");
    let alias = heap.snapshot_share(&array).expect("alias should share");

    assert!(matches!(
        heap.array_push_owned(&array, ValueSlot::bool(true)),
        Err(VmHeapError::OwnershipViolation { .. })
    ));
    assert_eq!(heap.array_len(&array), Ok(1));
    assert!(heap.array_get(&alias, 0) == Ok(ValueSlot::number(1.0)));
    heap.release_snapshot(&alias).expect("release alias");
    heap.release_snapshot(&array).expect("release array");
}

#[test]
fn writable_path_cow_isolates_shared_aliases() {
    let mut heap = heap();
    let leaf = heap
        .allocate_record(
            &[VmRecordField {
                name: "x".to_string(),
                value: ValueSlot::integer(1),
            }],
            TAG,
            FLAGS,
        )
        .expect("leaf record");
    let record = heap
        .allocate_record(
            &[VmRecordField {
                name: "inner".to_string(),
                value: leaf,
            }],
            TAG,
            FLAGS,
        )
        .expect("outer record");
    let alias = heap.snapshot_share(&record).expect("alias should share");

    let prepared = heap
        .prepare_writable_path(
            &alias,
            &[
                VmHeapPathSegment::DenseField {
                    field: "inner".to_string(),
                },
                VmHeapPathSegment::DenseField {
                    field: "x".to_string(),
                },
            ],
            &[],
        )
        .expect("prepare should pin the nested path");
    let replacement = heap
        .commit_writable_path(prepared, ValueSlot::integer(2))
        .expect("shared commit should clone");
    assert!(
        !(replacement == record),
        "COW commit must return a new root handle"
    );

    let original_inner = heap.record_field(&record, "inner").expect("inner");
    assert!(
        heap.record_field(&original_inner, "x") == Ok(ValueSlot::integer(1)),
        "the alias aggregate must keep the original leaf"
    );
    let replaced_inner = heap.record_field(&replacement, "inner").expect("new inner");
    assert!(
        heap.record_field(&replaced_inner, "x") == Ok(ValueSlot::integer(2)),
        "the replacement aggregate must hold the new leaf"
    );

    heap.release_snapshot(&alias).expect("release alias owner");
    heap.release_snapshot(&record)
        .expect("release original owner");
    heap.release_snapshot(&replacement)
        .expect("release replacement owner");
    assert!(matches!(
        heap.validate_live(&leaf),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
}

#[test]
fn writable_path_cow_covers_shared_intermediate_containers() {
    let mut heap = heap();
    let shared = heap
        .allocate_record(
            &[VmRecordField {
                name: "x".to_string(),
                value: ValueSlot::integer(1),
            }],
            TAG,
            FLAGS,
        )
        .expect("shared intermediate");
    let first_container_owner = heap
        .snapshot_share(&shared)
        .expect("first container owner should share");
    let left = heap
        .allocate_record(
            &[VmRecordField {
                name: "a".to_string(),
                value: first_container_owner,
            }],
            TAG,
            FLAGS,
        )
        .expect("left record");
    let second_container_owner = heap
        .snapshot_share(&shared)
        .expect("second container owner should share");
    let right = heap
        .allocate_record(
            &[VmRecordField {
                name: "b".to_string(),
                value: second_container_owner,
            }],
            TAG,
            FLAGS,
        )
        .expect("right record");
    heap.release_snapshot(&shared)
        .expect("the temporary holder owner should release");

    let prepared = heap
        .prepare_writable_path(
            &left,
            &[
                VmHeapPathSegment::DenseField {
                    field: "a".to_string(),
                },
                VmHeapPathSegment::DenseField {
                    field: "x".to_string(),
                },
            ],
            &[],
        )
        .expect("prepare should pin");
    let replacement = heap
        .commit_writable_path(prepared, ValueSlot::integer(2))
        .expect("shared intermediate must clone");
    assert!(!(replacement == left));

    let right_shared = heap.record_field(&right, "b").expect("right shared");
    assert!(
        heap.record_field(&right_shared, "x") == Ok(ValueSlot::integer(1)),
        "mutation through the left path must not alias the right record"
    );
}

#[test]
fn recursive_snapshot_drop_releases_nested_aggregate_owners() {
    let mut heap = heap();
    let leaf = heap
        .allocate_record(
            &[VmRecordField {
                name: "x".to_string(),
                value: ValueSlot::integer(1),
            }],
            TAG,
            FLAGS,
        )
        .expect("leaf record");
    let array = heap
        .allocate_array(&[leaf], TAG, FLAGS)
        .expect("nested array");
    let outer = heap
        .allocate_record(
            &[VmRecordField {
                name: "field".to_string(),
                value: array,
            }],
            TAG,
            FLAGS,
        )
        .expect("outer record");
    let shared = heap.snapshot_share(&outer).expect("outer should share");

    heap.release_snapshot(&shared)
        .expect("release shared owner");
    assert_eq!(heap.validate_live(&leaf), Ok(()), "leaf stays live");
    heap.release_snapshot(&outer).expect("release final owner");
    assert!(matches!(
        heap.validate_live(&leaf),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
}

#[test]
fn prepare_failure_leaves_heap_state_unchanged() {
    let mut heap = heap();
    let array = heap
        .allocate_array(&[ValueSlot::number(1.0)], TAG, FLAGS)
        .expect("array should allocate");

    assert!(matches!(
        heap.prepare_writable_path(
            &array,
            &[VmHeapPathSegment::ArrayIndex],
            &[ValueSlot::integer(7)],
        ),
        Err(VmHeapError::HeapOperationFailed { .. })
    ));
    assert!(heap.array_get(&array, 0) == Ok(ValueSlot::number(1.0)));

    let prepared = heap
        .prepare_writable_path(
            &array,
            &[VmHeapPathSegment::ArrayIndex],
            &[ValueSlot::integer(0)],
        )
        .expect("valid prepare should succeed afterwards");
    let replacement = heap
        .commit_writable_path(prepared, ValueSlot::number(2.0))
        .expect("commit should apply");
    assert!(replacement == array);
    assert!(heap.array_get(&array, 0) == Ok(ValueSlot::number(2.0)));
}

#[test]
fn commit_cow_failure_leaves_the_old_chain_intact() {
    let limits = RequestHeapLimits {
        max_nodes: 1,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestVmHeap::with_domain(7, 0, limits);
    let array = heap
        .allocate_array(&[ValueSlot::number(1.0)], TAG, FLAGS)
        .expect("array should allocate");
    let alias = heap.snapshot_share(&array).expect("alias should share");
    let prepared = heap
        .prepare_writable_path(
            &alias,
            &[VmHeapPathSegment::ArrayIndex],
            &[ValueSlot::integer(0)],
        )
        .expect("prepare should pin");

    assert!(matches!(
        heap.commit_writable_path(prepared, ValueSlot::number(2.0)),
        Err(VmHeapError::ResourceLimitExceeded {
            operation: VmHeapOperation::CommitWritablePath,
            ..
        })
    ));
    assert!(heap.array_get(&alias, 0) == Ok(ValueSlot::number(1.0)));
    assert_eq!(heap.validate_live(&alias), Ok(()));
    heap.release_snapshot(&alias).expect("release alias");
    heap.release_snapshot(&array).expect("release array");
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

    let prepared = heap
        .prepare_writable_path(
            &record,
            &[VmHeapPathSegment::DenseField {
                field: "count".to_string(),
            }],
            &[],
        )
        .expect("prepare should pin");
    let replacement = heap
        .commit_writable_path(prepared, ValueSlot::bool(true))
        .expect("commit should apply");
    assert!(replacement == record);
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

// --- ResourceRef fail-closed heap contract tests ---

#[test]
fn validate_live_rejects_resource_ref_without_table() {
    let heap = heap();
    let slot = resource_ref(1);

    assert!(matches!(
        heap.validate_live(&slot),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ValidateLive,
            kind: ValueKind::ResourceRef,
        })
    ));
}

#[test]
fn snapshot_share_rejects_resource_ref() {
    let mut heap = heap();
    let slot = resource_ref(1);

    assert!(matches!(
        heap.snapshot_share(&slot),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::SnapshotShare,
            kind: ValueKind::ResourceRef,
        })
    ));
}

#[test]
fn release_snapshot_rejects_resource_ref() {
    let mut heap = heap();
    let slot = resource_ref(1);

    assert!(matches!(
        heap.release_snapshot(&slot),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseSnapshot,
            kind: ValueKind::ResourceRef,
        })
    ));
}

#[test]
fn transfer_owner_rejects_resource_ref() {
    let mut heap = heap();
    let slot = resource_ref(7);

    assert!(matches!(
        heap.transfer_owner(&slot),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::TransferOwner,
            kind: ValueKind::ResourceRef,
        })
    ));
}

#[test]
fn release_resource_rejects_without_resource_table() {
    let mut heap = heap();
    let slot = resource_ref(404);

    assert!(matches!(
        heap.release_resource(&slot),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseResource,
            kind: ValueKind::ResourceRef,
        })
    ));
}
