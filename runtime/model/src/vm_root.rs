//! Root enumeration shared by VM fibers and external scheduler owners.

use crate::{vm_heap::VmHeapError, vm_value::ValueSlot};

/// Receives stable value slots during a root walk.
///
/// Sources may include immediates; the visitor decides which slot kinds refer
/// to its heap. The borrowed slot cannot be retained or mutated through this
/// interface.
pub trait VmRootVisitor {
    fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError>;
}

/// A holder of VM roots, such as a frame, pending owner, adapter, or stream.
///
/// Implementations must enumerate every owned slot exactly once for the
/// duration of a safepoint. Moving a root between sources is an owner protocol
/// outside this read-only walk; no public ownership token is minted here.
pub trait VmRootSource {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError>;
}
