use std::fmt;

use skiff_runtime_linked_bytecode::ConstantIndex;
use skiff_runtime_model::vm_value::ValueSlot;

/// Immutable values materialized from the verified frozen constant graph.
///
/// Fields and construction are private to the verifier. Every future non-empty
/// value is represented by a [`ValueSlot`] of kind `ConstRef`; that handle is
/// meaningful only together with the same pinned
/// [`crate::VerifiedLinkedBytecodeImage`]. The current conservative checkpoint can
/// construct only an empty heap. This type never accepts values or handles
/// supplied by a caller.
///
/// ```compile_fail
/// use skiff_runtime_bytecode_verifier::VerifiedConstantHeap;
/// use skiff_runtime_model::vm_value::ValueSlot;
///
/// fn extract_values(heap: &VerifiedConstantHeap) -> &[ValueSlot] {
///     &heap.values
/// }
/// ```
pub struct VerifiedConstantHeap {
    pub(super) values: Box<[ValueSlot]>,
    pub(super) _seal: VerifiedConstantHeapSeal,
}

#[derive(Debug)]
pub(super) struct VerifiedConstantHeapSeal;

impl fmt::Debug for VerifiedConstantHeap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedConstantHeap")
            .field("len", &self.values.len())
            .finish_non_exhaustive()
    }
}

impl VerifiedConstantHeap {
    /// Returns one verified constant value by its image-local index.
    pub fn get(&self, index: ConstantIndex) -> Option<ValueSlot> {
        let index = usize::try_from(index.get()).ok()?;
        self.values.get(index).copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
