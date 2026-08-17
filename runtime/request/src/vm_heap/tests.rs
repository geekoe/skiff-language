use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use skiff_artifact_model::{PackageBuildId, ServiceProtocolIdentity, ServiceRequirementKey};
use skiff_runtime_linked_bytecode::{
    LinkedLocalInterfaceTable, LinkedPublicInstanceKey, LinkedRemoteInterfaceTable, TypeIndex,
};
use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    request_heap::RequestHeapLimits,
    runtime_value::ActorRef,
    service_error::{CatchIdentity, LocalExecutionTypeIdentity, NominalTypeIdentity},
    vm_heap::{
        VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapOperation, VmHeapPathSegment,
        VmLocalInterfaceTable, VmRecordField, VmRemoteInterfaceTable,
    },
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeSchedulerPorts, RequestByteStreamPullFuture, RequestByteStreamPullStartError,
    RequestByteStreamSource, RequestExecutionContext, RequestResourceTermination,
};
use skiff_runtime_vm::VmFiber;

use super::RequestVmHeap;
use crate::RequestMemoryLedger;

const fn tag(type_index: u32) -> CompactTypeTag {
    match CompactTypeTag::try_from_type_index(type_index) {
        Some(tag) => tag,
        None => panic!("test type index must fit compact tag"),
    }
}

const TAG: CompactTypeTag = tag(17);
const FLAGS: ValueFlags = ValueFlags::new(1);
const RESOURCE_TAG: CompactTypeTag = tag(27);
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
fn release_resource_rejects_number_before_resource_lookup() {
    let mut heap = heap();
    assert!(matches!(
        heap.release_resource(&ValueSlot::number(1.0)),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseResource,
            kind: ValueKind::Number
        })
    ));
}

#[test]
fn actor_state_ref_round_trips_and_release_keeps_logical_identity() {
    let mut heap = heap();
    let actor_ref = ActorRef::new(
        "svc",
        "Counter",
        "id",
        "skiff-canonical-v1",
        b"\"phase6\"".to_vec(),
        "sha256:actor-id",
        Some(1),
    );
    let slot = heap
        .alloc_actor_state_ref(actor_ref.clone(), TAG, FLAGS)
        .expect("alloc actor state ref");
    assert_eq!(
        heap.actor_state_ref_value(&slot).expect("read actor ref"),
        actor_ref
    );
    assert!(heap.validate_live(&slot).is_ok());

    let shared = heap.snapshot_share(&slot).expect("share actor ref");
    assert!(shared == slot);
    heap.release_snapshot(&slot).expect("release actor ref");
    assert_eq!(
        heap.actor_state_ref_value(&slot)
            .expect("read after release"),
        actor_ref
    );
}

#[test]
fn phase_6_local_interface_carrier_allocates_reads_and_releases_exact_payload() {
    let mut heap = heap();
    let live_before = heap.live_value_count();
    let payload = heap
        .alloc_typed_string("interface-payload".to_string(), tag(5), FLAGS)
        .expect("payload string");
    let local =
        LinkedLocalInterfaceTable::new(TypeIndex::new(5), Box::new([])).expect("empty local table");
    let exact_arc = Arc::new(local.clone());
    let exact_any: Arc<dyn Any + Send + Sync> =
        Arc::clone(&exact_arc) as Arc<dyn Any + Send + Sync>;
    let table = VmLocalInterfaceTable::new(3, 5, 0, exact_any);

    let heap_obj: &mut dyn VmHeap = &mut heap;
    let carrier = heap_obj
        .allocate_local_interface(&payload, table, tag(6), FLAGS)
        .expect("local interface carrier allocation");

    assert!(heap.local_interface_payload(&carrier) == Ok(payload));
    let carrier_table = heap
        .local_interface_table(&carrier)
        .expect("carrier table read");
    assert_eq!(carrier_table.table_index(), 3);
    assert_eq!(carrier_table.concrete_type(), 5);
    assert_eq!(carrier_table.method_count(), 0);
    let linked = heap
        .local_interface_linked_table(&carrier)
        .expect("exact linked table read");
    assert!(Arc::ptr_eq(&linked, &exact_arc));
    assert_eq!(&*linked, &local);

    assert!(matches!(
        heap.representation_payload(&carrier),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::RepresentationPayload,
            ..
        })
    ));
    assert!(matches!(
        heap.local_interface_payload(&payload),
        Err(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::LocalInterfacePayload,
            ..
        })
    ));

    let foreign = RequestVmHeap::with_domain(8, 0, RequestHeapLimits::default());
    assert!(matches!(
        foreign.local_interface_payload(&carrier),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::WrongDomain,
            ..
        })
    ));

    heap.release_snapshot(&carrier)
        .expect("carrier release releases its exact payload");
    assert_eq!(heap.live_value_count(), live_before);
    assert!(heap.validate_live(&carrier).is_err());
    assert!(heap.validate_live(&payload).is_err());
}

#[test]
fn phase_6_local_interface_carrier_rejects_wrong_exact_table_and_cleans_payload() {
    let mut heap = heap();
    let live_before = heap.live_value_count();
    let payload = heap
        .alloc_typed_string("interface-payload".to_string(), tag(5), FLAGS)
        .expect("payload string");
    let exact_any: Arc<dyn Any + Send + Sync> = Arc::new(17_u32);
    let table = VmLocalInterfaceTable::new(3, 5, 0, exact_any);

    let heap_obj: &mut dyn VmHeap = &mut heap;
    let carrier = heap_obj
        .allocate_local_interface(&payload, table, tag(6), FLAGS)
        .expect("local interface carrier allocation");

    assert!(heap.local_interface_payload(&carrier) == Ok(payload));
    assert!(heap.local_interface_linked_table(&carrier).is_err());

    heap.release_snapshot(&carrier)
        .expect("carrier release releases its exact payload");
    assert_eq!(heap.live_value_count(), live_before);
}

#[test]
fn phase_6_remote_interface_carrier_allocates_reads_and_releases_exact_table() {
    let mut heap = heap();
    let live_before = heap.live_value_count();
    let remote = LinkedRemoteInterfaceTable::new(
        ServiceRequirementKey {
            caller_package_build_id: PackageBuildId::new("build:caller"),
            service_requirement_slot: 0,
        },
        LinkedPublicInstanceKey::parse("instance:reader").expect("public instance key"),
        Box::new([]),
        ServiceProtocolIdentity::new("protocol:reader-v1"),
    )
    .expect("empty remote table is canonical");
    let exact_arc = Arc::new(remote.clone());
    let exact_any: Arc<dyn Any + Send + Sync> =
        Arc::clone(&exact_arc) as Arc<dyn Any + Send + Sync>;
    let table = VmRemoteInterfaceTable::new(4, 0, exact_any);

    let heap_obj: &mut dyn VmHeap = &mut heap;
    let carrier = heap_obj
        .allocate_remote_interface(table, tag(6), FLAGS)
        .expect("remote interface carrier allocation");

    let carrier_table = heap
        .remote_interface_table(&carrier)
        .expect("carrier table read");
    assert_eq!(carrier_table.table_index(), 4);
    assert_eq!(carrier_table.method_count(), 0);
    let linked = heap
        .remote_interface_linked_table(&carrier)
        .expect("exact linked remote table read");
    assert!(Arc::ptr_eq(&linked, &exact_arc));
    assert_eq!(&*linked, &remote);

    assert!(matches!(
        heap.representation_payload(&carrier),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::RepresentationPayload,
            ..
        })
    ));

    heap.release_snapshot(&carrier)
        .expect("remote carrier release");
    assert_eq!(heap.live_value_count(), live_before);
    assert!(heap.validate_live(&carrier).is_err());
}

#[test]
fn phase_6_remote_interface_carrier_rejects_wrong_exact_table() {
    let mut heap = heap();
    let exact_any: Arc<dyn Any + Send + Sync> = Arc::new(17_u32);
    let table = VmRemoteInterfaceTable::new(4, 0, exact_any);

    let heap_obj: &mut dyn VmHeap = &mut heap;
    let carrier = heap_obj
        .allocate_remote_interface(table, tag(6), FLAGS)
        .expect("remote interface carrier allocation");

    assert!(heap.remote_interface_linked_table(&carrier).is_err());
    assert!(heap.local_interface_table(&carrier).is_err());
    heap.release_snapshot(&carrier)
        .expect("remote carrier release");
}

struct RecordingByteStreamSource(Arc<Mutex<Vec<RequestResourceTermination>>>);

impl VmRootSource for RecordingByteStreamSource {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
    }
}

impl RequestByteStreamSource for RecordingByteStreamSource {
    fn start_pull(&self) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
        Ok(Box::pin(async { Ok(None) }))
    }

    fn terminate(self: Box<Self>, termination: RequestResourceTermination) {
        self.0.lock().unwrap().push(termination);
    }
}

#[test]
fn ledger_issued_heap_charges_allocations_and_fails_closed_on_hard_cap() {
    let ledger = Arc::new(RequestMemoryLedger::new(64));
    let (domain, epoch) = ledger.mint_heap_identity().expect("mint identity");
    let mut heap = RequestVmHeap::with_ledger(
        Arc::clone(&ledger),
        domain.get(),
        epoch.get(),
        RequestHeapLimits::default(),
    );

    let error = match heap.alloc_typed_bytes(vec![0; 128], TAG, FLAGS) {
        Err(error) => error,
        Ok(_) => panic!("aggregate ledger hard cap must reject the allocation"),
    };
    assert!(matches!(error, VmHeapError::HeapOperationFailed { .. }));
    assert_eq!(ledger.snapshot().committed, 0);

    drop(heap);
    assert_eq!(ledger.snapshot().committed, 0);
    ledger.mark_terminal().expect("ledger terminal at zero");
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

    assert!(heap.take_dense_field(&record, 0) == Ok(taken_child));
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
    assert!(heap.get_dense_field(&record, 0) == Ok(ValueSlot::integer(1)));
    heap.release_snapshot(&alias).expect("release alias");

    assert!(matches!(
        heap.take_dense_field(&record, 7),
        Err(VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::TakeDenseField,
            ..
        })
    ));
    assert!(heap.get_dense_field(&record, 0) == Ok(ValueSlot::integer(1)));
    assert!(heap.take_dense_field(&record, 0) == Ok(ValueSlot::integer(1)));
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
    assert!(heap.get_dense_field(&record, 0) == Ok(ValueSlot::integer(1)));
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
    let key = heap
        .alloc_typed_string("a".to_string(), TAG, FLAGS)
        .expect("string key should allocate");
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

    let payload = heap
        .alloc_typed_string("payload".to_string(), TAG, FLAGS)
        .expect("payload string");
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

#[test]
fn phase_5_resource_request_heap_admits_validates_moves_and_drops_exact_route() {
    let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
    let table = context.resource_table();
    let terminations = Arc::new(Mutex::new(Vec::new()));
    let handle = table
        .register_byte_stream(Box::new(RecordingByteStreamSource(Arc::clone(
            &terminations,
        ))))
        .unwrap();
    let mut heap = RequestVmHeap::for_execution(table.clone(), RequestHeapLimits::default());
    let slot = heap
        .admit_resource_ref(handle.vm_handle(), RESOURCE_TAG, RESOURCE_FLAGS)
        .unwrap();

    assert_eq!(heap.validate_live(&slot), Ok(()));
    assert!(heap.transfer_owner(&slot) == Ok(slot));
    let forged = ValueSlot::resource_ref(
        handle.vm_handle(),
        tag(RESOURCE_TAG.type_index() + 1),
        RESOURCE_FLAGS,
    );
    assert_eq!(
        heap.validate_live(&forged),
        Err(VmHeapError::InvalidValueMetadata)
    );
    heap.release_resource(&slot).unwrap();
    heap.release_resource(&slot).unwrap();
    assert_eq!(table.snapshot().total(), 0);
    assert_eq!(
        *terminations.lock().unwrap(),
        [RequestResourceTermination::VmDrop]
    );

    let snapshot = context.into_not_started();
    assert_eq!(snapshot.resource.current, 0);
}

#[test]
fn phase_5_first_poll_typed_host_carriers_keep_nonzero_verified_metadata() {
    let mut heap = heap();
    let bytes_tag = tag(71);
    let string_tag = tag(72);
    let flags = ValueFlags::new(5);
    let bytes = heap
        .alloc_typed_bytes(b"typed".to_vec(), bytes_tag, flags)
        .unwrap();
    let string = heap
        .alloc_typed_string("header".to_string(), string_tag, flags)
        .unwrap();

    assert_eq!(bytes.compact_type_tag(), Some(bytes_tag));
    assert_eq!(bytes.flags(), flags);
    assert_eq!(heap.bytes_value(&bytes).unwrap(), b"typed");
    assert_eq!(string.compact_type_tag(), Some(string_tag));
    assert_eq!(string.flags(), flags);
    assert_eq!(heap.string_value(&string).unwrap(), "header");

    let drifted_bytes = ValueSlot::request_heap_ref(
        bytes.as_request_heap_ref().unwrap(),
        tag(bytes_tag.type_index() + 1),
        flags,
    );
    let drifted_string = ValueSlot::request_heap_ref(
        string.as_request_heap_ref().unwrap(),
        string_tag,
        ValueFlags::new(flags.bits() + 1),
    );
    assert_eq!(
        heap.bytes_value(&drifted_bytes),
        Err(VmHeapError::InvalidValueMetadata)
    );
    assert_eq!(
        heap.string_value(&drifted_string),
        Err(VmHeapError::InvalidValueMetadata)
    );
}

#[test]
fn phase_5_resource_take_dense_field_preflights_and_drops_only_the_remainder_route() {
    let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
    let table = context.resource_table();
    let selected_terminations = Arc::new(Mutex::new(Vec::new()));
    let remainder_terminations = Arc::new(Mutex::new(Vec::new()));
    let selected_handle = table
        .register_byte_stream(Box::new(RecordingByteStreamSource(Arc::clone(
            &selected_terminations,
        ))))
        .unwrap();
    let remainder_handle = table
        .register_byte_stream(Box::new(RecordingByteStreamSource(Arc::clone(
            &remainder_terminations,
        ))))
        .unwrap();
    let mut heap = RequestVmHeap::for_execution(table.clone(), RequestHeapLimits::default());
    let selected = heap
        .admit_resource_ref(selected_handle.vm_handle(), RESOURCE_TAG, RESOURCE_FLAGS)
        .unwrap();
    let remainder = heap
        .admit_resource_ref(remainder_handle.vm_handle(), RESOURCE_TAG, RESOURCE_FLAGS)
        .unwrap();
    let record = heap
        .allocate_record(
            &[
                VmRecordField {
                    name: "body".to_string(),
                    value: selected,
                },
                VmRecordField {
                    name: "headers".to_string(),
                    value: remainder,
                },
            ],
            TAG,
            FLAGS,
        )
        .unwrap();

    assert!(heap.take_dense_field(&record, 0) == Ok(selected));
    assert_eq!(heap.validate_live(&selected), Ok(()));
    assert!(heap.validate_live(&remainder).is_err());
    assert!(selected_terminations.lock().unwrap().is_empty());
    assert_eq!(
        *remainder_terminations.lock().unwrap(),
        [RequestResourceTermination::VmDrop]
    );
    heap.release_resource(&selected).unwrap();
    assert_eq!(table.snapshot().total(), 0);

    let snapshot = context.into_not_started();
    assert_eq!(snapshot.resource.current, 0);
}

#[test]
fn phase_5_recursive_snapshot_drop_atomically_commits_mixed_heap_and_resource_carriers() {
    let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
    let table = context.resource_table();
    let terminations = Arc::new(Mutex::new(Vec::new()));
    let handle = table
        .register_byte_stream(Box::new(RecordingByteStreamSource(Arc::clone(
            &terminations,
        ))))
        .unwrap();
    let mut heap = RequestVmHeap::for_execution(table.clone(), RequestHeapLimits::default());
    let resource = heap
        .admit_resource_ref(handle.vm_handle(), RESOURCE_TAG, RESOURCE_FLAGS)
        .unwrap();
    let nested = heap
        .allocate_record(
            &[
                VmRecordField {
                    name: "body".to_string(),
                    value: resource,
                },
                VmRecordField {
                    name: "status".to_string(),
                    value: ValueSlot::integer(200),
                },
            ],
            TAG,
            FLAGS,
        )
        .unwrap();
    let root = heap
        .allocate_record(
            &[
                VmRecordField {
                    name: "handle".to_string(),
                    value: nested,
                },
                VmRecordField {
                    name: "marker".to_string(),
                    value: ValueSlot::integer(1),
                },
            ],
            TAG,
            FLAGS,
        )
        .unwrap();

    heap.release_snapshot(&root).unwrap();

    for released in [root, nested] {
        assert!(matches!(
            heap.validate_live(&released),
            Err(VmHeapError::InvalidHandle {
                reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
                ..
            })
        ));
    }
    assert_eq!(table.snapshot().total(), 0);
    assert_eq!(
        *terminations.lock().unwrap(),
        [RequestResourceTermination::VmDrop]
    );
    let snapshot = context.into_not_started();
    assert_eq!(snapshot.resource.current, 0);
}

#[test]
fn phase_5_recursive_snapshot_drop_first_and_middle_preflight_failures_are_retryable() {
    for corrupted_field in ["z-resource", "m-resource"] {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let table = context.resource_table();
        let terminations = Arc::new(Mutex::new(Vec::new()));
        let handle = table
            .register_byte_stream(Box::new(RecordingByteStreamSource(Arc::clone(
                &terminations,
            ))))
            .unwrap();
        let mut heap = RequestVmHeap::for_execution(table.clone(), RequestHeapLimits::default());
        let resource = heap
            .admit_resource_ref(handle.vm_handle(), RESOURCE_TAG, RESOURCE_FLAGS)
            .unwrap();
        let first = heap
            .allocate_record(
                &[VmRecordField {
                    name: "value".to_string(),
                    value: ValueSlot::integer(1),
                }],
                TAG,
                FLAGS,
            )
            .unwrap();
        let last = heap
            .allocate_record(
                &[VmRecordField {
                    name: "value".to_string(),
                    value: ValueSlot::integer(2),
                }],
                TAG,
                FLAGS,
            )
            .unwrap();
        let root = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "a-first".to_string(),
                        value: first,
                    },
                    VmRecordField {
                        name: "m-resource".to_string(),
                        value: resource,
                    },
                    VmRecordField {
                        name: "z-last".to_string(),
                        value: last,
                    },
                ],
                TAG,
                FLAGS,
            )
            .unwrap();
        let root_heap = heap.live_entry(&root).unwrap().heap_handle;
        let target_field = if corrupted_field == "z-resource" {
            // BTreeMap children are pushed in ascending order and consumed as
            // a stack, so replacing the last field makes this the first
            // preflight failure. The middle case first validates `z-last`.
            "z-last"
        } else {
            "m-resource"
        };
        let corrupted = ValueSlot::resource_ref(
            handle.vm_handle(),
            tag(RESOURCE_TAG.type_index() + 1),
            RESOURCE_FLAGS,
        );
        let original = heap
            .object_slots
            .get_mut(&root_heap)
            .unwrap()
            .insert(target_field.to_string(), corrupted)
            .unwrap();

        assert_eq!(
            heap.release_snapshot(&root),
            Err(VmHeapError::InvalidValueMetadata)
        );
        assert_eq!(heap.snapshot_owner_count(&root), Ok(1));
        assert_eq!(heap.snapshot_owner_count(&first), Ok(1));
        assert_eq!(heap.snapshot_owner_count(&last), Ok(1));
        assert!(
            heap.object_slots
                .get(&root_heap)
                .and_then(|slots| slots.get(target_field))
                == Some(&corrupted),
            "preflight failure must not detach or rewrite the root sidecar"
        );
        assert_eq!(heap.validate_live(&root), Ok(()));
        assert_eq!(heap.validate_live(&first), Ok(()));
        assert_eq!(heap.validate_live(&last), Ok(()));
        assert_eq!(table.snapshot().total(), 1);
        assert!(terminations.lock().unwrap().is_empty());

        heap.object_slots
            .get_mut(&root_heap)
            .unwrap()
            .insert(target_field.to_string(), original);
        heap.release_snapshot(&root)
            .expect("restoring the exact carrier makes the same owner retryable");
        assert_eq!(table.snapshot().total(), 0);
        assert_eq!(
            *terminations.lock().unwrap(),
            [RequestResourceTermination::VmDrop]
        );
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
    }
}
