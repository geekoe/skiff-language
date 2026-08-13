use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    request_heap::RequestHeapLimits,
    service_error::{CatchIdentity, LocalExecutionTypeIdentity, NominalTypeIdentity},
    vm_heap::{
        VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapOperation, VmHeapPathSegment, VmMapEntry,
        VmRecordField,
    },
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};
use skiff_runtime_scheduler::RequestExecutionOwnerInventory;

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Barrier,
    },
    time::Duration,
};

use super::{RegisterResourceError, RequestVmHeap, ResourceTable};

const TAG: CompactTypeTag = CompactTypeTag::new(17);
const FLAGS: ValueFlags = ValueFlags::new(1);
const RESOURCE_TAG: CompactTypeTag = CompactTypeTag::new(27);
const RESOURCE_FLAGS: ValueFlags = ValueFlags::new(2);

fn heap() -> RequestVmHeap {
    RequestVmHeap::with_domain(7, 0, RequestHeapLimits::default())
}

fn heap_with_resource_table() -> (RequestVmHeap, ResourceTable) {
    let mut heap = RequestVmHeap::with_domain(7, 0, RequestHeapLimits::default());
    let (registrations, _freeze) = RequestExecutionOwnerInventory::open().into_parts();
    let table = ResourceTable::new(registrations.resource());
    heap.set_resource_table(table.clone());
    (heap, table)
}

fn register_resource(table: &ResourceTable, handle: u64, cancels: Arc<AtomicUsize>) -> ValueSlot {
    let vm_handle = skiff_runtime_model::vm_value::VmHandle::new(handle);
    let cancels_clone = Arc::clone(&cancels);
    table
        .register(
            vm_handle,
            RESOURCE_TAG,
            RESOURCE_FLAGS,
            Arc::new(move || {
                cancels_clone.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    ValueSlot::resource_ref(vm_handle, RESOURCE_TAG, RESOURCE_FLAGS)
}

fn resource_ref(handle: u64) -> ValueSlot {
    ValueSlot::resource_ref(
        skiff_runtime_model::vm_value::VmHandle::new(handle),
        RESOURCE_TAG,
        RESOURCE_FLAGS,
    )
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

// --- ResourceRef heap contract tests ---

#[test]
fn validate_live_accepts_registered_resource_ref() {
    let (heap, table) = heap_with_resource_table();
    let cancels = Arc::new(AtomicUsize::new(0));
    let slot = register_resource(&table, 42, Arc::clone(&cancels));

    assert_eq!(heap.validate_live(&slot), Ok(()));
}

#[test]
fn validate_live_rejects_unregistered_resource_ref() {
    let (heap, _table) = heap_with_resource_table();
    let slot = resource_ref(99);

    assert!(matches!(
        heap.validate_live(&slot),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
}

#[test]
fn validate_live_rejects_metadata_mismatch() {
    let (heap, table) = heap_with_resource_table();
    let cancels = Arc::new(AtomicUsize::new(0));
    let vm_handle = skiff_runtime_model::vm_value::VmHandle::new(42);
    let slot = register_resource(&table, 42, cancels);
    let wrong_metadata =
        ValueSlot::resource_ref(vm_handle, CompactTypeTag::new(28), RESOURCE_FLAGS);

    assert_eq!(heap.validate_live(&slot), Ok(()));
    assert_eq!(
        heap.validate_live(&wrong_metadata),
        Err(VmHeapError::InvalidValueMetadata)
    );
}

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
    let (mut heap, table) = heap_with_resource_table();
    let cancels = Arc::new(AtomicUsize::new(0));
    let slot = register_resource(&table, 1, cancels);

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
    let (mut heap, table) = heap_with_resource_table();
    let cancels = Arc::new(AtomicUsize::new(0));
    let slot = register_resource(&table, 1, cancels);

    assert!(matches!(
        heap.release_snapshot(&slot),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseSnapshot,
            kind: ValueKind::ResourceRef,
        })
    ));
}

#[test]
fn transfer_owner_accepts_resource_ref() {
    let (mut heap, table) = heap_with_resource_table();
    let cancels = Arc::new(AtomicUsize::new(0));
    let slot = register_resource(&table, 7, cancels);

    let transferred = heap.transfer_owner(&slot);
    assert!(transferred.is_ok());
    assert!(transferred.unwrap() == slot);
}

#[test]
fn record_sidecar_roundtrip_preserves_resource_ref() {
    let (mut heap, table) = heap_with_resource_table();
    let cancels = Arc::new(AtomicUsize::new(0));
    let resource = register_resource(&table, 3, cancels);

    let record = heap
        .allocate_record(
            &[VmRecordField {
                name: "body".to_string(),
                value: resource,
            }],
            TAG,
            FLAGS,
        )
        .expect("record should allocate with a resource sidecar");

    assert!(heap.record_field(&record, "body") == Ok(resource));
    heap.release_snapshot(&record)
        .expect("record snapshot should release");
    assert_eq!(heap.release_resource(&resource), Ok(()));
}

#[test]
fn array_and_map_sidecars_preserve_resource_ref() {
    let (mut heap, table) = heap_with_resource_table();
    let array_resource = register_resource(&table, 10, Arc::new(AtomicUsize::new(0)));
    let map_resource = register_resource(&table, 11, Arc::new(AtomicUsize::new(0)));

    let array = heap
        .allocate_array(&[array_resource], TAG, FLAGS)
        .expect("array should allocate with a resource sidecar");
    assert!(heap.array_get(&array, 0) == Ok(array_resource));

    let key = heap
        .alloc_string("stream")
        .expect("map key should allocate");
    let map = heap
        .allocate_map(
            &[VmMapEntry {
                key,
                value: map_resource,
            }],
            TAG,
            FLAGS,
        )
        .expect("map should allocate with a resource sidecar");
    assert!(heap.map_get(&map, &key) == Ok(map_resource));

    heap.release_snapshot(&array)
        .expect("array snapshot should release");
    heap.release_snapshot(&map)
        .expect("map snapshot should release");
    assert_eq!(heap.release_resource(&array_resource), Ok(()));
    assert_eq!(heap.release_resource(&map_resource), Ok(()));
}

#[test]
fn release_resource_invokes_cancel_exactly_once_and_is_idempotent() {
    let (mut heap, table) = heap_with_resource_table();
    let cancels = Arc::new(AtomicUsize::new(0));
    let slot = register_resource(&table, 99, Arc::clone(&cancels));

    assert_eq!(cancels.load(Ordering::SeqCst), 0);
    assert_eq!(heap.release_resource(&slot), Ok(()));
    assert_eq!(cancels.load(Ordering::SeqCst), 1);

    // Second release is idempotent (handle already removed from table)
    assert_eq!(heap.release_resource(&slot), Ok(()));
    assert_eq!(cancels.load(Ordering::SeqCst), 1);
    assert!(matches!(
        heap.validate_live(&slot),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
}

#[test]
fn resource_entry_releases_its_actual_inventory_lease_after_cancel() {
    let inventory = RequestExecutionOwnerInventory::open();
    let (registrations, freeze) = inventory.into_parts();
    let table = ResourceTable::new(registrations.resource());
    let mut heap = RequestVmHeap::with_domain(7, 0, RequestHeapLimits::default());
    heap.set_resource_table(table.clone());
    let slot = register_resource(&table, 100, Arc::new(AtomicUsize::new(0)));

    heap.release_resource(&slot).unwrap();

    let frozen = freeze.freeze();
    assert_eq!(frozen.resource().current(), 0);
    assert!(frozen.resource().ever_created());
}

#[test]
fn occupied_resource_registration_aborts_without_an_inventory_increment() {
    let inventory = RequestExecutionOwnerInventory::open();
    let (registrations, freeze) = inventory.into_parts();
    let table = ResourceTable::new(registrations.resource());
    let handle = skiff_runtime_model::vm_value::VmHandle::new(101);
    table
        .register(handle, RESOURCE_TAG, RESOURCE_FLAGS, Arc::new(|| {}))
        .unwrap();

    assert_eq!(
        table.register(handle, RESOURCE_TAG, RESOURCE_FLAGS, Arc::new(|| {})),
        Err(RegisterResourceError::OccupiedHandle)
    );

    let frozen = freeze.freeze();
    assert_eq!(frozen.resource().current(), 1);
    assert!(frozen.resource().ever_created());
    table.remove_live(handle).unwrap().cancel();
}

#[test]
fn resource_release_unlocks_the_table_before_the_inventory_lease_drops() {
    let inventory = RequestExecutionOwnerInventory::open();
    let (registrations, freeze) = inventory.into_parts();
    let table = ResourceTable::new(registrations.resource());
    let first = skiff_runtime_model::vm_value::VmHandle::new(102);
    let cancel_entered = Arc::new(Barrier::new(2));
    let cancel_release = Arc::new(Barrier::new(2));
    let callback_entered = Arc::clone(&cancel_entered);
    let callback_release = Arc::clone(&cancel_release);
    table
        .register(
            first,
            RESOURCE_TAG,
            RESOURCE_FLAGS,
            Arc::new(move || {
                callback_entered.wait();
                callback_release.wait();
            }),
        )
        .unwrap();

    let releasing_table = table.clone();
    let releasing = std::thread::spawn(move || {
        releasing_table.remove_live(first).unwrap().cancel();
    });
    cancel_entered.wait();

    let inspecting_table = table.clone();
    let (inspected_tx, inspected_rx) = mpsc::channel();
    let inspecting = std::thread::spawn(move || {
        inspected_tx
            .send(inspecting_table.contains_live(first))
            .unwrap();
    });
    let still_live = match inspected_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(still_live) => still_live,
        Err(error) => {
            cancel_release.wait();
            panic!("resource table stayed locked while its removed lease was live: {error}");
        }
    };
    cancel_release.wait();
    releasing.join().unwrap();
    inspecting.join().unwrap();
    assert!(!still_live);

    let frozen = freeze.freeze();
    assert_eq!(frozen.resource().current(), 0);
    assert!(frozen.resource().ever_created());
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

#[test]
fn release_resource_rejects_unregistered_resource_ref() {
    let (mut heap, _table) = heap_with_resource_table();
    let slot = resource_ref(405);

    assert!(matches!(
        heap.release_resource(&slot),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
}
