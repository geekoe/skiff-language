use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex, MutexGuard, Weak},
};

use skiff_runtime_model::{
    vm_heap::VmHeapError,
    vm_root::{VmRootSource, VmRootVisitor},
};

use crate::owner_inventory::{OwnerCreationError, ResourceOwnerLease, ResourceOwnerRegistration};

/// Why a request resource left its table-owned live state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceTermination {
    ExplicitRelease,
    RequestNotStarted,
    RequestScopeClosed,
    OwnerAbandoned,
}

/// Provider state owned exclusively by one [`RequestResourceTable`] entry.
///
/// Implementations must enumerate every VM root retained by the resource.
/// `visit_roots` runs under the table lock at a scheduler safepoint and must
/// obey the crate-level non-blocking, non-reentrant root-walk contract.
/// `terminate` runs after the entry was removed and tombstoned and after the
/// table lock was released. Consuming `self` makes the provider terminal
/// transition unique.
pub trait RequestResourceState: VmRootSource + Send + 'static {
    fn terminate(self: Box<Self>, termination: RequestResourceTermination);
}

struct RequestResourceOwnerIdentity;

#[derive(Clone)]
struct RequestResourceOwner(Weak<RequestResourceOwnerIdentity>);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestResourceSlot(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestResourceGeneration(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestResourceKey {
    slot: RequestResourceSlot,
    generation: RequestResourceGeneration,
}

/// Non-owning route to one exact request resource generation.
///
/// Its owner, slot and generation are intentionally opaque. Cloning a route
/// never clones the resource state, inventory lease or table authority.
#[derive(Clone)]
pub struct RequestResourceHandle {
    owner: RequestResourceOwner,
    key: RequestResourceKey,
}

impl PartialEq for RequestResourceHandle {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.owner.0, &other.owner.0) && self.key == other.key
    }
}

impl Eq for RequestResourceHandle {}

impl fmt::Debug for RequestResourceHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestResourceHandle")
            .finish_non_exhaustive()
    }
}

/// Successful disposition of an explicit resource release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceRelease {
    Released,
    AlreadyReleased,
}

/// Fail-closed rejection of a resource route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceLookupError {
    WrongOwner,
    UnknownSlot,
    StaleGeneration,
}

impl fmt::Display for RequestResourceLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongOwner => "request resource belongs to a different owner",
            Self::UnknownSlot => "request resource slot is unknown",
            Self::StaleGeneration => "request resource generation is stale",
        })
    }
}

impl std::error::Error for RequestResourceLookupError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceRegistrationError {
    OwnerCreation(OwnerCreationError),
    TableClosed,
    SlotSpaceExhausted,
}

impl fmt::Display for RequestResourceRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerCreation(error) => error.fmt(formatter),
            Self::TableClosed => formatter.write_str("request resource table is closed"),
            Self::SlotSpaceExhausted => {
                formatter.write_str("request resource slot space is exhausted")
            }
        }
    }
}

impl std::error::Error for RequestResourceRegistrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OwnerCreation(error) => Some(error),
            Self::TableClosed | Self::SlotSpaceExhausted => None,
        }
    }
}

/// A failed admission returns the still-owned provider state to its caller.
pub struct RequestResourceRegistrationFailure {
    reason: RequestResourceRegistrationError,
    state: Box<dyn RequestResourceState>,
}

impl RequestResourceRegistrationFailure {
    pub const fn reason(&self) -> RequestResourceRegistrationError {
        self.reason
    }

    pub fn into_state(self) -> Box<dyn RequestResourceState> {
        self.state
    }
}

impl fmt::Debug for RequestResourceRegistrationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestResourceRegistrationFailure")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RequestResourceRegistrationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(formatter)
    }
}

impl std::error::Error for RequestResourceRegistrationFailure {}

struct RequestResourceEntry {
    state: Box<dyn RequestResourceState>,
    owner_lease: Option<ResourceOwnerLease>,
}

impl RequestResourceEntry {
    fn unarmed(state: Box<dyn RequestResourceState>) -> Self {
        Self {
            state,
            owner_lease: None,
        }
    }

    fn arm(&mut self, owner_lease: ResourceOwnerLease) {
        assert!(
            self.owner_lease.replace(owner_lease).is_none(),
            "a request resource entry arms exactly once"
        );
    }

    fn terminate(self, termination: RequestResourceTermination) {
        let Self { state, owner_lease } = self;
        state.terminate(termination);
        drop(owner_lease);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestResourceTablePhase {
    Open,
    Closed,
}

struct RequestResourceTableState {
    phase: RequestResourceTablePhase,
    next_slot: u64,
    free_slots: Vec<RequestResourceSlot>,
    generations: HashMap<RequestResourceSlot, RequestResourceGeneration>,
    closed: HashSet<RequestResourceKey>,
    entries: HashMap<RequestResourceSlot, (RequestResourceGeneration, RequestResourceEntry)>,
}

impl RequestResourceTableState {
    fn open() -> Self {
        Self {
            phase: RequestResourceTablePhase::Open,
            next_slot: 1,
            free_slots: Vec::new(),
            generations: HashMap::new(),
            closed: HashSet::new(),
            entries: HashMap::new(),
        }
    }

    fn mint_key(&mut self) -> Result<RequestResourceKey, RequestResourceRegistrationError> {
        while let Some(slot) = self.free_slots.pop() {
            let previous = self
                .generations
                .get(&slot)
                .copied()
                .expect("a reusable resource slot has an issued generation");
            if let Some(generation) = previous.0.checked_add(1) {
                let generation = RequestResourceGeneration(generation);
                self.generations.insert(slot, generation);
                return Ok(RequestResourceKey { slot, generation });
            }
        }

        if self.next_slot == 0 {
            return Err(RequestResourceRegistrationError::SlotSpaceExhausted);
        }
        let slot = RequestResourceSlot(self.next_slot);
        self.next_slot = self.next_slot.checked_add(1).unwrap_or(0);
        let generation = RequestResourceGeneration(1);
        self.generations.insert(slot, generation);
        Ok(RequestResourceKey { slot, generation })
    }

    fn validate_key(&self, key: RequestResourceKey) -> Result<(), RequestResourceLookupError> {
        match self.entries.get(&key.slot) {
            Some((generation, _)) if *generation == key.generation => Ok(()),
            Some(_) => Err(RequestResourceLookupError::StaleGeneration),
            None if self.generations.contains_key(&key.slot) => {
                Err(RequestResourceLookupError::StaleGeneration)
            }
            None => Err(RequestResourceLookupError::UnknownSlot),
        }
    }
}

struct RequestResourceTableShared {
    owner: Arc<RequestResourceOwnerIdentity>,
    state: Mutex<RequestResourceTableState>,
}

impl RequestResourceTableShared {
    fn lock(&self) -> MutexGuard<'_, RequestResourceTableState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn owns(&self, handle: &RequestResourceHandle) -> bool {
        Weak::ptr_eq(&handle.owner.0, &Arc::downgrade(&self.owner))
    }

    fn drain(
        &self,
        termination: RequestResourceTermination,
    ) -> Vec<(RequestResourceEntry, RequestResourceTermination)> {
        let mut state = self.lock();
        state.phase = RequestResourceTablePhase::Closed;
        let removed: Vec<_> = state.entries.drain().collect();
        for (slot, (generation, _)) in &removed {
            state.closed.insert(RequestResourceKey {
                slot: *slot,
                generation: *generation,
            });
            state.free_slots.push(*slot);
        }
        removed
            .into_iter()
            .map(|(_, (_, entry))| (entry, termination))
            .collect()
    }
}

impl Drop for RequestResourceTableShared {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries: Vec<_> = state.entries.drain().map(|(_, (_, entry))| entry).collect();
        for entry in entries {
            entry.terminate(RequestResourceTermination::OwnerAbandoned);
        }
    }
}

/// The single scheduler-owned resource authority for one request.
///
/// Construction is scheduler-private and requires the resource registration
/// from the same [`RequestExecutionContext`](crate::RequestExecutionContext)
/// owner inventory. Clones share one table capability; handles hold only a
/// weak owner identity and cannot keep this table or a provider alive.
#[derive(Clone)]
pub struct RequestResourceTable {
    shared: Arc<RequestResourceTableShared>,
    owner_registration: ResourceOwnerRegistration,
}

impl RequestResourceTable {
    pub(crate) fn new(owner_registration: ResourceOwnerRegistration) -> Self {
        Self {
            shared: Arc::new(RequestResourceTableShared {
                owner: Arc::new(RequestResourceOwnerIdentity),
                state: Mutex::new(RequestResourceTableState::open()),
            }),
            owner_registration,
        }
    }

    /// Installs one prepared provider as the table's sole strong owner.
    ///
    /// The inventory guard is acquired before the table lock. The entry is
    /// inserted unarmed, the infallible inventory commit mints its private
    /// lease, and the entry is armed before the table lock is released.
    pub fn register(
        &self,
        state: Box<dyn RequestResourceState>,
    ) -> Result<RequestResourceHandle, RequestResourceRegistrationFailure> {
        let owner = match self.owner_registration.prepare() {
            Ok(owner) => owner,
            Err(reason) => {
                return Err(RequestResourceRegistrationFailure {
                    reason: RequestResourceRegistrationError::OwnerCreation(reason),
                    state,
                });
            }
        };
        let mut table = self.shared.lock();
        if table.phase == RequestResourceTablePhase::Closed {
            drop(table);
            drop(owner);
            return Err(RequestResourceRegistrationFailure {
                reason: RequestResourceRegistrationError::TableClosed,
                state,
            });
        }
        let key = match table.mint_key() {
            Ok(key) => key,
            Err(reason) => {
                drop(table);
                drop(owner);
                return Err(RequestResourceRegistrationFailure { reason, state });
            }
        };
        let previous = table.entries.insert(
            key.slot,
            (key.generation, RequestResourceEntry::unarmed(state)),
        );
        assert!(previous.is_none(), "a minted resource slot is vacant");
        let lease = owner.commit();
        table
            .entries
            .get_mut(&key.slot)
            .expect("the freshly inserted resource entry remains present")
            .1
            .arm(lease);
        drop(table);
        Ok(RequestResourceHandle {
            owner: RequestResourceOwner(Arc::downgrade(&self.shared.owner)),
            key,
        })
    }

    /// Validates that the route denotes this table's currently live exact
    /// generation, without exposing or cloning provider state.
    pub fn validate(
        &self,
        handle: &RequestResourceHandle,
    ) -> Result<(), RequestResourceLookupError> {
        if !self.shared.owns(handle) {
            return Err(RequestResourceLookupError::WrongOwner);
        }
        self.shared.lock().validate_key(handle.key)
    }

    /// Removes and tombstones an exact live entry under the table lock, then
    /// terminates it and releases its private inventory lease outside it.
    pub fn release(
        &self,
        handle: &RequestResourceHandle,
    ) -> Result<RequestResourceRelease, RequestResourceLookupError> {
        if !self.shared.owns(handle) {
            return Err(RequestResourceLookupError::WrongOwner);
        }
        let entry = {
            let mut table = self.shared.lock();
            if table.closed.contains(&handle.key) {
                return Ok(RequestResourceRelease::AlreadyReleased);
            }
            table.validate_key(handle.key)?;
            let (generation, entry) = table
                .entries
                .remove(&handle.key.slot)
                .expect("a validated resource entry remains live");
            assert_eq!(generation, handle.key.generation);
            table.closed.insert(handle.key);
            table.free_slots.push(handle.key.slot);
            entry
        };
        entry.terminate(RequestResourceTermination::ExplicitRelease);
        Ok(RequestResourceRelease::Released)
    }

    /// Closes the table permanently and terminates every live resource once.
    ///
    /// The drain and tombstones are completed under the lock. Provider
    /// terminal code and inventory lease drops run only after the lock is
    /// released. A repeated close is an idempotent no-op.
    pub fn close_all(&self, termination: RequestResourceTermination) -> usize {
        let entries = self.shared.drain(termination);
        let count = entries.len();
        for (entry, termination) in entries {
            entry.terminate(termination);
        }
        count
    }

    pub fn live_count(&self) -> usize {
        self.shared.lock().entries.len()
    }

    #[cfg(test)]
    fn owner_weak_for_test(&self) -> Weak<RequestResourceTableShared> {
        Arc::downgrade(&self.shared)
    }
}

impl VmRootSource for RequestResourceTable {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        let table = self.shared.lock();
        for (_, entry) in table.entries.values() {
            entry.state.visit_roots(visitor)?;
        }
        Ok(())
    }
}

impl fmt::Debug for RequestResourceTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestResourceTable")
            .field("live_count", &self.live_count())
            .finish_non_exhaustive()
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

    use super::*;
    use crate::owner_inventory::RequestExecutionOwnerInventory;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Event {
        Terminated(RequestResourceTermination),
        TableUnlocked,
    }

    struct RecordingResource {
        roots: Box<[ValueSlot]>,
        events: Arc<Mutex<Vec<Event>>>,
        table: Option<Weak<RequestResourceTableShared>>,
    }

    impl VmRootSource for RecordingResource {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            for root in &self.roots {
                visitor.visit_root(root)?;
            }
            Ok(())
        }
    }

    impl RequestResourceState for RecordingResource {
        fn terminate(self: Box<Self>, termination: RequestResourceTermination) {
            self.events
                .lock()
                .unwrap()
                .push(Event::Terminated(termination));
            if let Some(table) = self.table.as_ref().and_then(Weak::upgrade) {
                let _guard = table
                    .state
                    .try_lock()
                    .expect("termination runs outside table lock");
                self.events.lock().unwrap().push(Event::TableUnlocked);
            }
        }
    }

    fn table() -> (
        RequestResourceTable,
        crate::owner_inventory::RequestExecutionOwnerInventoryFreezePermit,
    ) {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        (RequestResourceTable::new(registrations.resource()), freeze)
    }

    fn resource(events: &Arc<Mutex<Vec<Event>>>) -> Box<dyn RequestResourceState> {
        Box::new(RecordingResource {
            roots: Box::new([]),
            events: Arc::clone(events),
            table: None,
        })
    }

    #[test]
    fn exact_handle_rejects_wrong_owner() {
        let (left, left_freeze) = table();
        let (right, right_freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = right.register(resource(&events)).unwrap();

        assert_eq!(
            left.validate(&handle),
            Err(RequestResourceLookupError::WrongOwner)
        );
        assert_eq!(
            left.release(&handle),
            Err(RequestResourceLookupError::WrongOwner)
        );

        left.close_all(RequestResourceTermination::RequestScopeClosed);
        right.close_all(RequestResourceTermination::RequestScopeClosed);
        assert_eq!(left_freeze.freeze().resource.current, 0);
        assert_eq!(right_freeze.freeze().resource.current, 0);
    }

    #[test]
    fn reused_slot_rejects_stale_generation() {
        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let stale = table.register(resource(&events)).unwrap();
        assert_eq!(
            table.release(&stale).unwrap(),
            RequestResourceRelease::Released
        );
        let current = table.register(resource(&events)).unwrap();

        assert_ne!(stale, current);
        assert_eq!(
            table.validate(&stale),
            Err(RequestResourceLookupError::StaleGeneration)
        );
        assert!(table.validate(&current).is_ok());

        table.close_all(RequestResourceTermination::RequestScopeClosed);
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn duplicate_release_is_an_idempotent_no_op() {
        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = table.register(resource(&events)).unwrap();

        assert_eq!(
            table.release(&handle).unwrap(),
            RequestResourceRelease::Released
        );
        assert_eq!(
            table.release(&handle).unwrap(),
            RequestResourceRelease::AlreadyReleased
        );
        assert_eq!(
            *events.lock().unwrap(),
            [Event::Terminated(
                RequestResourceTermination::ExplicitRelease
            )]
        );

        table.close_all(RequestResourceTermination::RequestScopeClosed);
        let snapshot = freeze.freeze();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
    }

    #[test]
    fn close_all_tombstones_before_exact_once_termination_outside_lock() {
        let (table, freeze) = table();
        let first_events = Arc::new(Mutex::new(Vec::new()));
        let second_events = Arc::new(Mutex::new(Vec::new()));
        let first = table
            .register(Box::new(RecordingResource {
                roots: Box::new([]),
                events: Arc::clone(&first_events),
                table: Some(table.owner_weak_for_test()),
            }))
            .unwrap();
        let second = table
            .register(Box::new(RecordingResource {
                roots: Box::new([]),
                events: Arc::clone(&second_events),
                table: Some(table.owner_weak_for_test()),
            }))
            .unwrap();

        assert_eq!(
            table.close_all(RequestResourceTermination::RequestScopeClosed),
            2
        );
        assert_eq!(
            table.close_all(RequestResourceTermination::RequestScopeClosed),
            0
        );
        assert_eq!(
            table.release(&first).unwrap(),
            RequestResourceRelease::AlreadyReleased
        );
        assert_eq!(
            table.release(&second).unwrap(),
            RequestResourceRelease::AlreadyReleased
        );
        assert_eq!(
            *first_events.lock().unwrap(),
            [
                Event::Terminated(RequestResourceTermination::RequestScopeClosed),
                Event::TableUnlocked,
            ]
        );
        assert_eq!(
            *second_events.lock().unwrap(),
            [
                Event::Terminated(RequestResourceTermination::RequestScopeClosed),
                Event::TableUnlocked,
            ]
        );
        let snapshot = freeze.freeze();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
    }

    #[test]
    fn closed_table_returns_unadmitted_state_and_never_mints_another_lease() {
        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        table.close_all(RequestResourceTermination::RequestScopeClosed);

        let failure = table.register(resource(&events)).unwrap_err();
        assert_eq!(
            failure.reason(),
            RequestResourceRegistrationError::TableClosed
        );
        drop(failure.into_state());
        let snapshot = freeze.freeze();
        assert_eq!(snapshot.resource.current, 0);
        assert!(!snapshot.resource.ever_created);
        assert!(events.lock().unwrap().is_empty());
    }

    struct CollectRoots(Vec<ValueSlot>);

    impl VmRootVisitor for CollectRoots {
        fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError> {
            self.0.push(*root);
            Ok(())
        }
    }

    #[test]
    fn table_pins_and_enumerates_only_live_resource_roots() {
        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let first = table
            .register(Box::new(RecordingResource {
                roots: vec![ValueSlot::integer(1)].into_boxed_slice(),
                events: Arc::clone(&events),
                table: None,
            }))
            .unwrap();
        table
            .register(Box::new(RecordingResource {
                roots: vec![ValueSlot::integer(2)].into_boxed_slice(),
                events: Arc::clone(&events),
                table: None,
            }))
            .unwrap();
        table.release(&first).unwrap();

        let mut visitor = CollectRoots(Vec::new());
        table.visit_roots(&mut visitor).unwrap();
        assert_eq!(visitor.0.len(), 1);
        assert!(visitor.0[0] == ValueSlot::integer(2));

        table.close_all(RequestResourceTermination::RequestScopeClosed);
        assert_eq!(freeze.freeze().resource.current, 0);
    }
}
