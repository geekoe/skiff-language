use std::{
    collections::HashMap,
    fmt,
    num::NonZeroU64,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, Weak,
    },
};

use skiff_runtime_model::vm_heap::VmHeapError;
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};
use skiff_runtime_vm::{PendingOperation, PendingTicket, ResumeOutcome, VmResumeToken};

use crate::{
    owner_inventory::{
        OwnerCreationError, PendingOwnerCreationGuard, PendingOwnerLease, PendingOwnerRegistration,
    },
    resource::RequestResourceRootPin,
    root_escrow::PendingRootSet,
    RootDisposition, RootEscrow,
};

static NEXT_PENDING_TICKET: AtomicU64 = AtomicU64::new(1);

/// Observable state names for diagnostics and focused race tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingCellState {
    Open,
    Waiting,
    Settled,
    Claimed,
}

/// A unique resume token plus the suspended invocation chain, before roots
/// have atomically transferred out of the completion cell.
///
/// Neither the draft nor its fields need to implement `Clone`. Publication
/// consumes the draft, preventing two pending owners for one VM resume site.
/// Until then, the draft enumerates the suspended chain as its root source.
#[must_use = "a pending owner draft must be published or synchronously unwound"]
#[derive(Debug)]
pub struct PendingOwnerDraft<R, S> {
    resume: R,
    suspended: S,
}

impl<R, S> PendingOwnerDraft<R, S> {
    pub fn new(resume: R, suspended: S) -> Self {
        Self { resume, suspended }
    }

    pub fn into_parts(self) -> (R, S) {
        (self.resume, self.suspended)
    }

    fn attach(
        self,
        ticket: PendingTicket,
        roots: PendingRootSet,
        owner_lease: Option<PendingOwnerLease>,
    ) -> PendingOwner<R, S> {
        PendingOwner {
            ticket,
            roots,
            resume: Some(self.resume),
            suspended: self.suspended,
            owner_lease,
        }
    }
}

impl<R, S> VmRootSource for PendingOwnerDraft<R, S>
where
    S: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.suspended.visit_roots(visitor)
    }
}

/// The sole owner of a parked invocation chain and its escrowed VM roots.
#[must_use = "a pending owner must be resumed or terminated"]
#[derive(Debug)]
pub struct PendingOwner<R, S> {
    ticket: PendingTicket,
    // Declared before the suspended chain so fail-closed Drop unregisters and
    // releases escrowed roots while their stable owner storage still exists.
    roots: PendingRootSet,
    resume: Option<R>,
    suspended: S,
    owner_lease: Option<PendingOwnerLease>,
}

impl<R, S> PendingOwner<R, S> {
    pub const fn ticket(&self) -> PendingTicket {
        self.ticket
    }

    pub(crate) const fn resume(&self) -> &R {
        self.resume
            .as_ref()
            .expect("a pending owner retains its resume token until install")
    }
}

impl<R, S> VmRootSource for PendingOwner<R, S>
where
    S: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        // The suspended invocation chain stays enumerable while parked: its
        // active and blocked fibers own live handles that must survive a
        // safepoint GC. The transferred escrow covers roots that already left
        // the chain (adapter arguments, stream items), so the two walks never
        // alias each other.
        self.suspended.visit_roots(visitor)?;
        self.roots.visit_roots(visitor)
    }
}

/// One claimed completion ready to enter the scheduler runnable queue.
#[must_use = "a claimed pending wake must be enqueued and resumed"]
#[derive(Debug)]
pub struct PendingWake<R, S, O> {
    owner: PendingOwner<R, S>,
    settlement: PendingSettlement<O>,
}

impl<R, S, O> PendingWake<R, S, O> {
    pub fn owner(&self) -> &PendingOwner<R, S> {
        &self.owner
    }

    #[cfg(test)]
    fn into_parts(self) -> (PendingOwner<R, S>, PendingSettlement<O>) {
        (self.owner, self.settlement)
    }

    /// Claims a queue-owned wake into one non-cloneable, root-enumerable
    /// synchronous handoff guard.
    pub fn claim(self) -> ClaimedPendingWakeGuard<R, S, O> {
        ClaimedPendingWakeGuard {
            owner: self.owner,
            settlement: Some(self.settlement),
        }
    }
}

impl<R, S, O> VmRootSource for PendingWake<R, S, O>
where
    S: VmRootSource,
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.owner.visit_roots(visitor)?;
        self.settlement.visit_roots(visitor)
    }
}

/// Queue-claimed wake kept as one root source through synchronous outcome
/// mapping. Phase 5 has no concurrent GC safepoint; Phase 6 must register this
/// guard with its process-wide safepoint before permitting mapper callbacks or
/// awaits in this handoff.
#[must_use = "a claimed pending wake must be synchronously mapped and resumed"]
pub struct ClaimedPendingWakeGuard<R, S, O> {
    owner: PendingOwner<R, S>,
    settlement: Option<PendingSettlement<O>>,
}

impl<R, S, O> ClaimedPendingWakeGuard<R, S, O> {
    pub(crate) fn map<M>(
        mut self,
        map: impl FnOnce(&R, O, &dyn VmRootSource) -> M,
    ) -> MappedPendingWakeGuard<R, S, M>
    where
        S: VmRootSource,
        O: VmRootSource,
        M: VmRootSource,
    {
        let settlement = self
            .settlement
            .take()
            .expect("one claimed wake maps its settlement exactly once");
        let source = settlement.source();
        let outcome = map(self.owner.resume(), settlement.into_outcome(), &self.owner);
        MappedPendingWakeGuard {
            owner: self.owner,
            source,
            outcome: Some(outcome),
        }
    }
}

impl<R, S, O> VmRootSource for ClaimedPendingWakeGuard<R, S, O>
where
    S: VmRootSource,
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.owner.visit_roots(visitor)?;
        if let Some(settlement) = &self.settlement {
            settlement.visit_roots(visitor)?;
        }
        Ok(())
    }
}

pub(crate) struct MappedPendingWakeGuard<R, S, O> {
    owner: PendingOwner<R, S>,
    source: SettlementSource,
    outcome: Option<O>,
}

/// Ownership-aware rejection from one mapped pending resume attempt.
///
/// `Rejected` returns the untouched resume inputs so the pending guard can
/// keep owning and enumerating them. `Terminal` is only valid after the
/// callback has committed validation and consumed both inputs into a rootless
/// terminal state.
#[derive(Debug)]
pub(crate) enum PendingResumeFailure<R, O, E> {
    Terminal(E),
    Rejected { error: E, resume: R, outcome: O },
}

/// A failed resume together with the complete mapped pending owner.
///
/// Callers must route or explicitly terminate this carrier. In particular,
/// ordinary error propagation must not implicitly discard the suspended
/// owner, its pending lease or its retained request-resource pin.
#[must_use = "a failed mapped pending owner must be routed or explicitly terminated"]
pub(crate) struct MappedPendingResumeFailure<R, S, O, E> {
    error: E,
    guard: MappedPendingWakeGuard<R, S, O>,
}

impl<R, S, O, E> MappedPendingResumeFailure<R, S, O, E> {
    pub(crate) fn into_parts(self) -> (E, MappedPendingWakeGuard<R, S, O>) {
        (self.error, self.guard)
    }
}

impl<R, S, O, E> fmt::Debug for MappedPendingResumeFailure<R, S, O, E>
where
    E: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MappedPendingResumeFailure")
            .field("error", &self.error)
            .field("guard", &"opaque pending owner")
            .finish()
    }
}

impl<R, S, O, E> VmRootSource for MappedPendingResumeFailure<R, S, O, E>
where
    S: VmRootSource,
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.guard.visit_roots(visitor)
    }
}

impl<R, S, O> MappedPendingWakeGuard<R, S, O> {
    /// Rehouses the suspended owner without exposing decomposed root-bearing
    /// parts. The only production use converts a `SuspendedTrampoline` into a
    /// `FlatTrampoline`; that conversion is infallible and has no callback,
    /// allocation, heap access, await or safepoint.
    pub(crate) fn map_suspended<T>(
        self,
        map: impl FnOnce(S) -> T,
    ) -> MappedPendingWakeGuard<R, T, O>
    where
        S: VmRootSource,
        T: VmRootSource,
        O: VmRootSource,
    {
        let PendingOwner {
            ticket,
            roots,
            resume,
            suspended,
            owner_lease,
        } = self.owner;
        MappedPendingWakeGuard {
            owner: PendingOwner {
                ticket,
                roots,
                resume,
                suspended: map(suspended),
                owner_lease,
            },
            source: self.source,
            outcome: self.outcome,
        }
    }

    /// Terminalizes transferred roots and performs the one synchronous resume
    /// commit while this non-cloneable guard remains the composite root source.
    ///
    /// The `resume` callback must not allocate, touch the heap, await or enter
    /// a safepoint. Phase 6 must register this guard with the process-wide
    /// safepoint before relaxing that rule. On success only, `commit` moves the
    /// resumed owner and retained resource pin into the runnable scheduler. On
    /// rejection, the callback must return the resume token and outcome in a
    /// [`PendingResumeFailure::Rejected`]; this method reinstalls both before
    /// returning the complete guard. A [`PendingResumeFailure::Terminal`] is
    /// only valid after the callback has consumed both into a rootless terminal
    /// state. Either failure keeps the suspended owner, pending lease and
    /// retained resource pin in a [`MappedPendingResumeFailure`] for explicit
    /// caller routing or termination. Transferred roots have already taken
    /// their exact settlement terminal before the callback runs.
    pub(crate) fn resume_and_commit<T, E>(
        mut self,
        resume: impl FnOnce(&mut S, R, O) -> Result<(), PendingResumeFailure<R, O, E>>,
        commit: impl FnOnce(S, Option<RequestResourceRootPin>) -> T,
    ) -> Result<T, MappedPendingResumeFailure<R, S, O, E>>
    where
        S: VmRootSource,
        O: VmRootSource,
        T: VmRootSource,
    {
        self.owner.roots.settle_transferred(self.source);
        let resume_token = self
            .owner
            .resume
            .take()
            .expect("a mapped pending wake installs its resume token once");
        let outcome = self
            .outcome
            .take()
            .expect("a mapped pending wake installs its outcome once");
        match resume(&mut self.owner.suspended, resume_token, outcome) {
            Ok(()) => {}
            Err(PendingResumeFailure::Rejected {
                error,
                resume,
                outcome,
            }) => {
                debug_assert!(self.owner.resume.is_none());
                debug_assert!(self.outcome.is_none());
                self.owner.resume = Some(resume);
                self.outcome = Some(outcome);
                return Err(MappedPendingResumeFailure { error, guard: self });
            }
            Err(PendingResumeFailure::Terminal(error)) => {
                return Err(MappedPendingResumeFailure { error, guard: self });
            }
        }

        let resource_roots = self.owner.roots.take_retained();
        let PendingOwner {
            ticket: _,
            roots,
            resume,
            suspended,
            owner_lease,
        } = self.owner;
        debug_assert!(resume.is_none());
        drop(roots);
        let runnable = commit(suspended, resource_roots);
        drop(owner_lease);
        Ok(runnable)
    }
}

impl<R, S, O> VmRootSource for MappedPendingWakeGuard<R, S, O>
where
    S: VmRootSource,
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.owner.visit_roots(visitor)?;
        if let Some(outcome) = &self.outcome {
            outcome.visit_roots(visitor)?;
        }
        Ok(())
    }
}

/// Participant that won the single terminal settlement race.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettlementSource {
    HostCompletion,
    Cancellation,
    Deadline,
    InternalStop,
}

/// Typed terminal outcome and its winning arbiter source.
#[derive(Debug, PartialEq, Eq)]
pub struct PendingSettlement<O> {
    source: SettlementSource,
    outcome: O,
}

impl<O> PendingSettlement<O> {
    pub const fn source(&self) -> SettlementSource {
        self.source
    }

    pub fn outcome(&self) -> &O {
        &self.outcome
    }

    pub fn into_outcome(self) -> O {
        self.outcome
    }
}

impl<O> VmRootSource for PendingSettlement<O>
where
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.outcome.visit_roots(visitor)
    }
}

/// Runtime-neutral queue used by completion threads.
///
/// `enqueue` must only transfer ownership into a runnable queue. It must not
/// poll a VM/adapter unit inline. Implementations may use Tokio internally,
/// but Tokio senders and futures are intentionally absent from this port.
pub trait PendingWakeQueue<R, S, O>: Send + Sync + 'static {
    fn enqueue(&self, wake: PendingWake<R, S, O>);
}

/// Result of publishing the waiting owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingPublication {
    Waiting,
    PrecompletedEnqueued,
}

/// A publication failure returns the still-owned draft so the current fiber
/// can synchronously unwind without releasing an Actor lease first. The
/// carrier must not be flattened into a reason string or discarded before the
/// caller recovers the draft and explicitly routes that suspended owner.
#[must_use = "a publication failure still owns the unpublished suspended draft"]
#[derive(Debug)]
pub struct PendingPublicationError<R, S> {
    draft: PendingOwnerDraft<R, S>,
    reason: PendingPublicationFailure,
}

impl<R, S> PendingPublicationError<R, S> {
    pub const fn reason(&self) -> PendingPublicationFailure {
        self.reason
    }

    pub fn into_draft(self) -> PendingOwnerDraft<R, S> {
        self.draft
    }
}

impl<R, S> fmt::Display for PendingPublicationError<R, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(formatter)
    }
}

impl<R, S> std::error::Error for PendingPublicationError<R, S>
where
    R: fmt::Debug,
    S: fmt::Debug,
{
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingPublicationFailure {
    UnknownOrAlreadyPublishedTicket,
    CellAlreadyWaiting,
    CellAlreadyClaimed,
}

impl fmt::Display for PendingPublicationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOrAlreadyPublishedTicket => {
                formatter.write_str("pending ticket is unknown or was already published")
            }
            Self::CellAlreadyWaiting => {
                formatter.write_str("pending cell already owns a waiting fiber")
            }
            Self::CellAlreadyClaimed => formatter.write_str("pending cell was already claimed"),
        }
    }
}

/// Whether a completion became the winner or arrived after the terminal
/// arbiter had already selected one.
#[must_use = "a duplicate outcome may own host payload that must be released"]
#[derive(Debug)]
pub enum SettleDisposition<O> {
    StoredBeforePublication,
    Enqueued,
    Duplicate(PendingSettlement<O>),
}

/// Failure to allocate another non-zero pending ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeginPendingError {
    TicketSpaceExhausted,
    TicketCollision,
    OwnerCreation(OwnerCreationError),
    ResourceOwnerMismatch,
}

impl fmt::Display for BeginPendingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TicketSpaceExhausted => formatter.write_str("pending ticket space is exhausted"),
            Self::TicketCollision => formatter.write_str("pending ticket collided in the registry"),
            Self::OwnerCreation(error) => error.fmt(formatter),
            Self::ResourceOwnerMismatch => formatter.write_str(
                "pending registry and retained resource roots belong to different requests",
            ),
        }
    }
}

impl std::error::Error for BeginPendingError {}

enum CellState<R, S, O> {
    Open {
        roots: PendingRootSet,
        owner_lease: Option<PendingOwnerLease>,
    },
    Waiting {
        queue: Arc<dyn PendingWakeQueue<R, S, O>>,
        owner: PendingOwner<R, S>,
    },
    Settled {
        settlement: PendingSettlement<O>,
        roots: PendingRootSet,
        owner_lease: Option<PendingOwnerLease>,
    },
    Claimed,
}

struct PendingCell<R, S, O> {
    ticket: PendingTicket,
    state: Mutex<CellState<R, S, O>>,
}

impl<R, S, O> PendingCell<R, S, O> {
    fn abandon_before_publication(&self) -> bool {
        let mut state = lock_unpoisoned(&self.state);
        let previous = std::mem::replace(&mut *state, CellState::Claimed);
        match previous {
            CellState::Open { roots, owner_lease } => {
                drop(state);
                roots.discard_transferred(RootDisposition::PublicationFailed);
                drop(owner_lease);
                true
            }
            CellState::Settled {
                settlement,
                roots,
                owner_lease,
            } => {
                drop(state);
                roots.discard_transferred(RootDisposition::PublicationFailed);
                drop(settlement);
                drop(owner_lease);
                true
            }
            waiting @ CellState::Waiting { .. } => {
                *state = waiting;
                false
            }
            CellState::Claimed => false,
        }
    }
}

impl<R, S, O> VmRootSource for PendingCell<R, S, O>
where
    S: VmRootSource,
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match &*lock_unpoisoned(&self.state) {
            CellState::Open { roots, .. } => roots.visit_roots(visitor),
            CellState::Settled {
                settlement, roots, ..
            } => {
                roots.visit_roots(visitor)?;
                settlement.visit_roots(visitor)
            }
            CellState::Waiting { owner, .. } => owner.visit_roots(visitor),
            CellState::Claimed => Ok(()),
        }
    }
}

impl<R, S, O> PendingCell<R, S, O>
where
    R: Send + 'static,
    S: Send + 'static,
    O: Send + 'static,
{
    fn new(ticket: PendingTicket, roots: PendingRootSet) -> Self {
        Self {
            ticket,
            state: Mutex::new(CellState::Open {
                roots,
                owner_lease: None,
            }),
        }
    }

    /// Arms the freshly inserted unarmed cell with its owner lease.
    ///
    /// Called exactly once, while the registry table lock and the inventory
    /// guard are still held, immediately after the infallible commit. The cell
    /// is not reachable by any other operation before this completes.
    fn arm(&self, owner_lease: PendingOwnerLease) {
        let mut state = lock_unpoisoned(&self.state);
        let CellState::Open {
            owner_lease: slot, ..
        } = &mut *state
        else {
            unreachable!("a pending cell arms exactly once while still open");
        };
        *slot = Some(owner_lease);
    }

    fn state(&self) -> PendingCellState {
        match &*lock_unpoisoned(&self.state) {
            CellState::Open { .. } => PendingCellState::Open,
            CellState::Waiting { .. } => PendingCellState::Waiting,
            CellState::Settled { .. } => PendingCellState::Settled,
            CellState::Claimed => PendingCellState::Claimed,
        }
    }

    fn publish(
        &self,
        draft: PendingOwnerDraft<R, S>,
        queue: Arc<dyn PendingWakeQueue<R, S, O>>,
    ) -> Result<PendingPublication, PendingPublicationError<R, S>> {
        let mut state = lock_unpoisoned(&self.state);
        let previous = std::mem::replace(&mut *state, CellState::Claimed);
        match previous {
            CellState::Open { roots, owner_lease } => {
                *state = CellState::Waiting {
                    queue,
                    owner: draft.attach(self.ticket, roots, owner_lease),
                };
                Ok(PendingPublication::Waiting)
            }
            CellState::Settled {
                settlement,
                roots,
                owner_lease,
            } => {
                let wake = PendingWake {
                    owner: draft.attach(self.ticket, roots, owner_lease),
                    settlement,
                };
                drop(state);
                queue.enqueue(wake);
                Ok(PendingPublication::PrecompletedEnqueued)
            }
            waiting @ CellState::Waiting { .. } => {
                *state = waiting;
                Err(PendingPublicationError {
                    draft,
                    reason: PendingPublicationFailure::CellAlreadyWaiting,
                })
            }
            CellState::Claimed => Err(PendingPublicationError {
                draft,
                reason: PendingPublicationFailure::CellAlreadyClaimed,
            }),
        }
    }

    fn settle(&self, settlement: PendingSettlement<O>) -> SettleDisposition<O> {
        let mut state = lock_unpoisoned(&self.state);
        let previous = std::mem::replace(&mut *state, CellState::Claimed);
        match previous {
            CellState::Open { roots, owner_lease } => {
                *state = CellState::Settled {
                    settlement,
                    roots,
                    owner_lease,
                };
                SettleDisposition::StoredBeforePublication
            }
            CellState::Waiting { queue, owner } => {
                let wake = PendingWake { owner, settlement };
                drop(state);
                queue.enqueue(wake);
                SettleDisposition::Enqueued
            }
            settled @ CellState::Settled { .. } => {
                *state = settled;
                SettleDisposition::Duplicate(settlement)
            }
            CellState::Claimed => SettleDisposition::Duplicate(settlement),
        }
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

type SharedPendingCell<R, S, O> = Arc<PendingCell<R, S, O>>;
type PendingCellTable<R, S, O> = HashMap<PendingTicket, SharedPendingCell<R, S, O>>;

struct RegistryInner<R, S, O> {
    cells: Mutex<PendingCellTable<R, S, O>>,
}

impl<R, S, O> RegistryInner<R, S, O> {
    fn remove_exact(&self, ticket: PendingTicket, expected: &SharedPendingCell<R, S, O>) {
        let mut cells = lock_unpoisoned(&self.cells);
        if cells
            .get(&ticket)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            cells.remove(&ticket);
        }
    }
}

/// Cloneable authority handed to a host operation after roots are escrowed.
/// Every clone races through the same single-winner terminal arbiter.
pub struct CompletionHandle<R, S, O> {
    cell: SharedPendingCell<R, S, O>,
    registry: Weak<RegistryInner<R, S, O>>,
}

impl<R, S, O> CompletionHandle<R, S, O>
where
    R: Send + 'static,
    S: Send + 'static,
    O: Send + 'static,
{
    pub fn ticket(&self) -> PendingTicket {
        self.cell.ticket
    }

    pub fn state(&self) -> PendingCellState {
        self.cell.state()
    }

    /// Competes a host completion against all other terminal sources.
    pub fn complete(&self, outcome: O) -> SettleDisposition<O> {
        self.settle(SettlementSource::HostCompletion, outcome)
    }

    /// Competes explicit cancellation through the same terminal arbiter.
    pub fn cancel(&self, outcome: O) -> SettleDisposition<O> {
        self.settle(SettlementSource::Cancellation, outcome)
    }

    /// Competes a deadline through the same terminal arbiter.
    pub fn deadline(&self, outcome: O) -> SettleDisposition<O> {
        self.settle(SettlementSource::Deadline, outcome)
    }

    /// Competes an uncatchable internal stop through the same terminal arbiter.
    pub fn internal_stop(&self, outcome: O) -> SettleDisposition<O> {
        self.settle(SettlementSource::InternalStop, outcome)
    }

    fn settle(&self, source: SettlementSource, outcome: O) -> SettleDisposition<O> {
        let disposition = self.cell.settle(PendingSettlement { source, outcome });
        if matches!(&disposition, SettleDisposition::Enqueued) {
            if let Some(registry) = self.registry.upgrade() {
                registry.remove_exact(self.cell.ticket, &self.cell);
            }
        }
        disposition
    }
}

impl<R, S, O> Clone for CompletionHandle<R, S, O> {
    fn clone(&self) -> Self {
        Self {
            cell: Arc::clone(&self.cell),
            registry: self.registry.clone(),
        }
    }
}

impl<R, S, O> fmt::Debug for CompletionHandle<R, S, O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompletionHandle")
            .field("ticket", &self.cell.ticket)
            .finish_non_exhaustive()
    }
}

/// Registry joining the host-side completion handle with the later VM `Park`.
///
/// `begin` installs `Open(rootEscrow)` before returning a handle. The public VM
/// publication path consumes one complete `PendingOperation` and performs
/// either `Open -> Waiting` or `Settled -> Claimed`. A waiting cell stays in
/// this scheduler-owned table (and remains root-enumerable) until one terminal
/// source claims and enqueues it. No registry operation polls user or host
/// code.
///
/// Root enumeration follows the crate-level safepoint contract. In particular,
/// the visitor and escrow backing must not block or re-enter this registry.
pub struct PendingRegistry<R, S, O> {
    inner: Arc<RegistryInner<R, S, O>>,
    owner_registration: PendingOwnerRegistration,
}

/// Pending registry specialized to the VM's non-forgeable resume envelope.
pub type VmPendingRegistry<S, O = ResumeOutcome> = PendingRegistry<VmResumeToken, S, O>;

/// Host completion authority paired with a [`VmPendingRegistry`].
pub type VmCompletionHandle<S, O = ResumeOutcome> = CompletionHandle<VmResumeToken, S, O>;

/// Claimed VM wake ready for a runtime-neutral runnable queue.
pub type VmPendingWake<S, O = ResumeOutcome> = PendingWake<VmResumeToken, S, O>;

/// The unique parked VM owner after a successful publication.
pub type VmPendingOwner<S> = PendingOwner<VmResumeToken, S>;

impl<R, S, O> PendingRegistry<R, S, O> {
    pub fn new(owner_registration: PendingOwnerRegistration) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                cells: Mutex::new(HashMap::new()),
            }),
            owner_registration,
        }
    }
}

impl<R, S, O> PendingRegistry<R, S, O>
where
    R: Send + 'static,
    S: Send + 'static,
    O: Send + 'static,
{
    pub fn begin(&self, roots: RootEscrow) -> Result<CompletionHandle<R, S, O>, BeginPendingError> {
        self.begin_with_roots(PendingRootSet::transferred_only(roots))
    }

    /// Begins one pending cell with transferred roots and an exact retained
    /// request authority in the same Phase 4 root graph.
    fn begin_with_roots(
        &self,
        roots: PendingRootSet,
    ) -> Result<CompletionHandle<R, S, O>, BeginPendingError> {
        let raw = match NEXT_PENDING_TICKET.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| current.checked_add(1),
        ) {
            Ok(raw) => raw,
            Err(_) => {
                roots.discard_transferred(RootDisposition::PublicationFailed);
                return Err(BeginPendingError::TicketSpaceExhausted);
            }
        };
        let ticket = PendingTicket::new(
            NonZeroU64::new(raw).expect("pending ticket counter starts at one and never wraps"),
        );
        self.begin_with_ticket(ticket, roots)
    }

    /// Begins one pending cell with the exact scheduler-minted resource root
    /// pin that must remain visible through Open, Settled, Waiting and the
    /// claimed runnable wake.
    pub fn begin_with_resource_roots(
        &self,
        transferred: RootEscrow,
        resource_roots: RequestResourceRootPin,
    ) -> Result<CompletionHandle<R, S, O>, BeginPendingError> {
        if !self
            .owner_registration
            .matches_resource_root_pin(&resource_roots)
        {
            transferred.discard(RootDisposition::PublicationFailed);
            return Err(BeginPendingError::ResourceOwnerMismatch);
        }
        self.begin_with_roots(PendingRootSet::retaining_resource(
            transferred,
            resource_roots,
        ))
    }

    fn begin_with_ticket(
        &self,
        ticket: PendingTicket,
        roots: PendingRootSet,
    ) -> Result<CompletionHandle<R, S, O>, BeginPendingError> {
        let owner = match self.owner_registration.prepare() {
            Ok(owner) => owner,
            Err(error) => {
                roots.discard_transferred(RootDisposition::PublicationFailed);
                return Err(BeginPendingError::OwnerCreation(error));
            }
        };
        self.install_with_guard(ticket, roots, owner)
    }

    fn install_with_guard(
        &self,
        ticket: PendingTicket,
        roots: PendingRootSet,
        owner: PendingOwnerCreationGuard<'_>,
    ) -> Result<CompletionHandle<R, S, O>, BeginPendingError> {
        // The non-cloneable guard already owns the inventory lock. Container
        // acquisition must remain after it throughout this installation path.
        let mut cells = lock_unpoisoned(&self.inner.cells);
        if cells.contains_key(&ticket) {
            drop(cells);
            drop(owner);
            roots.discard_transferred(RootDisposition::PublicationFailed);
            return Err(BeginPendingError::TicketCollision);
        }
        let cell = Arc::new(PendingCell::new(ticket, roots));
        cells.insert(ticket, Arc::clone(&cell));
        // Commit is infallible: it mints the lease and releases the inventory
        // lock. The cell is armed before the table lock is released, so no
        // other operation can observe an unarmed cell.
        let lease = owner.commit();
        cell.arm(lease);
        drop(cells);
        Ok(CompletionHandle {
            cell,
            registry: Arc::downgrade(&self.inner),
        })
    }

    /// Publishes one sealed pending owner after a typed envelope has
    /// established that `ticket` and `draft.resume` are one logical handoff.
    pub fn publish(
        &self,
        ticket: PendingTicket,
        draft: PendingOwnerDraft<R, S>,
        queue: Arc<dyn PendingWakeQueue<R, S, O>>,
    ) -> Result<PendingPublication, PendingPublicationError<R, S>> {
        let cell = lock_unpoisoned(&self.inner.cells).get(&ticket).cloned();
        let Some(cell) = cell else {
            return Err(PendingPublicationError {
                draft,
                reason: PendingPublicationFailure::UnknownOrAlreadyPublishedTicket,
            });
        };
        let publication = cell.publish(draft, queue)?;
        if matches!(publication, PendingPublication::PrecompletedEnqueued) {
            self.inner.remove_exact(ticket, &cell);
        }
        Ok(publication)
    }

    /// Fail-closed cleanup for a host start that cannot return a valid `Park`.
    pub fn abandon(&self, ticket: PendingTicket) -> bool {
        let cell = lock_unpoisoned(&self.inner.cells).get(&ticket).cloned();
        let Some(cell) = cell else {
            return false;
        };
        if !cell.abandon_before_publication() {
            return false;
        }
        self.inner.remove_exact(ticket, &cell);
        true
    }

    fn abandon_publication_error(
        &self,
        ticket: PendingTicket,
        error: PendingPublicationError<R, S>,
    ) -> PendingPublicationError<R, S> {
        // The error still owns the unpublished draft and its suspended stable
        // storage. Terminalize roots in the completion cell before returning
        // that draft to the caller for drop.
        let _ = self.abandon(ticket);
        error
    }

    pub fn unpublished_count(&self) -> usize {
        let cells: Vec<_> = lock_unpoisoned(&self.inner.cells)
            .values()
            .cloned()
            .collect();
        cells
            .into_iter()
            .filter(|cell| {
                matches!(
                    cell.state(),
                    PendingCellState::Open | PendingCellState::Settled
                )
            })
            .count()
    }

    pub fn live_count(&self) -> usize {
        lock_unpoisoned(&self.inner.cells).len()
    }
}

impl<S, O> PendingRegistry<VmResumeToken, S, O>
where
    S: Send + 'static,
    O: Send + 'static,
{
    /// Publishes the VM's actual-`Pending` envelope without exposing a seam
    /// that can exchange its ticket and unique resume token independently.
    ///
    /// The ticket and token remain one sealed [`PendingOperation`] through
    /// [`PendingOperation::into_parts`]; the generic `publish` method is the
    /// lower-level transition used after that exact typed envelope.
    pub fn publish_operation(
        &self,
        operation: PendingOperation,
        suspended: S,
        queue: Arc<dyn PendingWakeQueue<VmResumeToken, S, O>>,
    ) -> Result<PendingPublication, PendingPublicationError<VmResumeToken, S>> {
        let (ticket, resume) = operation.into_parts();
        self.publish(ticket, PendingOwnerDraft::new(resume, suspended), queue)
    }

    /// Publishes one sealed VM pending operation and, on failure, terminalizes
    /// its completion cell before returning the still-owned suspended draft.
    pub fn publish_operation_or_abandon(
        &self,
        operation: PendingOperation,
        suspended: S,
        queue: Arc<dyn PendingWakeQueue<VmResumeToken, S, O>>,
    ) -> Result<PendingPublication, PendingPublicationError<VmResumeToken, S>> {
        let ticket = operation.ticket();
        self.publish_operation(operation, suspended, queue)
            .map_err(|error| self.abandon_publication_error(ticket, error))
    }
}

impl<R, S, O> VmRootSource for PendingRegistry<R, S, O>
where
    S: VmRootSource,
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        let cells: Vec<_> = lock_unpoisoned(&self.inner.cells)
            .values()
            .cloned()
            .collect();
        for cell in cells {
            cell.visit_roots(visitor)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroU64,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc, Arc, Barrier, Mutex,
        },
        time::Duration,
    };

    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };
    use skiff_runtime_vm::{PendingTicket, ResumeOutcome, VmError};

    use super::{
        lock_unpoisoned, BeginPendingError, PendingCellState, PendingOwnerDraft,
        PendingPublication, PendingRegistry, PendingResumeFailure, PendingRootSet, PendingWake,
        PendingWakeQueue, SettleDisposition, SettlementSource,
    };
    use crate::{
        owner_inventory::{PendingOwnerRegistration, RequestExecutionOwnerInventory},
        BytecodeSchedulerPorts, RequestByteStreamPullFuture, RequestByteStreamSource,
        RequestExecutionContext, RequestResourceFinish, RequestResourceFinishReason,
        RequestResourceRelease, RootDisposition, RootEscrow, RootEscrowBacking,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum RootEvent {
        Restored,
        Dropped(RootDisposition),
    }

    type RootEventLog = Arc<Mutex<Vec<RootEvent>>>;

    struct RecordingRoots(RootEventLog);

    impl VmRootSource for RecordingRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for RecordingRoots {
        fn root_count(&self) -> usize {
            0
        }

        fn restore_roots(self: Box<Self>) {
            self.0.lock().unwrap().push(RootEvent::Restored);
        }

        fn drop_roots(self: Box<Self>, disposition: RootDisposition) {
            self.0.lock().unwrap().push(RootEvent::Dropped(disposition));
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PublicationDropEvent {
        Roots(RootDisposition),
        Suspended,
    }

    struct PublicationOrderingRoots {
        suspended_alive: Arc<AtomicBool>,
        events: Arc<Mutex<Vec<PublicationDropEvent>>>,
    }

    impl VmRootSource for PublicationOrderingRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for PublicationOrderingRoots {
        fn root_count(&self) -> usize {
            1
        }

        fn restore_roots(self: Box<Self>) {
            panic!("failed publication must not restore transferred roots")
        }

        fn drop_roots(self: Box<Self>, disposition: RootDisposition) {
            assert!(
                self.suspended_alive.load(Ordering::SeqCst),
                "transferred roots must terminate while suspended stable storage is alive"
            );
            self.events
                .lock()
                .unwrap()
                .push(PublicationDropEvent::Roots(disposition));
        }
    }

    #[derive(Debug)]
    struct PublicationSuspended {
        alive: Arc<AtomicBool>,
        events: Arc<Mutex<Vec<PublicationDropEvent>>>,
    }

    impl VmRootSource for PublicationSuspended {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl Drop for PublicationSuspended {
        fn drop(&mut self) {
            assert!(self.alive.swap(false, Ordering::SeqCst));
            self.events
                .lock()
                .unwrap()
                .push(PublicationDropEvent::Suspended);
        }
    }

    struct PublicationOrderingQueue;

    impl PendingWakeQueue<u64, PublicationSuspended, NoRoots> for PublicationOrderingQueue {
        fn enqueue(&self, _wake: PendingWake<u64, PublicationSuspended, NoRoots>) {
            panic!("a failed publication cannot enqueue a wake")
        }
    }

    #[derive(Debug, Default)]
    struct RootWalkCounts {
        source_visits: AtomicUsize,
        backing_drops: AtomicUsize,
        queue_enqueues: AtomicUsize,
    }

    struct SafepointRoots {
        root: ValueSlot,
        counts: Arc<RootWalkCounts>,
    }

    impl VmRootSource for SafepointRoots {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            self.counts.source_visits.fetch_add(1, Ordering::Relaxed);
            visitor.visit_root(&self.root)
        }
    }

    impl RootEscrowBacking for SafepointRoots {
        fn root_count(&self) -> usize {
            1
        }

        fn restore_roots(self: Box<Self>) {}

        fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {
            self.counts.backing_drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct NoRoots;

    impl VmRootSource for NoRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TrackedPendingOutcome(Arc<AtomicUsize>);

    impl VmRootSource for TrackedPendingOutcome {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl Drop for TrackedPendingOutcome {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Default)]
    struct TrackedOutcomeQueue(Mutex<Vec<PendingWake<u64, NoRoots, TrackedPendingOutcome>>>);

    impl PendingWakeQueue<u64, NoRoots, TrackedPendingOutcome> for TrackedOutcomeQueue {
        fn enqueue(&self, wake: PendingWake<u64, NoRoots, TrackedPendingOutcome>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    #[derive(Default)]
    struct CountingVisitor(usize);

    impl VmRootVisitor for CountingVisitor {
        fn visit_root(&mut self, _root: &ValueSlot) -> Result<(), VmHeapError> {
            self.0 += 1;
            Ok(())
        }
    }

    struct CountingQueue(Arc<RootWalkCounts>);

    impl PendingWakeQueue<u64, NoRoots, NoRoots> for CountingQueue {
        fn enqueue(&self, _wake: PendingWake<u64, NoRoots, NoRoots>) {
            self.0.queue_enqueues.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct RetainedByteStreamRoot(ValueSlot);

    impl VmRootSource for RetainedByteStreamRoot {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            visitor.visit_root(&self.0)
        }
    }

    impl RequestByteStreamSource for RetainedByteStreamRoot {
        fn start_pull(
            &self,
        ) -> Result<RequestByteStreamPullFuture, crate::RequestByteStreamPullStartError> {
            Ok(Box::pin(std::future::pending()))
        }

        fn terminate(self: Box<Self>, _termination: crate::RequestResourceTermination) {}
    }

    #[derive(Default)]
    struct NoRootWakeQueue(Mutex<Vec<PendingWake<u64, NoRoots, NoRoots>>>);

    impl PendingWakeQueue<u64, NoRoots, NoRoots> for NoRootWakeQueue {
        fn enqueue(&self, wake: PendingWake<u64, NoRoots, NoRoots>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    #[derive(Default)]
    struct RecordingQueue(Mutex<Vec<PendingWake<u64, &'static str, &'static str>>>);

    impl PendingWakeQueue<u64, &'static str, &'static str> for RecordingQueue {
        fn enqueue(&self, wake: PendingWake<u64, &'static str, &'static str>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    #[derive(Default)]
    struct RecordingResumeQueue(Mutex<Vec<PendingWake<u64, &'static str, ResumeOutcome>>>);

    impl PendingWakeQueue<u64, &'static str, ResumeOutcome> for RecordingResumeQueue {
        fn enqueue(&self, wake: PendingWake<u64, &'static str, ResumeOutcome>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    type Registry = PendingRegistry<u64, &'static str, &'static str>;
    type TestCompletion = super::CompletionHandle<u64, &'static str, &'static str>;
    type PendingFixture = (TestCompletion, RootEventLog);

    fn pending_registration() -> PendingOwnerRegistration {
        let (mut registrations, _freeze) = RequestExecutionOwnerInventory::open().into_parts();
        registrations.take_pending().unwrap()
    }

    fn begin(registry: &Registry) -> PendingFixture {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = registry
            .begin(RootEscrow::new(Box::new(RecordingRoots(Arc::clone(
                &events,
            )))))
            .unwrap();
        (handle, events)
    }

    #[test]
    fn cloned_completion_handles_do_not_keep_an_abandoned_owner_registered() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (mut registrations, freeze) = inventory.into_parts();
        let registry = Registry::new(registrations.take_pending().unwrap());
        let (completion, _roots) = begin(&registry);
        let completion_clone = completion.clone();

        assert!(registry.abandon(completion.ticket()));
        assert_eq!(completion_clone.state(), PendingCellState::Claimed);

        let frozen = freeze.freeze();
        assert_eq!(frozen.pending.current, 0);
        assert!(frozen.pending.ever_created);
    }

    #[test]
    fn pending_owner_lease_moves_into_the_claimed_wake() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (mut registrations, freeze) = inventory.into_parts();
        let registry = Registry::new(registrations.take_pending().unwrap());
        let queue = Arc::new(RecordingQueue::default());
        let (completion, _roots) = begin(&registry);

        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(1, "fiber"),
                queue.clone(),
            )
            .unwrap();
        assert!(matches!(
            completion.complete("ready"),
            SettleDisposition::Enqueued
        ));
        assert_eq!(registry.live_count(), 0);

        let frozen = freeze.freeze();
        assert_eq!(frozen.pending.current, 1);
        assert!(frozen.pending.ever_created);
        drop(queue.0.lock().unwrap().pop().unwrap());
    }

    #[test]
    fn occupied_pending_ticket_aborts_without_an_inventory_increment() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (mut registrations, freeze) = inventory.into_parts();
        let registry = Registry::new(registrations.take_pending().unwrap());
        let ticket = PendingTicket::new(NonZeroU64::new(41).unwrap());
        let first_roots = Arc::new(Mutex::new(Vec::new()));
        let rejected_roots = Arc::new(Mutex::new(Vec::new()));
        let completion = registry
            .begin_with_ticket(
                ticket,
                PendingRootSet::transferred_only(RootEscrow::new(Box::new(RecordingRoots(
                    Arc::clone(&first_roots),
                )))),
            )
            .unwrap();

        let collision = registry.begin_with_ticket(
            ticket,
            PendingRootSet::transferred_only(RootEscrow::new(Box::new(RecordingRoots(
                Arc::clone(&rejected_roots),
            )))),
        );

        assert!(matches!(collision, Err(BeginPendingError::TicketCollision)));
        assert_eq!(
            *rejected_roots.lock().unwrap(),
            [RootEvent::Dropped(RootDisposition::PublicationFailed)]
        );
        let frozen = freeze.freeze();
        assert_eq!(frozen.pending.current, 1);
        assert!(frozen.pending.ever_created);
        assert!(registry.abandon(completion.ticket()));
    }

    #[test]
    fn pending_creation_holds_inventory_before_locking_the_registry() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (mut registrations, freeze) = inventory.into_parts();
        let registry = Arc::new(Registry::new(registrations.take_pending().unwrap()));
        let ticket = PendingTicket::new(NonZeroU64::new(42).unwrap());
        let roots = Arc::new(Mutex::new(Vec::new()));
        let registry_lock = lock_unpoisoned(&registry.inner.cells);
        let (prepared_tx, prepared_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let creating_registry = Arc::clone(&registry);
        let creating_roots = Arc::clone(&roots);
        let creating = std::thread::spawn(move || {
            let owner = creating_registry.owner_registration.prepare().unwrap();
            prepared_tx.send(()).unwrap();
            continue_rx.recv().unwrap();
            creating_registry.install_with_guard(
                ticket,
                PendingRootSet::transferred_only(RootEscrow::new(Box::new(RecordingRoots(
                    creating_roots,
                )))),
                owner,
            )
        });
        prepared_rx.recv().unwrap();

        let (snapshot_tx, snapshot_rx) = mpsc::channel();
        let freeze_started = Arc::new(Barrier::new(2));
        let freezing_started = Arc::clone(&freeze_started);
        let freezing = std::thread::spawn(move || {
            freezing_started.wait();
            snapshot_tx.send(freeze.freeze()).unwrap();
        });
        freeze_started.wait();
        assert_eq!(
            snapshot_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        continue_tx.send(()).unwrap();
        drop(registry_lock);
        let completion = creating.join().unwrap().unwrap();
        let frozen = snapshot_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        freezing.join().unwrap();
        assert_eq!(frozen.pending.current, 1);
        assert!(frozen.pending.ever_created);
        assert!(registry.abandon(completion.ticket()));
    }

    #[test]
    fn safepoint_root_walk_neither_enqueues_nor_drops_pending_payloads() {
        let registry = PendingRegistry::<u64, NoRoots, NoRoots>::new(pending_registration());
        let counts = Arc::new(RootWalkCounts::default());
        let completion = registry
            .begin(RootEscrow::new(Box::new(SafepointRoots {
                root: ValueSlot::integer(1),
                counts: Arc::clone(&counts),
            })))
            .unwrap();
        let queue = Arc::new(CountingQueue(Arc::clone(&counts)));
        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(1, NoRoots),
                    queue
                )
                .unwrap(),
            PendingPublication::Waiting
        );

        let mut visitor = CountingVisitor::default();
        registry.visit_roots(&mut visitor).unwrap();

        assert_eq!(visitor.0, 1);
        assert_eq!(counts.source_visits.load(Ordering::Relaxed), 1);
        assert_eq!(counts.queue_enqueues.load(Ordering::Relaxed), 0);
        assert_eq!(counts.backing_drops.load(Ordering::Relaxed), 0);

        assert!(matches!(
            completion.internal_stop(NoRoots),
            SettleDisposition::Enqueued
        ));
        assert_eq!(counts.queue_enqueues.load(Ordering::Relaxed), 1);
        assert_eq!(counts.backing_drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn phase_5_first_poll_wake_before_publication_claims_and_enqueues_once() {
        let registry = PendingRegistry::<u64, NoRoots, NoRoots>::new(pending_registration());
        let queue = Arc::new(NoRootWakeQueue::default());
        let roots = Arc::new(Mutex::new(Vec::new()));
        let completion = registry
            .begin(RootEscrow::new(Box::new(RecordingRoots(Arc::clone(
                &roots,
            )))))
            .unwrap();

        assert!(matches!(
            completion.complete(NoRoots),
            SettleDisposition::StoredBeforePublication
        ));
        assert_eq!(completion.state(), PendingCellState::Settled);
        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(7, NoRoots),
                    queue.clone()
                )
                .unwrap(),
            PendingPublication::PrecompletedEnqueued
        );
        assert_eq!(completion.state(), PendingCellState::Claimed);
        assert_eq!(registry.live_count(), 0);

        let wake = queue.0.lock().unwrap().pop().unwrap();
        let suspended = wake
            .claim()
            .map(|_, outcome, _| outcome)
            .resume_and_commit(
                |suspended, resume, outcome| {
                    assert_eq!(resume, 7);
                    assert_eq!(*suspended, NoRoots);
                    assert_eq!(outcome, NoRoots);
                    Ok::<(), PendingResumeFailure<u64, NoRoots, ()>>(())
                },
                |suspended, resource_roots| {
                    assert!(resource_roots.is_none());
                    suspended
                },
            )
            .unwrap();
        assert_eq!(suspended, NoRoots);
        assert_eq!(*roots.lock().unwrap(), [RootEvent::Restored]);
    }

    #[test]
    fn phase_5_first_poll_queued_host_payload_drops_once_when_terminal_mapper_wins() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (mut registrations, freeze) = inventory.into_parts();
        let registry = PendingRegistry::<u64, NoRoots, TrackedPendingOutcome>::new(
            registrations.take_pending().unwrap(),
        );
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        let queue = Arc::new(TrackedOutcomeQueue::default());
        let wake_queue: Arc<dyn PendingWakeQueue<u64, NoRoots, TrackedPendingOutcome>> =
            queue.clone();
        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(17, NoRoots),
                    wake_queue,
                )
                .unwrap(),
            PendingPublication::Waiting
        );
        let drops = Arc::new(AtomicUsize::new(0));
        assert!(matches!(
            completion.complete(TrackedPendingOutcome(Arc::clone(&drops))),
            SettleDisposition::Enqueued
        ));

        let materializations = AtomicUsize::new(0);
        let resumes = AtomicUsize::new(0);
        let terminal_winner = true;
        let wake = queue.0.lock().unwrap().pop().unwrap();
        let resumed = wake
            .claim()
            .map(|_, late_payload, _| {
                if terminal_winner {
                    drop(late_payload);
                } else {
                    materializations.fetch_add(1, Ordering::SeqCst);
                    drop(late_payload);
                }
                NoRoots
            })
            .resume_and_commit(
                |suspended, resume, NoRoots| {
                    resumes.fetch_add(1, Ordering::SeqCst);
                    assert_eq!((*suspended, resume), (NoRoots, 17));
                    Ok::<(), PendingResumeFailure<u64, NoRoots, ()>>(())
                },
                |suspended, resource_roots| {
                    assert!(resource_roots.is_none());
                    suspended
                },
            )
            .unwrap();

        assert_eq!(resumed, NoRoots);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(materializations.load(Ordering::SeqCst), 0);
        assert_eq!(resumes.load(Ordering::SeqCst), 1);
        assert_eq!(registry.live_count(), 0);
        assert_eq!(freeze.freeze().pending.current, 0);
    }

    #[test]
    fn phase_5_resource_pending_root_pin_covers_every_cell_state_and_hides_terminal_provider() {
        let mut context = RequestExecutionContext::<skiff_runtime_vm::VmFiber>::create(
            BytecodeSchedulerPorts::default(),
        );
        let table = context.resource_table();
        let handle = table
            .register_byte_stream(Box::new(RetainedByteStreamRoot(ValueSlot::integer(73))))
            .unwrap();
        let registry = PendingRegistry::<u64, NoRoots, NoRoots>::new(
            context.take_pending_registration().unwrap(),
        );
        let queue = Arc::new(NoRootWakeQueue::default());

        let precompleted = registry
            .begin_with_resource_roots(RootEscrow::empty(), table.root_pin())
            .unwrap();
        let mut open = CountingVisitor::default();
        registry.visit_roots(&mut open).unwrap();
        assert_eq!(open.0, 1, "Open must enumerate the provider root");

        assert!(matches!(
            precompleted.complete(NoRoots),
            SettleDisposition::StoredBeforePublication
        ));
        let mut settled = CountingVisitor::default();
        registry.visit_roots(&mut settled).unwrap();
        assert_eq!(settled.0, 1, "Settled must enumerate the provider root");
        assert_eq!(
            registry
                .publish(
                    precompleted.ticket(),
                    PendingOwnerDraft::new(1, NoRoots),
                    queue.clone(),
                )
                .unwrap(),
            PendingPublication::PrecompletedEnqueued
        );
        let first_wake = queue.0.lock().unwrap().pop().unwrap();
        let mut first_queued = CountingVisitor::default();
        first_wake.visit_roots(&mut first_queued).unwrap();
        assert_eq!(
            first_queued.0, 1,
            "queued wake must retain the provider root"
        );
        let claimed = first_wake.claim();
        let mut claimed_roots = CountingVisitor::default();
        claimed.visit_roots(&mut claimed_roots).unwrap();
        assert_eq!(
            claimed_roots.0, 1,
            "claimed guard must retain provider roots"
        );
        let mapped = claimed.map(|_, outcome, roots| {
            let mut during_mapper = CountingVisitor::default();
            roots.visit_roots(&mut during_mapper).unwrap();
            assert_eq!(
                during_mapper.0, 1,
                "mapper retains the pending owner root pin"
            );
            outcome
        });
        let mut mapped_roots = CountingVisitor::default();
        mapped.visit_roots(&mut mapped_roots).unwrap();
        assert_eq!(mapped_roots.0, 1, "mapped guard must retain provider roots");
        let first_commit = mapped
            .resume_and_commit(
                |suspended, resume, NoRoots| {
                    assert_eq!((*suspended, resume), (NoRoots, 1));
                    Ok::<(), PendingResumeFailure<u64, NoRoots, ()>>(())
                },
                |_suspended, resource_roots| {
                    drop(resource_roots);
                    NoRoots
                },
            )
            .unwrap();
        assert_eq!(first_commit, NoRoots);

        let waiting = registry
            .begin_with_resource_roots(RootEscrow::empty(), table.root_pin())
            .unwrap();
        assert_eq!(
            registry
                .publish(
                    waiting.ticket(),
                    PendingOwnerDraft::new(2, NoRoots),
                    queue.clone(),
                )
                .unwrap(),
            PendingPublication::Waiting
        );
        let mut waiting_roots = CountingVisitor::default();
        registry.visit_roots(&mut waiting_roots).unwrap();
        assert_eq!(
            waiting_roots.0, 1,
            "Waiting must enumerate the provider root"
        );
        assert!(matches!(
            waiting.complete(NoRoots),
            SettleDisposition::Enqueued
        ));
        let second_wake = queue.0.lock().unwrap().pop().unwrap();
        let mut second_queued = CountingVisitor::default();
        second_wake.visit_roots(&mut second_queued).unwrap();
        assert_eq!(second_queued.0, 1);

        assert_eq!(
            table.finish(&handle, RequestResourceFinishReason::Exhausted),
            Ok(RequestResourceFinish::Finished)
        );
        let mut terminal = CountingVisitor::default();
        second_wake.visit_roots(&mut terminal).unwrap();
        assert_eq!(terminal.0, 0, "terminal provider roots must be hidden");
        assert_eq!(table.release(&handle), Ok(RequestResourceRelease::Released));
        let second_commit = second_wake
            .claim()
            .map(|_, outcome, _| outcome)
            .resume_and_commit(
                |suspended, resume, NoRoots| {
                    assert_eq!((*suspended, resume), (NoRoots, 2));
                    Ok::<(), PendingResumeFailure<u64, NoRoots, ()>>(())
                },
                |_suspended, resource_roots| {
                    drop(resource_roots);
                    NoRoots
                },
            )
            .unwrap();
        assert_eq!(second_commit, NoRoots);
        drop(registry);

        let snapshot = context.into_not_started();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
    }

    #[test]
    fn phase_5_resource_pending_registry_rejects_foreign_inventory_root_pin() {
        let mut left = RequestExecutionContext::<skiff_runtime_vm::VmFiber>::create(
            BytecodeSchedulerPorts::default(),
        );
        let right = RequestExecutionContext::<skiff_runtime_vm::VmFiber>::create(
            BytecodeSchedulerPorts::default(),
        );
        let registry = Registry::new(left.take_pending_registration().unwrap());
        let events = Arc::new(Mutex::new(Vec::new()));

        let error = registry
            .begin_with_resource_roots(
                RootEscrow::new(Box::new(RecordingRoots(Arc::clone(&events)))),
                right.resource_table().root_pin(),
            )
            .unwrap_err();

        assert_eq!(error, BeginPendingError::ResourceOwnerMismatch);
        assert_eq!(
            *events.lock().unwrap(),
            [RootEvent::Dropped(RootDisposition::PublicationFailed)]
        );
        drop(registry);
        assert_eq!(left.into_not_started().pending.current, 0);
        assert_eq!(right.into_not_started().resource.current, 0);
    }

    #[test]
    fn publication_before_completion_waits_then_enqueues_once() {
        let registry = Registry::new(pending_registration());
        let queue = Arc::new(RecordingQueue::default());
        let (completion, _roots) = begin(&registry);

        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(9, "chain"),
                    queue.clone()
                )
                .unwrap(),
            PendingPublication::Waiting
        );
        assert_eq!(completion.state(), PendingCellState::Waiting);
        assert_eq!(registry.live_count(), 1);
        assert!(matches!(
            completion.complete("complete"),
            SettleDisposition::Enqueued
        ));
        assert_eq!(registry.live_count(), 0);
        assert_eq!(queue.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn duplicate_terminal_outcome_is_returned_to_its_caller() {
        let registry = Registry::new(pending_registration());
        let (completion, _roots) = begin(&registry);

        assert!(matches!(
            completion.deadline("deadline"),
            SettleDisposition::StoredBeforePublication
        ));
        let duplicate = completion.complete("late-host-value");
        let SettleDisposition::Duplicate(duplicate) = duplicate else {
            panic!("late host completion must lose the terminal race");
        };
        assert_eq!(duplicate.source(), SettlementSource::HostCompletion);
        assert_eq!(duplicate.into_outcome(), "late-host-value");
    }

    #[test]
    fn concurrent_terminal_race_has_one_winner() {
        let registry = Registry::new(pending_registration());
        let (completion, _roots) = begin(&registry);
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for outcome in ["deadline", "cancel"] {
            let completion = completion.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                if outcome == "deadline" {
                    completion.deadline(outcome)
                } else {
                    completion.cancel(outcome)
                }
            }));
        }
        barrier.wait();

        let dispositions: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            dispositions
                .iter()
                .filter(|item| matches!(item, SettleDisposition::StoredBeforePublication))
                .count(),
            1
        );
        assert_eq!(
            dispositions
                .iter()
                .filter(|item| matches!(item, SettleDisposition::Duplicate(_)))
                .count(),
            1
        );
    }

    #[test]
    fn zero_result_resume_outcome_is_delivered_once_through_the_wake_queue() {
        let registry =
            PendingRegistry::<u64, &'static str, ResumeOutcome>::new(pending_registration());
        let queue = Arc::new(RecordingResumeQueue::default());
        let wake_queue: Arc<dyn PendingWakeQueue<u64, &'static str, ResumeOutcome>> = queue.clone();
        let completion = registry
            .begin(RootEscrow::new(Box::new(RecordingRoots(Arc::new(
                Mutex::new(Vec::new()),
            )))))
            .unwrap();

        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(7, "fiber"),
                    wake_queue
                )
                .unwrap(),
            PendingPublication::Waiting
        );
        assert!(matches!(
            completion.complete(ResumeOutcome::Empty),
            SettleDisposition::Enqueued
        ));
        assert!(matches!(
            completion.complete(ResumeOutcome::Empty),
            SettleDisposition::Duplicate(_)
        ));

        let wake = queue.0.lock().unwrap().pop().unwrap();
        let (owner, settlement) = wake.into_parts();
        assert_eq!(owner.ticket(), completion.ticket());
        assert!(matches!(settlement.into_outcome(), ResumeOutcome::Empty));
        assert_eq!(registry.live_count(), 0);
    }

    #[test]
    fn failure_resume_outcome_is_delivered_once_through_the_wake_queue() {
        let registry =
            PendingRegistry::<u64, &'static str, ResumeOutcome>::new(pending_registration());
        let queue = Arc::new(RecordingResumeQueue::default());
        let wake_queue: Arc<dyn PendingWakeQueue<u64, &'static str, ResumeOutcome>> = queue.clone();
        let completion = registry
            .begin(RootEscrow::new(Box::new(RecordingRoots(Arc::new(
                Mutex::new(Vec::new()),
            )))))
            .unwrap();
        let failure = ResumeOutcome::Failure(VmError::ResumeNotExpected);

        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(8, "fiber"),
                    wake_queue
                )
                .unwrap(),
            PendingPublication::Waiting
        );
        assert!(matches!(
            completion.complete(failure),
            SettleDisposition::Enqueued
        ));

        let wake = queue.0.lock().unwrap().pop().unwrap();
        let (_, settlement) = wake.into_parts();
        assert!(matches!(
            settlement.into_outcome(),
            ResumeOutcome::Failure(VmError::ResumeNotExpected)
        ));
        assert_eq!(registry.live_count(), 0);
    }

    #[test]
    fn abandoned_unpublished_cell_discards_roots_and_becomes_tombstone() {
        let registry = Registry::new(pending_registration());
        let (completion, roots) = begin(&registry);

        assert!(registry.abandon(completion.ticket()));
        assert_eq!(registry.live_count(), 0);
        assert_eq!(completion.state(), PendingCellState::Claimed);
        assert_eq!(
            *roots.lock().unwrap(),
            [RootEvent::Dropped(RootDisposition::PublicationFailed)]
        );
        let duplicate = completion.complete("late");
        assert!(matches!(duplicate, SettleDisposition::Duplicate(_)));
    }

    #[test]
    fn phase_5_resource_publication_failure_discards_roots_before_suspended_draft_drop() {
        let target_inventory = RequestExecutionOwnerInventory::open();
        let (mut target_registrations, target_freeze) = target_inventory.into_parts();
        let target = PendingRegistry::<u64, PublicationSuspended, NoRoots>::new(
            target_registrations.take_pending().unwrap(),
        );
        let source_inventory = RequestExecutionOwnerInventory::open();
        let (mut source_registrations, source_freeze) = source_inventory.into_parts();
        let source = PendingRegistry::<u64, PublicationSuspended, NoRoots>::new(
            source_registrations.take_pending().unwrap(),
        );
        let suspended_alive = Arc::new(AtomicBool::new(true));
        let events = Arc::new(Mutex::new(Vec::new()));
        let completion = target
            .begin(RootEscrow::new(Box::new(PublicationOrderingRoots {
                suspended_alive: Arc::clone(&suspended_alive),
                events: Arc::clone(&events),
            })))
            .unwrap();
        let queue: Arc<dyn PendingWakeQueue<u64, PublicationSuspended, NoRoots>> =
            Arc::new(PublicationOrderingQueue);
        let error = source
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(
                    9,
                    PublicationSuspended {
                        alive: Arc::clone(&suspended_alive),
                        events: Arc::clone(&events),
                    },
                ),
                queue,
            )
            .unwrap_err();

        let error = target.abandon_publication_error(completion.ticket(), error);
        assert_eq!(
            *events.lock().unwrap(),
            [PublicationDropEvent::Roots(
                RootDisposition::PublicationFailed
            )]
        );
        assert!(suspended_alive.load(Ordering::SeqCst));
        drop(error);
        assert_eq!(
            *events.lock().unwrap(),
            [
                PublicationDropEvent::Roots(RootDisposition::PublicationFailed),
                PublicationDropEvent::Suspended,
            ]
        );
        assert!(!suspended_alive.load(Ordering::SeqCst));
        assert_eq!(target.live_count(), 0);
        assert_eq!(source.live_count(), 0);
        assert_eq!(target_freeze.freeze().pending.current, 0);
        assert_eq!(source_freeze.freeze().pending.current, 0);
    }

    #[test]
    fn abandon_cannot_steal_an_already_published_owner() {
        let registry = Registry::new(pending_registration());
        let queue = Arc::new(RecordingQueue::default());
        let (completion, _roots) = begin(&registry);
        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(11, "fiber"),
                    queue.clone()
                )
                .unwrap(),
            PendingPublication::Waiting
        );

        assert!(!registry.abandon(completion.ticket()));
        assert_eq!(registry.live_count(), 1);
        assert!(matches!(
            completion.cancel("cancel"),
            SettleDisposition::Enqueued
        ));
        assert_eq!(registry.live_count(), 0);
        assert_eq!(queue.0.lock().unwrap().len(), 1);
    }

    struct SafepointSuspended {
        root: ValueSlot,
        counts: Arc<RootWalkCounts>,
    }

    impl std::fmt::Debug for SafepointSuspended {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("SafepointSuspended")
                .finish_non_exhaustive()
        }
    }

    impl VmRootSource for SafepointSuspended {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            self.counts.source_visits.fetch_add(1, Ordering::Relaxed);
            visitor.visit_root(&self.root)
        }
    }

    struct SafepointSettlement {
        root: ValueSlot,
        counts: Arc<RootWalkCounts>,
    }

    impl std::fmt::Debug for SafepointSettlement {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("SafepointSettlement")
                .finish_non_exhaustive()
        }
    }

    impl VmRootSource for SafepointSettlement {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            self.counts.source_visits.fetch_add(1, Ordering::Relaxed);
            visitor.visit_root(&self.root)
        }
    }

    struct SafepointQueue(Mutex<Vec<PendingWake<u64, SafepointSuspended, SafepointSettlement>>>);

    impl PendingWakeQueue<u64, SafepointSuspended, SafepointSettlement> for SafepointQueue {
        fn enqueue(&self, wake: PendingWake<u64, SafepointSuspended, SafepointSettlement>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    struct SafepointNoRootQueue(Mutex<Vec<PendingWake<u64, SafepointSuspended, NoRoots>>>);

    impl PendingWakeQueue<u64, SafepointSuspended, NoRoots> for SafepointNoRootQueue {
        fn enqueue(&self, wake: PendingWake<u64, SafepointSuspended, NoRoots>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    #[test]
    fn publication_failure_returns_a_root_enumerable_suspended_draft() {
        let target = PendingRegistry::<u64, SafepointSuspended, SafepointSettlement>::new(
            pending_registration(),
        );
        let source = PendingRegistry::<u64, SafepointSuspended, SafepointSettlement>::new(
            pending_registration(),
        );
        let completion = target.begin(RootEscrow::empty()).unwrap();
        let counts = Arc::new(RootWalkCounts::default());
        let queue = Arc::new(SafepointQueue(Mutex::new(Vec::new())));

        let error = source
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(
                    7,
                    SafepointSuspended {
                        root: ValueSlot::integer(2),
                        counts: Arc::clone(&counts),
                    },
                ),
                queue,
            )
            .unwrap_err();

        assert_eq!(
            error.reason(),
            super::PendingPublicationFailure::UnknownOrAlreadyPublishedTicket
        );
        let draft = error.into_draft();
        let mut visitor = CountingVisitor::default();
        draft.visit_roots(&mut visitor).unwrap();
        assert_eq!(visitor.0, 1);
        assert_eq!(counts.source_visits.load(Ordering::Relaxed), 1);
        let (resume, _suspended) = draft.into_parts();
        assert_eq!(resume, 7);

        assert!(target.abandon(completion.ticket()));
        assert_eq!(target.live_count(), 0);
        assert_eq!(source.live_count(), 0);
    }

    #[test]
    fn rejected_resume_reinstalls_owner_inputs_and_keeps_them_root_enumerable() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (mut registrations, freeze) = inventory.into_parts();
        let registry = PendingRegistry::<u64, SafepointSuspended, SafepointSettlement>::new(
            registrations.take_pending().unwrap(),
        );
        let counts = Arc::new(RootWalkCounts::default());
        let queue = Arc::new(SafepointQueue(Mutex::new(Vec::new())));
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(
                    7,
                    SafepointSuspended {
                        root: ValueSlot::integer(2),
                        counts: Arc::clone(&counts),
                    },
                ),
                queue.clone(),
            )
            .unwrap();
        assert!(matches!(
            completion.complete(SafepointSettlement {
                root: ValueSlot::integer(3),
                counts: Arc::clone(&counts),
            }),
            SettleDisposition::Enqueued
        ));

        let failure = queue
            .0
            .lock()
            .unwrap()
            .pop()
            .unwrap()
            .claim()
            .map(|_, outcome, _| outcome)
            .resume_and_commit(
                |_suspended, resume, outcome| {
                    Err(PendingResumeFailure::Rejected {
                        error: "resume rejected",
                        resume,
                        outcome,
                    })
                },
                |_suspended, _resource_roots| NoRoots,
            )
            .unwrap_err();

        let mut visitor = CountingVisitor::default();
        failure.visit_roots(&mut visitor).unwrap();
        assert_eq!(visitor.0, 2);
        assert_eq!(counts.source_visits.load(Ordering::Relaxed), 2);
        assert_eq!(freeze.freeze().pending.current, 1);

        let (error, guard) = failure.into_parts();
        assert_eq!(error, "resume rejected");
        assert_eq!(guard.owner.resume.as_ref(), Some(&7));
        assert!(guard.outcome.is_some());
        drop(guard);
    }

    #[test]
    fn terminal_resume_failure_retains_suspended_and_request_resource_roots() {
        let mut context = RequestExecutionContext::<skiff_runtime_vm::VmFiber>::create(
            BytecodeSchedulerPorts::default(),
        );
        let table = context.resource_table();
        let resource = table
            .register_byte_stream(Box::new(RetainedByteStreamRoot(ValueSlot::integer(73))))
            .unwrap();
        let registry = PendingRegistry::<u64, SafepointSuspended, NoRoots>::new(
            context.take_pending_registration().unwrap(),
        );
        let counts = Arc::new(RootWalkCounts::default());
        let queue = Arc::new(SafepointNoRootQueue(Mutex::new(Vec::new())));
        let completion = registry
            .begin_with_resource_roots(RootEscrow::empty(), table.root_pin())
            .unwrap();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(
                    11,
                    SafepointSuspended {
                        root: ValueSlot::integer(2),
                        counts: Arc::clone(&counts),
                    },
                ),
                queue.clone(),
            )
            .unwrap();
        assert!(matches!(
            completion.complete(NoRoots),
            SettleDisposition::Enqueued
        ));

        let failure = queue
            .0
            .lock()
            .unwrap()
            .pop()
            .unwrap()
            .claim()
            .map(|_, outcome, _| outcome)
            .resume_and_commit(
                |_suspended, _resume, NoRoots| {
                    Err(PendingResumeFailure::Terminal("terminal resume"))
                },
                |_suspended, _resource_roots| NoRoots,
            )
            .unwrap_err();

        let mut visitor = CountingVisitor::default();
        failure.visit_roots(&mut visitor).unwrap();
        assert_eq!(visitor.0, 2);
        assert_eq!(counts.source_visits.load(Ordering::Relaxed), 1);

        let (error, guard) = failure.into_parts();
        assert_eq!(error, "terminal resume");
        assert!(guard.owner.resume.is_none());
        assert!(guard.outcome.is_none());
        drop(guard);
        drop(registry);
        assert_eq!(
            table.finish(&resource, RequestResourceFinishReason::Exhausted),
            Ok(RequestResourceFinish::Finished)
        );
        assert_eq!(
            table.release(&resource),
            Ok(RequestResourceRelease::Released)
        );
        drop(table);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
    }

    #[test]
    fn pending_lease_releases_only_when_resume_failure_carrier_is_dropped() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (mut registrations, freeze) = inventory.into_parts();
        let identity = registrations.resource().root_inventory_identity();
        let registry =
            PendingRegistry::<u64, NoRoots, NoRoots>::new(registrations.take_pending().unwrap());
        let baseline_strong_count = Arc::strong_count(&identity);
        let queue = Arc::new(NoRootWakeQueue::default());
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(13, NoRoots),
                queue.clone(),
            )
            .unwrap();
        assert!(matches!(
            completion.complete(NoRoots),
            SettleDisposition::Enqueued
        ));
        let failure = queue
            .0
            .lock()
            .unwrap()
            .pop()
            .unwrap()
            .claim()
            .map(|_, outcome, _| outcome)
            .resume_and_commit(
                |_suspended, _resume, NoRoots| {
                    Err(PendingResumeFailure::Terminal("terminal resume"))
                },
                |_suspended, _resource_roots| NoRoots,
            )
            .unwrap_err();

        assert_eq!(
            Arc::strong_count(&identity),
            baseline_strong_count + 1,
            "the failure carrier must still own the pending lease"
        );
        drop(failure);
        assert_eq!(Arc::strong_count(&identity), baseline_strong_count);
        assert_eq!(freeze.freeze().pending.current, 0);
    }

    #[test]
    fn pending_owner_root_walk_combines_suspended_chain_escrow_and_wake_values() {
        let registry = PendingRegistry::<u64, SafepointSuspended, SafepointSettlement>::new(
            pending_registration(),
        );
        let counts = Arc::new(RootWalkCounts::default());
        let completion = registry
            .begin(RootEscrow::new(Box::new(SafepointRoots {
                root: ValueSlot::integer(1),
                counts: Arc::clone(&counts),
            })))
            .unwrap();
        let queue = Arc::new(SafepointQueue(Mutex::new(Vec::new())));
        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(
                        7,
                        SafepointSuspended {
                            root: ValueSlot::integer(2),
                            counts: Arc::clone(&counts),
                        },
                    ),
                    queue.clone(),
                )
                .unwrap(),
            PendingPublication::Waiting
        );

        // A waiting owner enumerates both the suspended invocation chain and
        // the transferred escrow.
        let mut waiting_visitor = CountingVisitor::default();
        registry.visit_roots(&mut waiting_visitor).unwrap();
        assert_eq!(waiting_visitor.0, 2);
        assert_eq!(counts.source_visits.load(Ordering::Relaxed), 2);

        assert!(matches!(
            completion.complete(SafepointSettlement {
                root: ValueSlot::integer(3),
                counts: Arc::clone(&counts),
            }),
            SettleDisposition::Enqueued
        ));
        let wake = queue.0.lock().unwrap().pop().unwrap();

        // A claimed wake enumerates the suspended chain, the escrow and the
        // completion value in one walk.
        let mut wake_visitor = CountingVisitor::default();
        wake.visit_roots(&mut wake_visitor).unwrap();
        assert_eq!(wake_visitor.0, 3);
        assert_eq!(counts.source_visits.load(Ordering::Relaxed), 5);
        assert_eq!(counts.backing_drops.load(Ordering::Relaxed), 0);
    }
}
