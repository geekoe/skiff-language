use std::fmt;

use skiff_runtime_model::vm_heap::VmHeapError;
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};

/// Why roots left the pending owner without being restored to the VM fiber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootDisposition {
    Cancelled,
    Deadline,
    InternalStop,
    PublicationFailed,
    OwnerAbandoned,
}

/// Owner-specific storage behind a [`RootEscrow`].
///
/// The backing must register every `ValueSlot` root before it is passed to
/// [`RootEscrow::new`]. It may retain stable owner handles, but it must not
/// retain a heap borrow, heap mutex guard, movable object pointer, or native
/// poll/future state. Both terminal methods consume the backing so restoration
/// and dropping cannot both happen.
pub trait RootEscrowBacking: VmRootSource + Send + 'static {
    fn root_count(&self) -> usize;

    /// Atomically returns every registered root to its original VM owner.
    fn restore_roots(self: Box<Self>);

    /// Executes the owner-specific drop/release plan for every registered root.
    fn drop_roots(self: Box<Self>, disposition: RootDisposition);
}

/// Roots transferred out of a runnable fiber before a host completion handle
/// becomes visible.
///
/// The escrow has exactly two explicit terminal paths: [`Self::restore`] and
/// [`Self::discard`]. Dropping it without choosing a path fails closed by
/// discarding the roots as an abandoned owner.
#[must_use = "a root escrow must be restored or discarded exactly once"]
pub struct RootEscrow {
    backing: Option<Box<dyn RootEscrowBacking>>,
    root_count: usize,
}

impl RootEscrow {
    pub fn new(backing: Box<dyn RootEscrowBacking>) -> Self {
        let root_count = backing.root_count();
        Self {
            backing: Some(backing),
            root_count,
        }
    }

    pub const fn root_count(&self) -> usize {
        self.root_count
    }

    pub const fn is_empty(&self) -> bool {
        self.root_count == 0
    }

    pub fn restore(mut self) {
        let backing = self
            .backing
            .take()
            .expect("root escrow terminal transition is unique");
        backing.restore_roots();
    }

    pub fn discard(mut self, disposition: RootDisposition) {
        let backing = self
            .backing
            .take()
            .expect("root escrow terminal transition is unique");
        backing.drop_roots(disposition);
    }
}

impl VmRootSource for RootEscrow {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.backing
            .as_ref()
            .expect("live root escrow retains its backing")
            .visit_roots(visitor)
    }
}

impl fmt::Debug for RootEscrow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RootEscrow")
            .field("root_count", &self.root_count)
            .field("live", &self.backing.is_some())
            .finish()
    }
}

impl Drop for RootEscrow {
    fn drop(&mut self) {
        if let Some(backing) = self.backing.take() {
            backing.drop_roots(RootDisposition::OwnerAbandoned);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };

    use super::{RootDisposition, RootEscrow, RootEscrowBacking};

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Event {
        Restored,
        Dropped(RootDisposition),
    }

    struct RecordingRoots {
        roots: Box<[ValueSlot]>,
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl VmRootSource for RecordingRoots {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            for root in &self.roots {
                visitor.visit_root(root)?;
            }
            Ok(())
        }
    }

    impl RootEscrowBacking for RecordingRoots {
        fn root_count(&self) -> usize {
            self.roots.len()
        }

        fn restore_roots(self: Box<Self>) {
            self.events.lock().unwrap().push(Event::Restored);
        }

        fn drop_roots(self: Box<Self>, disposition: RootDisposition) {
            self.events
                .lock()
                .unwrap()
                .push(Event::Dropped(disposition));
        }
    }

    fn escrow(events: &Arc<Mutex<Vec<Event>>>) -> RootEscrow {
        RootEscrow::new(Box::new(RecordingRoots {
            roots: vec![ValueSlot::integer(1), ValueSlot::integer(2)].into_boxed_slice(),
            events: Arc::clone(events),
        }))
    }

    #[test]
    fn explicit_restore_is_exactly_once() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let roots = escrow(&events);
        assert_eq!(roots.root_count(), 2);

        roots.restore();

        assert_eq!(*events.lock().unwrap(), [Event::Restored]);
    }

    #[test]
    fn explicit_discard_is_exactly_once() {
        let events = Arc::new(Mutex::new(Vec::new()));
        escrow(&events).discard(RootDisposition::Cancelled);

        assert_eq!(
            *events.lock().unwrap(),
            [Event::Dropped(RootDisposition::Cancelled)]
        );
    }

    #[test]
    fn abandoned_escrow_fails_closed() {
        let events = Arc::new(Mutex::new(Vec::new()));
        drop(escrow(&events));

        assert_eq!(
            *events.lock().unwrap(),
            [Event::Dropped(RootDisposition::OwnerAbandoned)]
        );
    }
}
