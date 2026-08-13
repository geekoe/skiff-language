use std::{
    fmt,
    sync::{Arc, Mutex},
};

use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    service_error::CatchIdentity,
    vm_heap::{VmHeap, VmHeapError, VmHeapPathSegment, VmMapEntry, VmRecordField},
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
    /// The Phase 1 heap trait exposes a single-phase mutation primitive. The
    /// recorded COW fact is the post-call root handle, which cannot change
    /// until K2 replaces it with `prepare_writable_path`/`commit_writable_path`
    /// returning a replacement root.
    SetWritablePath {
        root_before: SpySlot,
        root_after: SpySlot,
        segments: usize,
        selectors: usize,
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
/// harness observes the exact share/COW/drop primitive sequence.
pub struct RecordingVmHeap {
    inner: Box<RequestVmHeap>,
    trace: HeapSpyTrace,
}

impl RecordingVmHeap {
    pub fn new(limits: RequestHeapLimits, trace: HeapSpyTrace) -> Self {
        Self {
            inner: Box::new(RequestVmHeap::new(limits)),
            trace,
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

    fn alloc_bytes(&mut self, value: Vec<u8>) -> Result<ValueSlot, VmHeapError> {
        self.inner.alloc_bytes(value)
    }

    fn alloc_string(&mut self, value: String) -> Result<ValueSlot, VmHeapError> {
        self.inner.alloc_string(value)
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

    fn set_writable_path(
        &mut self,
        root: &ValueSlot,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
        value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        let root_before = SpySlot::of(*root);
        self.inner
            .set_writable_path(root, segments, selectors, value)?;
        let root_after = SpySlot::of(*root);
        self.trace.record(HeapSpyEvent::SetWritablePath {
            root_before,
            root_after,
            segments: segments.len(),
            selectors: selectors.len(),
        });
        Ok(())
    }

    fn get_dense_field(
        &self,
        record: &ValueSlot,
        field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        self.inner.get_dense_field(record, field_ordinal)
    }
}

/// The precise seam K2 must land before the VCP internal-fact obligations can
/// close. This text is asserted verbatim by the harness so the expected-red
/// message always names the exact missing write-boundary item.
pub fn heap_spy_seam_requirement() -> &'static str {
    "K2 seam missing: `BytecodeRequestExecutionInput` / `drive_runtime_bytecode_request` must accept \
     `heap: Option<Box<dyn VmHeap + Send>>` (injecting a recording heap instead of constructing \
     `RequestVmHeap` internally), and the host request-entry path (`request_entry/assembly.rs` -> \
     drive) must pass that heap through so `RuntimeHost::spawn_bytecode_request` callers can inject \
     `RecordingVmHeap`; additionally the model trait must expose two-phase \
     prepare_writable_path/commit_writable_path returning a replacement root so the spy can prove \
     the exact share/COW/drop sequence. Until then the VCP internal facts stay expected-red."
}
