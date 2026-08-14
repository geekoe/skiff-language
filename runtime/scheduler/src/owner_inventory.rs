use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use skiff_runtime_model::{
    bytecode_execution_observation::{FrozenOwnerDomain, RequestExecutionOwnerInventorySnapshot},
    vm_heap::VmHeap,
    vm_root::VmRootSource,
};
use skiff_runtime_vm::VmBudget;

use crate::{
    BytecodeScheduler, BytecodeSchedulerError, BytecodeSchedulerOutcome, BytecodeSchedulerPorts,
    BytecodeUnit, RequestResourceTable, RequestResourceTermination,
};

/// One counted owner domain inside a request execution inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OwnerDomain {
    Pending,
    Resource,
    Child,
}

impl fmt::Display for OwnerDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "pending",
            Self::Resource => "resource",
            Self::Child => "child",
        })
    }
}

/// Why an owner creation was rejected, without the domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerCreationErrorKind {
    InventoryFrozen,
    CountOverflow,
}

impl fmt::Display for OwnerCreationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InventoryFrozen => "request owner inventory is frozen",
            Self::CountOverflow => "request owner inventory count overflowed",
        })
    }
}

/// Domain-tagged rejection of one owner creation.
///
/// The error carries both the domain that was being created and the machine
/// reason. `InventoryFrozen` is checked, and `CountOverflow` is pre-checked,
/// inside the inventory lock at `prepare` time; `commit` never fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerCreationError {
    domain: OwnerDomain,
    kind: OwnerCreationErrorKind,
}

impl OwnerCreationError {
    pub const fn domain(self) -> OwnerDomain {
        self.domain
    }

    pub const fn kind(self) -> OwnerCreationErrorKind {
        self.kind
    }

    const fn frozen(domain: OwnerDomain) -> Self {
        Self {
            domain,
            kind: OwnerCreationErrorKind::InventoryFrozen,
        }
    }

    const fn count_overflow(domain: OwnerDomain) -> Self {
        Self {
            domain,
            kind: OwnerCreationErrorKind::CountOverflow,
        }
    }
}

impl fmt::Display for OwnerCreationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} owner creation failed: {}",
            self.domain, self.kind
        )
    }
}

impl std::error::Error for OwnerCreationError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DomainState {
    current: u64,
    ever_created: bool,
}

impl DomainState {
    const NEVER_CREATED: Self = Self {
        current: 0,
        ever_created: false,
    };
}

impl From<DomainState> for FrozenOwnerDomain {
    fn from(state: DomainState) -> Self {
        Self {
            current: state.current,
            ever_created: state.ever_created,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InventoryPhase {
    Open,
    Frozen,
}

struct InventoryState {
    phase: InventoryPhase,
    pending: DomainState,
    resource: DomainState,
    child: DomainState,
}

impl InventoryState {
    const fn open() -> Self {
        Self {
            phase: InventoryPhase::Open,
            pending: DomainState::NEVER_CREATED,
            resource: DomainState::NEVER_CREATED,
            child: DomainState::NEVER_CREATED,
        }
    }

    const fn domain(&self, domain: OwnerDomain) -> DomainState {
        match domain {
            OwnerDomain::Pending => self.pending,
            OwnerDomain::Resource => self.resource,
            OwnerDomain::Child => self.child,
        }
    }

    fn domain_mut(&mut self, domain: OwnerDomain) -> &mut DomainState {
        match domain {
            OwnerDomain::Pending => &mut self.pending,
            OwnerDomain::Resource => &mut self.resource,
            OwnerDomain::Child => &mut self.child,
        }
    }
}

struct InventoryShared(Mutex<InventoryState>);

impl InventoryShared {
    fn lock(&self) -> MutexGuard<'_, InventoryState> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Validates one creation inside the inventory lock.
    ///
    /// Both rejections are decided here: a frozen inventory can never accept
    /// another owner, and a domain whose `current` already saturated
    /// `u64::MAX` cannot overflow at commit. The returned guard keeps the
    /// inventory locked from `prepare` through `commit`.
    fn prepare(
        self: &Arc<Self>,
        domain: OwnerDomain,
    ) -> Result<OwnerCreationGuard<'_>, OwnerCreationError> {
        let state = self.lock();
        if state.phase == InventoryPhase::Frozen {
            return Err(OwnerCreationError::frozen(domain));
        }
        if state.domain(domain).current == u64::MAX {
            return Err(OwnerCreationError::count_overflow(domain));
        }
        Ok(OwnerCreationGuard {
            inventory: self,
            domain,
            state,
        })
    }
}

/// One validated creation site holding the inventory lock.
///
/// The guard deliberately has no install closure: between `prepare` and
/// `commit` the caller can allocate and publish an unarmed container
/// placeholder, and `commit` then performs only the infallible count and
/// ever-created update before releasing the lock. No caller code runs while
/// the inventory lock is held or at commit time.
struct OwnerCreationGuard<'a> {
    inventory: &'a Arc<InventoryShared>,
    domain: OwnerDomain,
    state: MutexGuard<'a, InventoryState>,
}

impl OwnerCreationGuard<'_> {
    /// Applies the prepared increment and mints the typed lease.
    ///
    /// Infallible: `prepare` already rejected both frozen inventories and
    /// saturated counts, so `current += 1` cannot overflow here.
    fn commit(self) -> OwnerLease {
        let (inventory, domain) = (self.inventory, self.domain);
        let mut state = self.state;
        let domain_state = state.domain_mut(domain);
        domain_state.current += 1;
        domain_state.ever_created = true;
        drop(state);
        OwnerLease {
            inventory: Arc::clone(inventory),
            domain,
        }
    }
}

struct OwnerLease {
    inventory: Arc<InventoryShared>,
    domain: OwnerDomain,
}

impl Drop for OwnerLease {
    fn drop(&mut self) {
        let mut state = self.inventory.lock();
        let domain = state.domain_mut(self.domain);
        domain.current = domain
            .current
            .checked_sub(1)
            .expect("an owner lease releases exactly one installed owner");
    }
}

/// Cloneable authority to create pending owners inside one request's owner
/// inventory.
///
/// The handle only admits [`PendingRegistry`](crate::PendingRegistry)
/// construction; it cannot mint leases, install owners or freeze the
/// inventory. It stays bound to the inventory it was cloned from, so a
/// registry built from it can never mix ownership with another request.
#[derive(Clone)]
pub struct PendingOwnerRegistration(Arc<InventoryShared>);

#[derive(Clone)]
pub(crate) struct ResourceOwnerRegistration(Arc<InventoryShared>);

#[derive(Clone)]
pub(crate) struct ChildOwnerRegistration(Arc<InventoryShared>);

pub(crate) struct PendingOwnerLease(OwnerLease);
pub(crate) struct ResourceOwnerLease(OwnerLease);
pub(crate) struct ChildOwnerLease(OwnerLease);

#[must_use = "a pending creation guard must be committed or explicitly aborted"]
pub(crate) struct PendingOwnerCreationGuard<'a>(OwnerCreationGuard<'a>);
#[must_use = "a resource creation guard must be committed or explicitly aborted"]
pub(crate) struct ResourceOwnerCreationGuard<'a>(OwnerCreationGuard<'a>);
#[must_use = "a child creation guard must be committed or explicitly aborted"]
pub(crate) struct ChildOwnerCreationGuard<'a>(OwnerCreationGuard<'a>);

macro_rules! opaque_lease_debug {
    ($lease:ident) => {
        impl fmt::Debug for $lease {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($lease))
                    .finish_non_exhaustive()
            }
        }
    };
}

opaque_lease_debug!(PendingOwnerLease);
opaque_lease_debug!(ResourceOwnerLease);
opaque_lease_debug!(ChildOwnerLease);

macro_rules! opaque_guard_debug {
    ($guard:ident) => {
        impl fmt::Debug for $guard<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($guard))
                    .finish_non_exhaustive()
            }
        }
    };
}

opaque_guard_debug!(PendingOwnerCreationGuard);
opaque_guard_debug!(ResourceOwnerCreationGuard);
opaque_guard_debug!(ChildOwnerCreationGuard);

impl fmt::Debug for PendingOwnerRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingOwnerRegistration")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ResourceOwnerRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceOwnerRegistration")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ChildOwnerRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildOwnerRegistration")
            .finish_non_exhaustive()
    }
}

macro_rules! registration_guard {
    ($registration:ident, $guard:ident, $lease:ident, $domain:ident) => {
        impl $registration {
            pub(crate) fn prepare(&self) -> Result<$guard<'_>, OwnerCreationError> {
                self.0.prepare(OwnerDomain::$domain).map($guard)
            }
        }

        impl $guard<'_> {
            pub(crate) fn commit(self) -> $lease {
                $lease(self.0.commit())
            }
        }
    };
}

registration_guard!(
    PendingOwnerRegistration,
    PendingOwnerCreationGuard,
    PendingOwnerLease,
    Pending
);
registration_guard!(
    ResourceOwnerRegistration,
    ResourceOwnerCreationGuard,
    ResourceOwnerLease,
    Resource
);
registration_guard!(
    ChildOwnerRegistration,
    ChildOwnerCreationGuard,
    ChildOwnerLease,
    Child
);

#[derive(Clone)]
pub(crate) struct RequestExecutionOwnerRegistrations {
    pending: PendingOwnerRegistration,
    resource: ResourceOwnerRegistration,
    child: ChildOwnerRegistration,
}

impl RequestExecutionOwnerRegistrations {
    pub(crate) fn pending(&self) -> PendingOwnerRegistration {
        self.pending.clone()
    }

    pub(crate) fn resource(&self) -> ResourceOwnerRegistration {
        self.resource.clone()
    }

    pub(crate) fn child(&self) -> ChildOwnerRegistration {
        self.child.clone()
    }
}

/// One request's actual owner inventory and unique freeze authority.
///
/// This type intentionally has no `Default`, zero snapshot constructor, reset,
/// merge, count or freeze-by-reference operation. It is scheduler-private:
/// the only public entry point into one inventory is
/// [`RequestExecutionContext`].
pub(crate) struct RequestExecutionOwnerInventory {
    registrations: RequestExecutionOwnerRegistrations,
    freeze: RequestExecutionOwnerInventoryFreezePermit,
}

impl RequestExecutionOwnerInventory {
    pub(crate) fn open() -> Self {
        let shared = Arc::new(InventoryShared(Mutex::new(InventoryState::open())));
        Self {
            registrations: RequestExecutionOwnerRegistrations {
                pending: PendingOwnerRegistration(Arc::clone(&shared)),
                resource: ResourceOwnerRegistration(Arc::clone(&shared)),
                child: ChildOwnerRegistration(Arc::clone(&shared)),
            },
            freeze: RequestExecutionOwnerInventoryFreezePermit { shared },
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RequestExecutionOwnerRegistrations,
        RequestExecutionOwnerInventoryFreezePermit,
    ) {
        (self.registrations, self.freeze)
    }
}

pub(crate) struct RequestExecutionOwnerInventoryFreezePermit {
    shared: Arc<InventoryShared>,
}

impl RequestExecutionOwnerInventoryFreezePermit {
    pub(crate) fn freeze(self) -> RequestExecutionOwnerInventorySnapshot {
        let mut state = self.shared.lock();
        assert_eq!(
            state.phase,
            InventoryPhase::Open,
            "the unique inventory freeze permit cannot observe a prior freeze"
        );
        state.phase = InventoryPhase::Frozen;
        RequestExecutionOwnerInventorySnapshot {
            pending: state.pending.into(),
            resource: state.resource.into(),
            child: state.child.into(),
        }
    }
}

/// One non-cloneable, opaque, owner-bound synchronous bytecode execution
/// context.
///
/// This is the only public authority over one request's owner inventory. It
/// opens a fresh inventory, installs the root unit exactly once, and then
/// consumes itself either into a frozen `NotStarted` snapshot or into the
/// single scheduler drive, which freezes the actual `Started` snapshot on
/// every outcome. Its resource table and pending registration remain bound to
/// this inventory; it exposes no raw registration/guard/lease/install parts
/// and no freeze-by-reference, so callers cannot forge a carrier or mix owner
/// authority from two requests.
pub struct RequestExecutionContext<U: BytecodeUnit> {
    registrations: RequestExecutionOwnerRegistrations,
    freeze: Option<RequestExecutionOwnerInventoryFreezePermit>,
    resources: RequestResourceTable,
    root: Option<U>,
    ports: BytecodeSchedulerPorts<U>,
}

impl<U> RequestExecutionContext<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    /// Opens one fresh, unshared owner inventory for this execution.
    pub fn create(ports: BytecodeSchedulerPorts<U>) -> Self {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let resources = RequestResourceTable::new(registrations.resource());
        Self {
            registrations,
            freeze: Some(freeze),
            resources,
            root: None,
            ports,
        }
    }

    /// Clones the pending-owner registration bound to this context so the
    /// caller can build a [`BytecodeSchedulerPorts`] that owns a real
    /// [`PendingRegistry`](crate::PendingRegistry) before the first drive.
    ///
    /// The registration cannot mint or release owners on its own, and it
    /// never outlives the context's inventory.
    pub fn pending_registration(&self) -> PendingOwnerRegistration {
        self.registrations.pending()
    }

    /// Clones this request's single scheduler-owned resource table capability.
    ///
    /// The clone cannot outlive the request as an open authority: every
    /// consuming terminal path and abandoned-context drop closes the shared
    /// table before freezing the owner inventory.
    pub fn resource_table(&self) -> RequestResourceTable {
        self.resources.clone()
    }

    /// Replaces the scheduler ports installed at [`Self::create`].
    ///
    /// The caller typically builds the real ports from
    /// [`Self::pending_registration`] and installs them here before any
    /// drive; both ports and registration remain bound to this one context.
    pub fn with_ports(mut self, ports: BytecodeSchedulerPorts<U>) -> Self {
        self.ports = ports;
        self
    }

    /// Clones the current scheduler ports for a pending wake resume.
    pub fn ports(&self) -> BytecodeSchedulerPorts<U> {
        self.ports.clone()
    }

    /// Installs the root unit exactly once, without running any caller code.
    ///
    /// # Panics
    ///
    /// Panics when a root unit was already installed.
    pub fn install_root(&mut self, root: U) {
        assert!(
            self.root.is_none(),
            "the request execution context installs its root unit exactly once"
        );
        self.root = Some(root);
    }

    /// Consumes the context before any drive and freezes the `NotStarted`
    /// owner inventory snapshot.
    pub fn into_not_started(mut self) -> RequestExecutionOwnerInventorySnapshot {
        self.close_and_freeze(RequestResourceTermination::RequestNotStarted)
    }

    /// Consumes the context, runs the scheduler exactly once, and freezes the
    /// `Started` owner inventory snapshot on every outcome.
    ///
    /// # Panics
    ///
    /// Panics when no root unit was installed before driving.
    pub fn drive(
        mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> (
        Result<BytecodeSchedulerOutcome<U>, BytecodeSchedulerError>,
        RequestExecutionOwnerInventorySnapshot,
    ) {
        let root = self
            .root
            .take()
            .expect("the request execution context must install its root unit before driving");
        let scheduler =
            BytecodeScheduler::new(root, self.ports.clone(), self.registrations.child());
        let result = scheduler.run(heap, budget);
        let snapshot = self.close_and_freeze(RequestResourceTermination::RequestScopeClosed);
        (result, snapshot)
    }

    /// Drives the installed root unit exactly once without freezing the owner
    /// inventory.
    ///
    /// A `Parked` outcome consumes the scheduler: the suspended invocation
    /// chain moves into the pending registry and can only return through a
    /// completed [`PendingWake`](crate::PendingWake). A `Complete` or error
    /// outcome also consumes the scheduler and leaves the context ready for
    /// [`Self::freeze`].
    ///
    /// # Panics
    ///
    /// Panics when no root unit was installed before driving.
    pub fn start_drive(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeSchedulerOutcome<U>, BytecodeSchedulerError> {
        let root = self
            .root
            .take()
            .expect("the request execution context must install its root unit before driving");
        let scheduler =
            BytecodeScheduler::new(root, self.ports.clone(), self.registrations.child());
        scheduler.run(heap, budget)
    }

    /// Drives a scheduler restored from a completed pending wake exactly once,
    /// still without freezing the owner inventory.
    ///
    /// The caller restores the scheduler with
    /// [`BytecodeScheduler::resume_from_pending_wake`] and passes it here; a
    /// second `Parked` outcome is legal and suspends the chain again.
    pub fn resume_drive(
        &mut self,
        scheduler: BytecodeScheduler<U>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeSchedulerOutcome<U>, BytecodeSchedulerError> {
        scheduler.run(heap, budget)
    }

    /// Consumes the context and freezes the actual `Started` owner inventory
    /// snapshot exactly once, after the request reached its terminal outcome.
    ///
    /// # Panics
    ///
    /// Panics when the context was already frozen by
    /// [`Self::into_not_started`] or an earlier freeze.
    pub fn freeze(mut self) -> RequestExecutionOwnerInventorySnapshot {
        self.close_and_freeze(RequestResourceTermination::RequestScopeClosed)
    }

    fn close_and_freeze(
        &mut self,
        termination: RequestResourceTermination,
    ) -> RequestExecutionOwnerInventorySnapshot {
        self.resources.close_all(termination);
        self.freeze
            .take()
            .expect("the request execution context freezes its inventory exactly once")
            .freeze()
    }
}

impl<U: BytecodeUnit> Drop for RequestExecutionContext<U> {
    fn drop(&mut self) {
        self.resources
            .close_all(RequestResourceTermination::OwnerAbandoned);
        if let Some(freeze) = self.freeze.take() {
            let _ = freeze.freeze();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc,
        },
        time::Duration,
    };

    use skiff_runtime_model::{vm_heap::VmHeapError, vm_root::VmRootVisitor};
    use skiff_runtime_vm::VmFiber;

    use super::*;

    fn open_inventory() -> (
        RequestExecutionOwnerRegistrations,
        RequestExecutionOwnerInventoryFreezePermit,
    ) {
        RequestExecutionOwnerInventory::open().into_parts()
    }

    #[test]
    fn live_and_released_owners_freeze_actual_current_and_ever_created_facts() {
        let (registrations, freeze) = open_inventory();
        let pending = registrations.pending().prepare().unwrap().commit();
        let resource = registrations.resource().prepare().unwrap().commit();
        drop(resource);

        let snapshot = freeze.freeze();
        assert_eq!(snapshot.pending.current, 1);
        assert!(snapshot.pending.ever_created);
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert_eq!(snapshot.child.current, 0);
        assert!(!snapshot.child.ever_created);
        drop(pending);
        assert_eq!(snapshot.pending.current, 1);
    }

    #[test]
    fn freeze_first_rejects_creation_without_minting_a_lease() {
        let (registrations, freeze) = open_inventory();
        let snapshot = freeze.freeze();

        let error = registrations.child().prepare().unwrap_err();
        assert_eq!(error.domain(), OwnerDomain::Child);
        assert_eq!(error.kind(), OwnerCreationErrorKind::InventoryFrozen);
        assert_eq!(snapshot.child.current, 0);
        assert!(!snapshot.child.ever_created);
    }

    #[test]
    fn aborting_a_prepared_creation_preserves_never_created() {
        let (registrations, freeze) = open_inventory();

        drop(registrations.resource().prepare().unwrap());

        let snapshot = freeze.freeze();
        assert_eq!(snapshot.resource.current, 0);
        assert!(!snapshot.resource.ever_created);
    }

    #[test]
    fn prepare_commit_holds_the_inventory_lock_against_freeze() {
        let (registrations, freeze) = open_inventory();
        let (guard_tx, guard_rx) = mpsc::channel();
        let (commit_tx, commit_rx) = mpsc::channel();
        let creating = std::thread::spawn(move || {
            let registration = registrations.pending();
            let guard = registration.prepare().unwrap();
            guard_tx.send(()).unwrap();
            commit_rx.recv().unwrap();
            guard.commit()
        });
        guard_rx.recv().unwrap();

        let (snapshot_tx, snapshot_rx) = mpsc::channel();
        let freezing = std::thread::spawn(move || snapshot_tx.send(freeze.freeze()).unwrap());
        assert_eq!(
            snapshot_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        commit_tx.send(()).unwrap();
        let lease = creating.join().unwrap();
        let snapshot = snapshot_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        freezing.join().unwrap();
        assert_eq!(snapshot.pending.current, 1);
        assert!(snapshot.pending.ever_created);
        drop(lease);
    }

    #[test]
    fn count_overflow_is_rejected_at_prepare() {
        let (registrations, freeze) = open_inventory();
        registrations.pending().0.lock().pending.current = u64::MAX;

        let error = registrations.pending().prepare().unwrap_err();
        assert_eq!(error.domain(), OwnerDomain::Pending);
        assert_eq!(error.kind(), OwnerCreationErrorKind::CountOverflow);

        let snapshot = freeze.freeze();
        assert_eq!(snapshot.pending.current, u64::MAX);
        assert!(!snapshot.pending.ever_created);
    }

    struct ContextResource(Arc<AtomicUsize>);

    impl VmRootSource for ContextResource {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl crate::RequestResourceState for ContextResource {
        fn terminate(self: Box<Self>, _termination: RequestResourceTermination) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn context_owns_one_shared_resource_table_and_closes_it_before_freeze() {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let first_view = context.resource_table();
        let second_view = context.resource_table();
        let terminal_count = Arc::new(AtomicUsize::new(0));
        let handle = first_view
            .register(Box::new(ContextResource(Arc::clone(&terminal_count))))
            .unwrap();

        assert!(second_view.validate(&handle).is_ok());
        assert_eq!(first_view.live_count(), 1);

        let snapshot = context.freeze();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert_eq!(terminal_count.load(Ordering::Relaxed), 1);
        assert_eq!(
            second_view.release(&handle).unwrap(),
            crate::RequestResourceRelease::AlreadyReleased
        );
    }
}
