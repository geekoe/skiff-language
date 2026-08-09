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

use crate::{RootDisposition, RootEscrow};

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

    fn attach(self, ticket: PendingTicket, roots: RootEscrow) -> PendingOwner<R, S> {
        PendingOwner {
            ticket,
            roots,
            resume: self.resume,
            suspended: self.suspended,
        }
    }
}

/// The sole owner of a parked invocation chain and its escrowed VM roots.
#[must_use = "a pending owner must be resumed or terminated"]
#[derive(Debug)]
pub struct PendingOwner<R, S> {
    ticket: PendingTicket,
    // Declared before the suspended chain so fail-closed Drop unregisters and
    // releases escrowed roots while their stable owner storage still exists.
    roots: RootEscrow,
    resume: R,
    suspended: S,
}

impl<R, S> PendingOwner<R, S> {
    pub const fn ticket(&self) -> PendingTicket {
        self.ticket
    }

    pub fn roots(&self) -> &RootEscrow {
        &self.roots
    }

    pub fn into_parts(self) -> (R, S, RootEscrow) {
        (self.resume, self.suspended, self.roots)
    }
}

impl<R, S> VmRootSource for PendingOwner<R, S> {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
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

    pub fn into_parts(self) -> (PendingOwner<R, S>, PendingSettlement<O>) {
        (self.owner, self.settlement)
    }
}

impl<R, S, O> VmRootSource for PendingWake<R, S, O>
where
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.owner.visit_roots(visitor)?;
        self.settlement.visit_roots(visitor)
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
/// can synchronously unwind without releasing an Actor lease first.
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
}

impl fmt::Display for BeginPendingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TicketSpaceExhausted => formatter.write_str("pending ticket space is exhausted"),
            Self::TicketCollision => formatter.write_str("pending ticket collided in the registry"),
        }
    }
}

impl std::error::Error for BeginPendingError {}

enum CellState<R, S, O> {
    Open(RootEscrow),
    Waiting {
        queue: Arc<dyn PendingWakeQueue<R, S, O>>,
        owner: PendingOwner<R, S>,
    },
    Settled {
        settlement: PendingSettlement<O>,
        roots: RootEscrow,
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
            CellState::Open(roots) => {
                drop(state);
                roots.discard(RootDisposition::PublicationFailed);
                true
            }
            CellState::Settled { settlement, roots } => {
                drop(state);
                roots.discard(RootDisposition::PublicationFailed);
                drop(settlement);
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
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match &*lock_unpoisoned(&self.state) {
            CellState::Open(roots) => roots.visit_roots(visitor),
            CellState::Settled { settlement, roots } => {
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
    fn new(ticket: PendingTicket, roots: RootEscrow) -> Self {
        Self {
            ticket,
            state: Mutex::new(CellState::Open(roots)),
        }
    }

    fn state(&self) -> PendingCellState {
        match &*lock_unpoisoned(&self.state) {
            CellState::Open(_) => PendingCellState::Open,
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
            CellState::Open(roots) => {
                *state = CellState::Waiting {
                    queue,
                    owner: draft.attach(self.ticket, roots),
                };
                Ok(PendingPublication::Waiting)
            }
            CellState::Settled { settlement, roots } => {
                let wake = PendingWake {
                    owner: draft.attach(self.ticket, roots),
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
            CellState::Open(roots) => {
                *state = CellState::Settled { settlement, roots };
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

struct RegistryInner<R, S, O> {
    cells: Mutex<HashMap<PendingTicket, Arc<PendingCell<R, S, O>>>>,
}

impl<R, S, O> RegistryInner<R, S, O> {
    fn remove_exact(&self, ticket: PendingTicket, expected: &Arc<PendingCell<R, S, O>>) {
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
    cell: Arc<PendingCell<R, S, O>>,
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
/// `begin` installs `Open(rootEscrow)` before returning a handle. `publish`
/// performs either `Open -> Waiting` or `Settled -> Claimed`. A waiting cell
/// stays in this scheduler-owned table (and remains root-enumerable) until one
/// terminal source claims and enqueues it. No registry operation polls user or
/// host code.
pub struct PendingRegistry<R, S, O> {
    inner: Arc<RegistryInner<R, S, O>>,
}

/// Pending registry specialized to the VM's non-forgeable resume envelope.
pub type VmPendingRegistry<S> = PendingRegistry<VmResumeToken, S, ResumeOutcome>;

/// Host completion authority paired with a [`VmPendingRegistry`].
pub type VmCompletionHandle<S> = CompletionHandle<VmResumeToken, S, ResumeOutcome>;

/// Claimed VM wake ready for a runtime-neutral runnable queue.
pub type VmPendingWake<S> = PendingWake<VmResumeToken, S, ResumeOutcome>;

/// The unique parked VM owner after a successful publication.
pub type VmPendingOwner<S> = PendingOwner<VmResumeToken, S>;

impl<R, S, O> Default for PendingRegistry<R, S, O> {
    fn default() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                cells: Mutex::new(HashMap::new()),
            }),
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
        let raw = match NEXT_PENDING_TICKET.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| current.checked_add(1),
        ) {
            Ok(raw) => raw,
            Err(_) => {
                roots.discard(RootDisposition::PublicationFailed);
                return Err(BeginPendingError::TicketSpaceExhausted);
            }
        };
        let ticket = PendingTicket::new(
            NonZeroU64::new(raw).expect("pending ticket counter starts at one and never wraps"),
        );
        let cell = Arc::new(PendingCell::new(ticket, roots));
        let inserted = {
            let mut cells = lock_unpoisoned(&self.inner.cells);
            match cells.entry(ticket) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(Arc::clone(&cell));
                    true
                }
                std::collections::hash_map::Entry::Occupied(_) => false,
            }
        };
        if !inserted {
            let abandoned = cell.abandon_before_publication();
            debug_assert!(abandoned, "a new cell is open before publication");
            return Err(BeginPendingError::TicketCollision);
        }
        Ok(CompletionHandle {
            cell,
            registry: Arc::downgrade(&self.inner),
        })
    }

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

impl<S> PendingRegistry<VmResumeToken, S, ResumeOutcome>
where
    S: Send + 'static,
{
    /// Publishes the VM's actual-`Pending` envelope without exposing a seam
    /// that can exchange its ticket and unique resume token independently.
    pub fn publish_operation(
        &self,
        operation: PendingOperation,
        suspended: S,
        queue: Arc<dyn PendingWakeQueue<VmResumeToken, S, ResumeOutcome>>,
    ) -> Result<PendingPublication, PendingPublicationError<VmResumeToken, S>> {
        let (ticket, resume) = operation.into_parts();
        self.publish(ticket, PendingOwnerDraft::new(resume, suspended), queue)
    }
}

impl<R, S, O> VmRootSource for PendingRegistry<R, S, O>
where
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
    use std::sync::{Arc, Barrier, Mutex};

    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::{VmRootSource, VmRootVisitor},
    };

    use super::{
        PendingCellState, PendingOwnerDraft, PendingPublication, PendingRegistry, PendingWake,
        PendingWakeQueue, SettleDisposition, SettlementSource,
    };
    use crate::{RootDisposition, RootEscrow, RootEscrowBacking};

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum RootEvent {
        Restored,
        Dropped(RootDisposition),
    }

    struct RecordingRoots(Arc<Mutex<Vec<RootEvent>>>);

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

    #[derive(Default)]
    struct RecordingQueue(Mutex<Vec<PendingWake<u64, &'static str, &'static str>>>);

    impl PendingWakeQueue<u64, &'static str, &'static str> for RecordingQueue {
        fn enqueue(&self, wake: PendingWake<u64, &'static str, &'static str>) {
            self.0.lock().unwrap().push(wake);
        }
    }

    type Registry = PendingRegistry<u64, &'static str, &'static str>;

    fn begin(
        registry: &Registry,
    ) -> (
        super::CompletionHandle<u64, &'static str, &'static str>,
        Arc<Mutex<Vec<RootEvent>>>,
    ) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = registry
            .begin(RootEscrow::new(Box::new(RecordingRoots(Arc::clone(
                &events,
            )))))
            .unwrap();
        (handle, events)
    }

    #[test]
    fn completion_before_publication_claims_and_enqueues_once() {
        let registry = Registry::default();
        let queue = Arc::new(RecordingQueue::default());
        let (completion, roots) = begin(&registry);

        assert!(matches!(
            completion.complete("ready"),
            SettleDisposition::StoredBeforePublication
        ));
        assert_eq!(completion.state(), PendingCellState::Settled);
        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(7, "fiber"),
                    queue.clone()
                )
                .unwrap(),
            PendingPublication::PrecompletedEnqueued
        );
        assert_eq!(completion.state(), PendingCellState::Claimed);
        assert_eq!(registry.live_count(), 0);

        let wake = queue.0.lock().unwrap().pop().unwrap();
        let (owner, settlement) = wake.into_parts();
        assert_eq!(settlement.source(), SettlementSource::HostCompletion);
        assert_eq!(settlement.into_outcome(), "ready");
        let (resume, suspended, escrow) = owner.into_parts();
        assert_eq!((resume, suspended), (7, "fiber"));
        escrow.restore();
        assert_eq!(*roots.lock().unwrap(), [RootEvent::Restored]);
    }

    #[test]
    fn publication_before_completion_waits_then_enqueues_once() {
        let registry = Registry::default();
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
        let registry = Registry::default();
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
        let registry = Registry::default();
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
    fn abandoned_unpublished_cell_discards_roots_and_becomes_tombstone() {
        let registry = Registry::default();
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
    fn abandon_cannot_steal_an_already_published_owner() {
        let registry = Registry::default();
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
}
