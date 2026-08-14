use std::{
    fmt,
    sync::{Arc, Mutex},
};

use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    service_error::CatchIdentity,
    vm_heap::{
        VmContainerElements, VmHeap, VmHeapError, VmHeapPathSegment, VmMapEntry, VmRecordField,
        WritablePathPreparation,
    },
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle},
};
use skiff_runtime_request::RequestVmHeap;

/// A physical identity snapshot of one `ValueSlot` that is safe to clone,
/// compare, and debug. The raw slot bits are intentionally opaque; ownership
/// and COW facts are only ever asserted through handle identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SpySlot {
    pub kind: Option<ValueKind>,
    pub handle: Option<VmHandle>,
    pub flags: ValueFlags,
}

impl SpySlot {
    pub fn of(value: ValueSlot) -> Self {
        Self {
            kind: value.kind(),
            handle: value.as_handle(),
            flags: value.flags(),
        }
    }

    pub fn handle_changed(self, after: ValueSlot) -> bool {
        self.handle != after.as_handle()
    }
}

impl fmt::Debug for SpySlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpySlot")
            .field("kind", &self.kind)
            .field("handle", &self.handle)
            .field("flags", &self.flags.bits())
            .finish()
    }
}

/// One recorded heap primitive transition. The sequence is the only fact the
/// Phase 2 VCP harness asserts: no artifact plan or VM semantic judgment is
/// re-derived here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapSpyEvent {
    SnapshotShare {
        source: SpySlot,
        result: SpySlot,
    },
    TransferOwner {
        source: SpySlot,
        result: SpySlot,
    },
    ReleaseSnapshot {
        owner: SpySlot,
    },
    ReleaseResource {
        owner: SpySlot,
    },
    AllocateRecord {
        result: SpySlot,
    },
    AllocateArray {
        result: SpySlot,
    },
    PrepareWritablePath {
        root: SpySlot,
        segments: Vec<VmHeapPathSegment>,
        selectors: usize,
    },
    CommitWritablePath {
        root_before: SpySlot,
        root_after: SpySlot,
        cow: bool,
    },
}

/// Shared, lock-protected recording side of the spy.
#[derive(Clone, Default)]
pub struct HeapSpyTrace {
    events: Arc<Mutex<Vec<HeapSpyEvent>>>,
}

impl HeapSpyTrace {
    pub fn events(&self) -> Vec<HeapSpyEvent> {
        self.events
            .lock()
            .expect("lock Phase 2 heap spy trace")
            .clone()
    }

    fn record(&self, event: HeapSpyEvent) {
        self.events
            .lock()
            .expect("lock Phase 2 heap spy trace")
            .push(event);
    }
}

/// A forwarding, recording `VmHeap` over the real `RequestVmHeap`.
///
/// Every call is delegated to the production heap and then recorded, so the
/// VM under test always observes the exact production behavior while the
/// harness observes the exact share/prepare/commit/drop primitive sequence.
/// Prepare/commit are correlated through a pending-root stack: `prepare`
/// pins the root handle and `commit` reports whether the returned replacement
/// root changed it (copy-on-write) or kept it (exclusive in-place write).
pub struct RecordingVmHeap {
    inner: Box<RequestVmHeap>,
    trace: HeapSpyTrace,
    pending_roots: Vec<SpySlot>,
}

impl RecordingVmHeap {
    pub fn new(limits: RequestHeapLimits, trace: HeapSpyTrace) -> Self {
        Self {
            inner: Box::new(RequestVmHeap::new(limits)),
            trace,
            pending_roots: Vec::new(),
        }
    }
}

impl VmHeap for RecordingVmHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.validate_live(value)
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        let result = self.inner.snapshot_share(source)?;
        self.trace.record(HeapSpyEvent::SnapshotShare {
            source: SpySlot::of(*source),
            result: SpySlot::of(result),
        });
        Ok(result)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        let result = self.inner.transfer_owner(source)?;
        self.trace.record(HeapSpyEvent::TransferOwner {
            source: SpySlot::of(*source),
            result: SpySlot::of(result),
        });
        Ok(result)
    }

    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.release_snapshot(owner)?;
        self.trace.record(HeapSpyEvent::ReleaseSnapshot {
            owner: SpySlot::of(*owner),
        });
        Ok(())
    }

    fn release_resource(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.inner.release_resource(owner)?;
        self.trace.record(HeapSpyEvent::ReleaseResource {
            owner: SpySlot::of(*owner),
        });
        Ok(())
    }

    fn allocate_array(
        &mut self,
        elements: &[ValueSlot],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        let result = self
            .inner
            .allocate_array(elements, compact_type_tag, flags)?;
        self.trace.record(HeapSpyEvent::AllocateArray {
            result: SpySlot::of(result),
        });
        Ok(result)
    }

    fn allocate_map(
        &mut self,
        entries: &[VmMapEntry],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.allocate_map(entries, compact_type_tag, flags)
    }

    fn allocate_record(
        &mut self,
        fields: &[VmRecordField],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        let result = self
            .inner
            .allocate_record(fields, compact_type_tag, flags)?;
        self.trace.record(HeapSpyEvent::AllocateRecord {
            result: SpySlot::of(result),
        });
        Ok(result)
    }

    fn allocate_representation(
        &mut self,
        payload: &ValueSlot,
        identity: CatchIdentity,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner
            .allocate_representation(payload, identity, compact_type_tag, flags)
    }

    fn alloc_typed_bytes(
        &mut self,
        value: Vec<u8>,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.alloc_typed_bytes(value, compact_type_tag, flags)
    }

    fn alloc_typed_string(
        &mut self,
        value: String,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner
            .alloc_typed_string(value, compact_type_tag, flags)
    }

    fn string_value(&self, value: &ValueSlot) -> Result<String, VmHeapError> {
        self.inner.string_value(value)
    }

    fn bytes_value(&self, value: &ValueSlot) -> Result<Vec<u8>, VmHeapError> {
        self.inner.bytes_value(value)
    }

    fn array_get(&self, array: &ValueSlot, index: usize) -> Result<ValueSlot, VmHeapError> {
        self.inner.array_get(array, index)
    }

    fn array_len(&self, array: &ValueSlot) -> Result<usize, VmHeapError> {
        self.inner.array_len(array)
    }

    fn map_get(&self, map: &ValueSlot, key: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.map_get(map, key)
    }

    fn map_len(&self, map: &ValueSlot) -> Result<usize, VmHeapError> {
        self.inner.map_len(map)
    }

    fn map_entry_at(&self, map: &ValueSlot, ordinal: usize) -> Result<VmMapEntry, VmHeapError> {
        self.inner.map_entry_at(map, ordinal)
    }

    fn record_field(&self, record: &ValueSlot, field: &str) -> Result<ValueSlot, VmHeapError> {
        self.inner.record_field(record, field)
    }

    fn representation_payload(&self, representation: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.inner.representation_payload(representation)
    }

    fn array_push_owned(&mut self, array: &ValueSlot, value: ValueSlot) -> Result<(), VmHeapError> {
        self.inner.array_push_owned(array, value)
    }

    fn map_put_owned(
        &mut self,
        map: &ValueSlot,
        key: ValueSlot,
        value: ValueSlot,
    ) -> Result<bool, VmHeapError> {
        self.inner.map_put_owned(map, key, value)
    }

    fn prepare_writable_path(
        &mut self,
        root: &ValueSlot,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
    ) -> Result<WritablePathPreparation, VmHeapError> {
        let prepared = self
            .inner
            .prepare_writable_path(root, segments, selectors)?;
        self.trace.record(HeapSpyEvent::PrepareWritablePath {
            root: SpySlot::of(*root),
            segments: segments.to_vec(),
            selectors: selectors.len(),
        });
        self.pending_roots.push(SpySlot::of(*root));
        Ok(prepared)
    }

    fn commit_writable_path(
        &mut self,
        prepared: WritablePathPreparation,
        value: ValueSlot,
    ) -> Result<ValueSlot, VmHeapError> {
        let replacement = self.inner.commit_writable_path(prepared, value)?;
        let root_before = self
            .pending_roots
            .pop()
            .expect("a successful commit must pair with a successful prepare");
        let root_after = SpySlot::of(replacement);
        self.trace.record(HeapSpyEvent::CommitWritablePath {
            root_before,
            root_after,
            cow: root_before.handle != root_after.handle,
        });
        Ok(replacement)
    }

    fn get_dense_field(
        &self,
        record: &ValueSlot,
        field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.get_dense_field(record, field_ordinal)
    }

    fn container_elements(
        &self,
        container: &ValueSlot,
    ) -> Result<VmContainerElements, VmHeapError> {
        self.inner.container_elements(container)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_string_and_bytes_forward_through_the_trait_object_with_exact_metadata() {
        let trace = HeapSpyTrace::default();
        let mut recording = RecordingVmHeap::new(RequestHeapLimits::default(), trace.clone());
        let string_tag =
            CompactTypeTag::try_from_type_index(37).expect("string type index fits compact tag");
        let bytes_tag =
            CompactTypeTag::try_from_type_index(41).expect("bytes type index fits compact tag");
        let string_flags = ValueFlags::new(3);
        let bytes_flags = ValueFlags::new(5);

        let heap: &mut dyn VmHeap = &mut recording;
        let string = heap
            .alloc_typed_string("err".to_string(), string_tag, string_flags)
            .expect("recording heap forwards typed string allocation");
        let bytes = heap
            .alloc_typed_bytes(vec![1, 2, 3], bytes_tag, bytes_flags)
            .expect("recording heap forwards typed bytes allocation");

        assert_eq!(string.compact_type_tag(), Some(string_tag));
        assert_eq!(string.flags(), string_flags);
        assert_eq!(heap.string_value(&string).unwrap(), "err");
        assert_eq!(bytes.compact_type_tag(), Some(bytes_tag));
        assert_eq!(bytes.flags(), bytes_flags);
        assert_eq!(heap.bytes_value(&bytes).unwrap(), vec![1, 2, 3]);

        heap.release_snapshot(&string).unwrap();
        heap.release_snapshot(&bytes).unwrap();
        let live_inventory = [string, bytes]
            .iter()
            .filter(|value| heap.validate_live(value).is_ok())
            .count();
        assert_eq!(live_inventory, 0, "released typed owners remain live");
        assert_eq!(
            trace
                .events()
                .iter()
                .filter(|event| matches!(event, HeapSpyEvent::ReleaseSnapshot { .. }))
                .count(),
            2
        );
    }
}

/// The K2 seam landed at `BytecodeRequestExecutionInput.heap`; what remains
/// unwired is the host spawn path, which keeps `heap: None` at every call
/// site. The Phase 2 harness therefore injects the recording heap through the
/// production driver input inside the production route composition and reports
/// this remaining host-side item here for the integrator.
pub fn host_passthrough_note() -> &'static str {
    "K2 heap seam landed (`BytecodeRequestExecutionInput.heap`); the host request-entry \
     passthrough (`RuntimeHost::spawn_bytecode_request` -> `request_entry/assembly.rs` -> drive) \
     still hard-codes `heap: None` and is outside the P2G write boundary. The VCP harness injects \
     `RecordingVmHeap` through the production driver input via the host's own production route \
     admission (`BytecodeDeploymentRegistry::route`), not through a second executor."
}
