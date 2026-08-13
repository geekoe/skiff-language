use std::{
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerDomain {
    Pending,
    Resource,
    Child,
}

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

    fn prepare(
        self: &Arc<Self>,
        domain: OwnerDomain,
    ) -> Result<OwnerCreationGuard<'_>, OwnerCreationError> {
        let state = self.lock();
        if state.phase == InventoryPhase::Frozen {
            return Err(OwnerCreationError::InventoryFrozen);
        }
        Ok(OwnerCreationGuard {
            inventory: self,
            domain,
            state,
        })
    }
}

struct OwnerCreationGuard<'a> {
    inventory: &'a Arc<InventoryShared>,
    domain: OwnerDomain,
    state: MutexGuard<'a, InventoryState>,
}

impl OwnerCreationGuard<'_> {
    fn install<T>(
        mut self,
        install: impl FnOnce(OwnerLease) -> T,
    ) -> Result<T, OwnerCreationError> {
        let Some(next) = self.state.domain(self.domain).current.checked_add(1) else {
            // Prepared carrier state is owned by `install`; release the
            // inventory lock before that state is dropped on rejection.
            drop(self);
            drop(install);
            return Err(OwnerCreationError::CountOverflow);
        };
        let domain_state = self.state.domain_mut(self.domain);
        domain_state.current = next;
        domain_state.ever_created = true;
        Ok(install(OwnerLease {
            inventory: Arc::clone(self.inventory),
            domain: self.domain,
        }))
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

#[derive(Clone)]
pub struct PendingOwnerRegistration(Arc<InventoryShared>);

#[derive(Clone)]
pub struct ResourceOwnerRegistration(Arc<InventoryShared>);

#[derive(Clone)]
pub struct ChildOwnerRegistration(Arc<InventoryShared>);

pub struct PendingOwnerLease(OwnerLease);
pub struct ResourceOwnerLease(OwnerLease);
pub struct ChildOwnerLease(OwnerLease);

#[must_use = "a pending creation guard must be installed or explicitly aborted"]
pub struct PendingOwnerCreationGuard<'a>(OwnerCreationGuard<'a>);
#[must_use = "a resource creation guard must be installed or explicitly aborted"]
pub struct ResourceOwnerCreationGuard<'a>(OwnerCreationGuard<'a>);
#[must_use = "a child creation guard must be installed or explicitly aborted"]
pub struct ChildOwnerCreationGuard<'a>(OwnerCreationGuard<'a>);

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
            pub fn prepare(&self) -> Result<$guard<'_>, OwnerCreationError> {
                self.0.prepare(OwnerDomain::$domain).map($guard)
            }
        }

        impl $guard<'_> {
            pub fn install<T>(
                self,
                install: impl FnOnce($lease) -> T,
            ) -> Result<T, OwnerCreationError> {
                self.0.install(|lease| install($lease(lease)))
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
pub struct RequestExecutionOwnerRegistrations {
    pending: PendingOwnerRegistration,
    resource: ResourceOwnerRegistration,
    child: ChildOwnerRegistration,
}

impl RequestExecutionOwnerRegistrations {
    pub fn pending(&self) -> PendingOwnerRegistration {
        self.pending.clone()
    }

    pub fn resource(&self) -> ResourceOwnerRegistration {
        self.resource.clone()
    }

    pub fn child(&self) -> ChildOwnerRegistration {
        self.child.clone()
    }
}

/// One request's actual owner inventory and unique freeze authority.
///
/// This type intentionally has no `Default`, zero snapshot constructor, reset,
/// merge, count or freeze-by-reference operation.
pub struct RequestExecutionOwnerInventory {
    registrations: RequestExecutionOwnerRegistrations,
    freeze: RequestExecutionOwnerInventoryFreezePermit,
}

impl RequestExecutionOwnerInventory {
    pub fn open() -> Self {
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

    pub fn into_parts(
        self,
    ) -> (
        RequestExecutionOwnerRegistrations,
        RequestExecutionOwnerInventoryFreezePermit,
    ) {
        (self.registrations, self.freeze)
    }
}

pub struct RequestExecutionOwnerInventoryFreezePermit {
    shared: Arc<InventoryShared>,
}

impl RequestExecutionOwnerInventoryFreezePermit {
    pub fn freeze(self) -> RequestExecutionOwnerInventorySnapshot {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrozenOwnerDomain {
    current: u64,
    ever_created: bool,
}

impl From<DomainState> for FrozenOwnerDomain {
    fn from(state: DomainState) -> Self {
        Self {
            current: state.current,
            ever_created: state.ever_created,
        }
    }
}

impl FrozenOwnerDomain {
    pub const fn current(self) -> u64 {
        self.current
    }

    pub const fn ever_created(self) -> bool {
        self.ever_created
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RequestExecutionOwnerInventorySnapshot {
    pending: FrozenOwnerDomain,
    resource: FrozenOwnerDomain,
    child: FrozenOwnerDomain,
}

impl RequestExecutionOwnerInventorySnapshot {
    pub const fn pending(&self) -> FrozenOwnerDomain {
        self.pending
    }

    pub const fn resource(&self) -> FrozenOwnerDomain {
        self.resource
    }

    pub const fn child(&self) -> FrozenOwnerDomain {
        self.child
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerCreationError {
    InventoryFrozen,
    CountOverflow,
}

impl fmt::Display for OwnerCreationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InventoryFrozen => "request owner inventory is frozen",
            Self::CountOverflow => "request owner inventory count overflowed",
        })
    }
}

impl std::error::Error for OwnerCreationError {}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    #[test]
    fn live_and_released_owners_freeze_actual_current_and_ever_created_facts() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let pending = registrations
            .pending()
            .prepare()
            .unwrap()
            .install(PendingCarrier)
            .unwrap();
        let resource = registrations
            .resource()
            .prepare()
            .unwrap()
            .install(ResourceCarrier)
            .unwrap();
        drop(resource);

        let snapshot = freeze.freeze();
        assert_eq!(snapshot.pending().current(), 1);
        assert!(snapshot.pending().ever_created());
        assert_eq!(snapshot.resource().current(), 0);
        assert!(snapshot.resource().ever_created());
        assert_eq!(snapshot.child().current(), 0);
        assert!(!snapshot.child().ever_created());
        drop(pending);
        assert_eq!(snapshot.pending().current(), 1);
    }

    #[test]
    fn freeze_first_rejects_creation_without_minting_a_lease() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let snapshot = freeze.freeze();

        assert!(matches!(
            registrations.child().prepare(),
            Err(OwnerCreationError::InventoryFrozen)
        ));
        assert_eq!(snapshot.child().current(), 0);
        assert!(!snapshot.child().ever_created());
    }

    #[test]
    fn aborting_a_prepared_creation_preserves_never_created() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();

        drop(registrations.resource().prepare().unwrap());

        let snapshot = freeze.freeze();
        assert_eq!(snapshot.resource().current(), 0);
        assert!(!snapshot.resource().ever_created());
    }

    #[test]
    fn create_and_freeze_have_one_lock_order() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let create_entered = Arc::clone(&entered);
        let create_release = Arc::clone(&release);
        let creating = std::thread::spawn(move || {
            registrations
                .pending()
                .prepare()
                .unwrap()
                .install(|lease| {
                    create_entered.wait();
                    create_release.wait();
                    PendingCarrier(lease)
                })
                .unwrap()
        });
        entered.wait();
        let freezing = std::thread::spawn(move || freeze.freeze());
        release.wait();
        let carrier = creating.join().unwrap();
        let snapshot = freezing.join().unwrap();
        assert_eq!(snapshot.pending().current(), 1);
        assert!(snapshot.pending().ever_created());
        drop(carrier);
    }

    struct PendingCarrier(PendingOwnerLease);
    struct ResourceCarrier(ResourceOwnerLease);
    struct ChildCarrier(ChildOwnerLease);

    impl From<PendingOwnerLease> for PendingCarrier {
        fn from(lease: PendingOwnerLease) -> Self {
            Self(lease)
        }
    }

    impl From<ResourceOwnerLease> for ResourceCarrier {
        fn from(lease: ResourceOwnerLease) -> Self {
            Self(lease)
        }
    }

    impl From<ChildOwnerLease> for ChildCarrier {
        fn from(lease: ChildOwnerLease) -> Self {
            Self(lease)
        }
    }
}
