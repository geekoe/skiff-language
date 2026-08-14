use std::{
    collections::HashMap,
    fmt,
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

#[cfg(test)]
use std::sync::Weak;

use skiff_runtime_model::{
    error::WirePayload,
    vm_heap::VmHeapError,
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, VmHandle},
};

use crate::owner_inventory::{OwnerCreationError, ResourceOwnerLease, ResourceOwnerRegistration};

/// Why a request resource left its table-owned live state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceTermination {
    VmDrop,
    Exhausted,
    HostError,
    Cancelled,
    Deadline,
    RouterDisconnected,
    WriterFailed,
    ResponseLimitExceeded,
    RequestNotStarted,
    RequestCompleted,
    RequestFailed,
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
pub type RequestByteStreamPullFuture = Pin<
    Box<dyn Future<Output = Result<Option<Vec<u8>>, RequestByteStreamFailure>> + Send + 'static>,
>;

#[derive(Debug)]
pub enum RequestByteStreamFailure {
    Cancelled,
    Ordinary(Box<dyn WirePayload>),
    InvalidProviderContract(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestByteStreamPullStartError {
    PullInProgress,
    Terminated,
    WrongResourceKind,
    Lookup(RequestResourceLookupError),
}

impl fmt::Display for RequestByteStreamPullStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PullInProgress => "request byte stream already has a pull in progress",
            Self::Terminated => "request byte stream is terminated",
            Self::WrongResourceKind => "request resource is not a byte stream",
            Self::Lookup(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for RequestByteStreamPullStartError {}

/// Dependency-neutral, heap-free source owned by one resource-table entry.
///
/// `start_pull` only prepares an owned future and must return without polling
/// or waiting. The future may retain provider operation state but never a VM
/// heap or resource-table lock.
pub trait RequestByteStreamSource: VmRootSource + Send + 'static {
    fn start_pull(&self) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError>;

    fn terminate(self: Box<Self>, termination: RequestResourceTermination);
}

/// Closed event vocabulary owned by the request's server-response stream.
/// Payload bytes and headers remain in the request crate; the central table
/// owns only protocol order, sequence allocation and the single in-flight
/// flush permit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestServerStreamEventKind {
    Start,
    Chunk { payload_bytes: usize },
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RequestServerStreamReservationFacts {
    operation: u64,
    kind: RequestServerStreamEventKind,
    sequence: Option<u64>,
    emitted_bytes_after: usize,
}

/// Opaque exact reservation for one capacity-one writer flush.
///
/// Only the resource table can mint this value. The request driver may copy it
/// into a heap-free completion payload, but cannot change its handle,
/// operation, event kind or allocated sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestServerStreamReservation {
    handle: RequestResourceHandle,
    facts: RequestServerStreamReservationFacts,
}

impl RequestServerStreamReservation {
    pub const fn handle(self) -> RequestResourceHandle {
        self.handle
    }

    pub const fn kind(self) -> RequestServerStreamEventKind {
        self.facts.kind
    }

    pub const fn sequence(self) -> Option<u64> {
        self.facts.sequence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestServerStreamReserveError {
    FlushInProgress,
    InvalidTransition,
    SequenceExhausted,
    ResponseLimitExceeded {
        limit_bytes: usize,
        emitted_bytes: usize,
        chunk_bytes: usize,
    },
    Terminated,
    WrongResourceKind,
    Lookup(RequestResourceLookupError),
}

impl fmt::Display for RequestServerStreamReserveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FlushInProgress => {
                formatter.write_str("server response stream already has a flush in progress")
            }
            Self::InvalidTransition => {
                formatter.write_str("server response stream event is out of order")
            }
            Self::SequenceExhausted => {
                formatter.write_str("server response stream sequence space is exhausted")
            }
            Self::ResponseLimitExceeded {
                limit_bytes,
                emitted_bytes,
                chunk_bytes,
            } => write!(
                formatter,
                "server response stream exceeds {limit_bytes} byte limit after {emitted_bytes} emitted bytes with a {chunk_bytes} byte chunk"
            ),
            Self::Terminated => formatter.write_str("server response stream is terminated"),
            Self::WrongResourceKind => {
                formatter.write_str("request resource is not a server response stream")
            }
            Self::Lookup(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RequestServerStreamReserveError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestServerStreamFlushCompletion {
    Committed,
    AlreadyCommitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestServerStreamFlushError {
    WrongOperation,
    Terminated,
    WrongResourceKind,
    Lookup(RequestResourceLookupError),
}

impl fmt::Display for RequestServerStreamFlushError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongOperation => "server response stream flush reservation does not match",
            Self::Terminated => "server response stream is terminated",
            Self::WrongResourceKind => "request resource is not a server response stream",
            Self::Lookup(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for RequestServerStreamFlushError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestServerStreamPhase {
    AwaitingStart,
    Streaming,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestServerStreamSnapshot {
    pub phase: RequestServerStreamPhase,
    pub next_sequence: u64,
    pub flush_in_progress: bool,
    pub emitted_bytes: usize,
    pub max_response_bytes: usize,
}

pub(crate) trait RequestResourceState: VmRootSource + Send + 'static {
    fn start_byte_stream_pull(
        &self,
    ) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
        Err(RequestByteStreamPullStartError::WrongResourceKind)
    }

    fn reserve_server_stream(
        &mut self,
        _kind: RequestServerStreamEventKind,
    ) -> Result<RequestServerStreamReservationFacts, RequestServerStreamReserveError> {
        Err(RequestServerStreamReserveError::WrongResourceKind)
    }

    fn complete_server_stream_flush(
        &mut self,
        _facts: RequestServerStreamReservationFacts,
    ) -> Result<RequestServerStreamFlushCompletion, RequestServerStreamFlushError> {
        Err(RequestServerStreamFlushError::WrongResourceKind)
    }

    fn server_stream_snapshot(
        &self,
    ) -> Result<RequestServerStreamSnapshot, RequestServerStreamFlushError> {
        Err(RequestServerStreamFlushError::WrongResourceKind)
    }

    fn terminate(self: Box<Self>, termination: RequestResourceTermination);
}

struct RequestByteStreamResource {
    source: Box<dyn RequestByteStreamSource>,
}

impl VmRootSource for RequestByteStreamResource {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.source.visit_roots(visitor)
    }
}

impl RequestResourceState for RequestByteStreamResource {
    fn start_byte_stream_pull(
        &self,
    ) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
        self.source.start_pull()
    }

    fn terminate(self: Box<Self>, termination: RequestResourceTermination) {
        self.source.terminate(termination);
    }
}

struct RequestServerStreamResource {
    phase: RequestServerStreamPhase,
    next_sequence: u64,
    next_operation: u64,
    in_flight: Option<RequestServerStreamReservationFacts>,
    last_committed: Option<RequestServerStreamReservationFacts>,
    emitted_bytes: usize,
    max_response_bytes: usize,
}

impl RequestServerStreamResource {
    fn new(max_response_bytes: usize) -> Self {
        Self {
            phase: RequestServerStreamPhase::AwaitingStart,
            next_sequence: 0,
            next_operation: 1,
            in_flight: None,
            last_committed: None,
            emitted_bytes: 0,
            max_response_bytes,
        }
    }
}

impl VmRootSource for RequestServerStreamResource {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
    }
}

impl RequestResourceState for RequestServerStreamResource {
    fn reserve_server_stream(
        &mut self,
        kind: RequestServerStreamEventKind,
    ) -> Result<RequestServerStreamReservationFacts, RequestServerStreamReserveError> {
        if self.in_flight.is_some() {
            return Err(RequestServerStreamReserveError::FlushInProgress);
        }
        let (sequence, emitted_bytes_after) = match (self.phase, kind) {
            (RequestServerStreamPhase::AwaitingStart, RequestServerStreamEventKind::Start) => {
                (None, self.emitted_bytes)
            }
            (
                RequestServerStreamPhase::Streaming,
                RequestServerStreamEventKind::Chunk { payload_bytes },
            ) => {
                if self.next_sequence == u64::MAX {
                    return Err(RequestServerStreamReserveError::SequenceExhausted);
                }
                let emitted_bytes_after = self.emitted_bytes.checked_add(payload_bytes).ok_or(
                    RequestServerStreamReserveError::ResponseLimitExceeded {
                        limit_bytes: self.max_response_bytes,
                        emitted_bytes: self.emitted_bytes,
                        chunk_bytes: payload_bytes,
                    },
                )?;
                if emitted_bytes_after > self.max_response_bytes {
                    return Err(RequestServerStreamReserveError::ResponseLimitExceeded {
                        limit_bytes: self.max_response_bytes,
                        emitted_bytes: self.emitted_bytes,
                        chunk_bytes: payload_bytes,
                    });
                }
                (Some(self.next_sequence), emitted_bytes_after)
            }
            (RequestServerStreamPhase::Streaming, RequestServerStreamEventKind::End) => {
                (None, self.emitted_bytes)
            }
            (RequestServerStreamPhase::Ended, _) => {
                return Err(RequestServerStreamReserveError::Terminated);
            }
            _ => return Err(RequestServerStreamReserveError::InvalidTransition),
        };
        let operation = self.next_operation;
        self.next_operation = self
            .next_operation
            .checked_add(1)
            .ok_or(RequestServerStreamReserveError::SequenceExhausted)?;
        let facts = RequestServerStreamReservationFacts {
            operation,
            kind,
            sequence,
            emitted_bytes_after,
        };
        self.in_flight = Some(facts);
        Ok(facts)
    }

    fn complete_server_stream_flush(
        &mut self,
        facts: RequestServerStreamReservationFacts,
    ) -> Result<RequestServerStreamFlushCompletion, RequestServerStreamFlushError> {
        if self.last_committed == Some(facts) && self.in_flight.is_none() {
            return Ok(RequestServerStreamFlushCompletion::AlreadyCommitted);
        }
        if self.in_flight != Some(facts) {
            return Err(RequestServerStreamFlushError::WrongOperation);
        }
        match facts.kind {
            RequestServerStreamEventKind::Start => {
                self.phase = RequestServerStreamPhase::Streaming;
            }
            RequestServerStreamEventKind::Chunk { .. } => {
                let expected = facts
                    .sequence
                    .ok_or(RequestServerStreamFlushError::WrongOperation)?;
                if expected != self.next_sequence {
                    return Err(RequestServerStreamFlushError::WrongOperation);
                }
                self.next_sequence = self
                    .next_sequence
                    .checked_add(1)
                    .ok_or(RequestServerStreamFlushError::WrongOperation)?;
                self.emitted_bytes = facts.emitted_bytes_after;
            }
            RequestServerStreamEventKind::End => {
                self.phase = RequestServerStreamPhase::Ended;
            }
        }
        self.in_flight = None;
        self.last_committed = Some(facts);
        Ok(RequestServerStreamFlushCompletion::Committed)
    }

    fn server_stream_snapshot(
        &self,
    ) -> Result<RequestServerStreamSnapshot, RequestServerStreamFlushError> {
        Ok(RequestServerStreamSnapshot {
            phase: self.phase,
            next_sequence: self.next_sequence,
            flush_in_progress: self.in_flight.is_some(),
            emitted_bytes: self.emitted_bytes,
            max_response_bytes: self.max_response_bytes,
        })
    }

    fn terminate(self: Box<Self>, _termination: RequestResourceTermination) {}
}

const RESOURCE_OWNER_SHIFT: u32 = 32;
const RESOURCE_SLOT_SHIFT: u32 = 16;
const RESOURCE_COMPONENT_MASK: u64 = u16::MAX as u64;
const MAX_RESOURCE_SLOT: u32 = u16::MAX as u32;

static NEXT_RESOURCE_OWNER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestResourceOwner(u32);

impl RequestResourceOwner {
    fn mint() -> Option<Self> {
        NEXT_RESOURCE_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                (current <= u64::from(u32::MAX)).then_some(current + 1)
            })
            .ok()
            .and_then(|owner| u32::try_from(owner).ok())
            .map(Self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestResourceSlot(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestResourceGeneration(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RequestResourceKey {
    slot: RequestResourceSlot,
    generation: RequestResourceGeneration,
}

/// Non-owning route to one exact request resource generation.
///
/// Its owner, slot and generation are intentionally opaque. Cloning a route
/// never clones the resource state, inventory lease or table authority.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestResourceHandle(u64);

impl RequestResourceHandle {
    fn pack(owner: RequestResourceOwner, key: RequestResourceKey) -> Self {
        let bits = (u64::from(owner.0) << RESOURCE_OWNER_SHIFT)
            | (u64::from(key.slot.0) << RESOURCE_SLOT_SHIFT)
            | u64::from(key.generation.0);
        debug_assert_ne!(bits, 0);
        Self(bits)
    }

    fn unpack(self) -> Option<(RequestResourceOwner, RequestResourceKey)> {
        let owner = u32::try_from(self.0 >> RESOURCE_OWNER_SHIFT).ok()?;
        let slot = u16::try_from((self.0 >> RESOURCE_SLOT_SHIFT) & RESOURCE_COMPONENT_MASK).ok()?;
        let generation = u16::try_from(self.0 & RESOURCE_COMPONENT_MASK).ok()?;
        if owner == 0 || slot == 0 || generation == 0 {
            return None;
        }
        Some((
            RequestResourceOwner(owner),
            RequestResourceKey {
                slot: RequestResourceSlot(slot),
                generation: RequestResourceGeneration(generation),
            },
        ))
    }

    /// Returns the opaque fixed-width route embedded in a VM `ResourceRef`.
    ///
    /// The numeric value is not authority: the exact request table still
    /// validates its packed owner, slot and generation on every operation.
    pub const fn vm_handle(self) -> VmHandle {
        VmHandle::new(self.0)
    }

    fn from_vm_handle(route: VmHandle) -> Option<Self> {
        let handle = Self(route.get());
        handle.unpack().map(|_| handle)
    }
}

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
    RouteAlreadyClaimed,
    VmRouteAlreadyAdmitted,
    VmMetadataMismatch,
}

impl fmt::Display for RequestResourceLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongOwner => "request resource belongs to a different owner",
            Self::UnknownSlot => "request resource slot is unknown",
            Self::StaleGeneration => "request resource generation is stale",
            Self::RouteAlreadyClaimed => "request resource route was already claimed",
            Self::VmRouteAlreadyAdmitted => {
                "request resource route was already admitted into the VM"
            }
            Self::VmMetadataMismatch => "request resource VM metadata does not match",
        })
    }
}

impl std::error::Error for RequestResourceLookupError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceRegistrationError {
    OwnerCreation(OwnerCreationError),
    OwnerSpaceExhausted,
    TableClosed,
    SlotSpaceExhausted,
    ServerResponseStreamAlreadyRegistered,
}

impl fmt::Display for RequestResourceRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerCreation(error) => error.fmt(formatter),
            Self::OwnerSpaceExhausted => {
                formatter.write_str("request resource owner space is exhausted")
            }
            Self::TableClosed => formatter.write_str("request resource table is closed"),
            Self::SlotSpaceExhausted => {
                formatter.write_str("request resource slot space is exhausted")
            }
            Self::ServerResponseStreamAlreadyRegistered => {
                formatter.write_str("request already owns a server response stream")
            }
        }
    }
}

impl std::error::Error for RequestResourceRegistrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OwnerCreation(error) => Some(error),
            Self::OwnerSpaceExhausted
            | Self::TableClosed
            | Self::SlotSpaceExhausted
            | Self::ServerResponseStreamAlreadyRegistered => None,
        }
    }
}

/// A failed admission returns the still-owned provider state to its caller.
pub(crate) struct RequestResourceRegistrationFailure {
    reason: RequestResourceRegistrationError,
    state: Box<dyn RequestResourceState>,
}

impl RequestResourceRegistrationFailure {
    pub(crate) const fn reason(&self) -> RequestResourceRegistrationError {
        self.reason
    }

    pub(crate) fn into_state(self) -> Box<dyn RequestResourceState> {
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
    state: Option<Box<dyn RequestResourceState>>,
    termination_pin: Option<RequestResourceTerminationPin>,
    route_claimed: bool,
    vm_metadata: Option<(CompactTypeTag, ValueFlags)>,
    terminal: Option<RequestResourceTermination>,
}

/// A private shared pin for the entry's one counted inventory lease.
///
/// Cloning this pin does not mint another inventory owner. It only prevents a
/// concurrent VM release or request-table close from dropping the entry's
/// unique lease before an already-started provider terminator returns.
#[derive(Clone)]
struct RequestResourceTerminationPin {
    _owner_lease: Arc<ResourceOwnerLease>,
}

struct TerminatingRequestResource {
    state: Box<dyn RequestResourceState>,
    _termination_pin: RequestResourceTerminationPin,
}

impl TerminatingRequestResource {
    fn terminate(self, termination: RequestResourceTermination) {
        let Self {
            state,
            _termination_pin,
        } = self;
        state.terminate(termination);
        drop(_termination_pin);
    }
}

impl RequestResourceEntry {
    fn unarmed(state: Box<dyn RequestResourceState>) -> Self {
        Self {
            state: Some(state),
            termination_pin: None,
            route_claimed: false,
            vm_metadata: None,
            terminal: None,
        }
    }

    fn arm(&mut self, owner_lease: ResourceOwnerLease) {
        assert!(
            self.termination_pin
                .replace(RequestResourceTerminationPin {
                    _owner_lease: Arc::new(owner_lease),
                })
                .is_none(),
            "a request resource entry arms exactly once"
        );
    }

    fn finish(
        &mut self,
        termination: RequestResourceTermination,
    ) -> Option<TerminatingRequestResource> {
        let state = self.state.take()?;
        let termination_pin = self
            .termination_pin
            .as_ref()
            .expect("an installed request resource entry is armed")
            .clone();
        self.terminal = Some(termination);
        Some(TerminatingRequestResource {
            state,
            _termination_pin: termination_pin,
        })
    }

    fn terminate_and_release(self, termination: RequestResourceTermination) {
        let Self {
            state,
            termination_pin,
            route_claimed: _,
            vm_metadata: _,
            terminal: _,
        } = self;
        let termination_pin =
            termination_pin.expect("an installed request resource entry is armed");
        if let Some(state) = state {
            TerminatingRequestResource {
                state,
                _termination_pin: termination_pin,
            }
            .terminate(termination);
        } else {
            drop(termination_pin);
        }
    }

    fn start_byte_stream_pull(
        &self,
    ) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
        self.state
            .as_ref()
            .ok_or(RequestByteStreamPullStartError::Terminated)?
            .start_byte_stream_pull()
    }

    fn reserve_server_stream(
        &mut self,
        kind: RequestServerStreamEventKind,
    ) -> Result<RequestServerStreamReservationFacts, RequestServerStreamReserveError> {
        self.state
            .as_mut()
            .ok_or(RequestServerStreamReserveError::Terminated)?
            .reserve_server_stream(kind)
    }

    fn complete_server_stream_flush(
        &mut self,
        facts: RequestServerStreamReservationFacts,
    ) -> Result<RequestServerStreamFlushCompletion, RequestServerStreamFlushError> {
        self.state
            .as_mut()
            .ok_or(RequestServerStreamFlushError::Terminated)?
            .complete_server_stream_flush(facts)
    }

    fn server_stream_snapshot(
        &self,
    ) -> Result<RequestServerStreamSnapshot, RequestServerStreamFlushError> {
        self.state
            .as_ref()
            .ok_or(RequestServerStreamFlushError::Terminated)?
            .server_stream_snapshot()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceFinish {
    Finished,
    AlreadyFinished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestResourceFinishReason {
    Exhausted,
    HostError,
}

impl RequestResourceFinishReason {
    const fn termination(self) -> RequestResourceTermination {
        match self {
            Self::Exhausted => RequestResourceTermination::Exhausted,
            Self::HostError => RequestResourceTermination::HostError,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestResourceTableSnapshot {
    pub live: usize,
    pub terminal: usize,
}

impl RequestResourceTableSnapshot {
    pub const fn total(self) -> usize {
        self.live + self.terminal
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestResourceTablePhase {
    Open,
    Closed,
}

struct RequestResourceTableState {
    phase: RequestResourceTablePhase,
    server_response_stream_registered: bool,
    next_slot: u32,
    free_slots: Vec<RequestResourceSlot>,
    generations: HashMap<RequestResourceSlot, RequestResourceGeneration>,
    closed: HashMap<RequestResourceKey, Option<(CompactTypeTag, ValueFlags)>>,
    entries: HashMap<RequestResourceSlot, (RequestResourceGeneration, RequestResourceEntry)>,
}

impl RequestResourceTableState {
    fn open() -> Self {
        Self {
            phase: RequestResourceTablePhase::Open,
            server_response_stream_registered: false,
            next_slot: 1,
            free_slots: Vec::new(),
            generations: HashMap::new(),
            closed: HashMap::new(),
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

        if self.next_slot == 0 || self.next_slot > MAX_RESOURCE_SLOT {
            return Err(RequestResourceRegistrationError::SlotSpaceExhausted);
        }
        let slot = RequestResourceSlot(
            u16::try_from(self.next_slot).expect("bounded resource slot fits in packed handle"),
        );
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
    owner: Option<RequestResourceOwner>,
    state: Mutex<RequestResourceTableState>,
}

impl RequestResourceTableShared {
    fn lock(&self) -> MutexGuard<'_, RequestResourceTableState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn key_for(
        &self,
        handle: RequestResourceHandle,
    ) -> Result<RequestResourceKey, RequestResourceLookupError> {
        let (owner, key) = handle
            .unpack()
            .ok_or(RequestResourceLookupError::UnknownSlot)?;
        if self.owner != Some(owner) {
            return Err(RequestResourceLookupError::WrongOwner);
        }
        Ok(key)
    }

    fn drain(
        &self,
        termination: RequestResourceTermination,
    ) -> Vec<(RequestResourceEntry, RequestResourceTermination)> {
        let mut state = self.lock();
        state.phase = RequestResourceTablePhase::Closed;
        let removed: Vec<_> = state.entries.drain().collect();
        for (slot, (generation, entry)) in &removed {
            state.closed.insert(
                RequestResourceKey {
                    slot: *slot,
                    generation: *generation,
                },
                entry.vm_metadata,
            );
            if generation.0 < u16::MAX {
                state.free_slots.push(*slot);
            }
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
            entry.terminate_and_release(RequestResourceTermination::OwnerAbandoned);
        }
    }
}

/// The single scheduler-owned resource authority for one request.
///
/// Construction is scheduler-private and requires the resource registration
/// from the same [`RequestExecutionContext`](crate::RequestExecutionContext)
/// owner inventory. Clones share one table capability; packed handles are
/// non-owning values and cannot keep this table or a provider alive.
#[derive(Clone)]
pub struct RequestResourceTable {
    shared: Arc<RequestResourceTableShared>,
    owner_registration: ResourceOwnerRegistration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestResourceRegistrationKind {
    General,
    ServerResponseStream,
}

/// Opaque root-only pin to one request's exact resource table.
///
/// This capability exposes neither registration nor lifecycle transitions. It
/// only lets runnable and pending scheduler owners enumerate the same table's
/// retained provider roots during an ownership handoff.
pub struct RequestResourceRootPin {
    shared: Arc<RequestResourceTableShared>,
    inventory: Arc<crate::owner_inventory::InventoryShared>,
}

impl RequestResourceRootPin {
    pub(crate) fn is_same_table(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }

    pub(crate) fn matches_inventory(
        &self,
        inventory: &Arc<crate::owner_inventory::InventoryShared>,
    ) -> bool {
        Arc::ptr_eq(&self.inventory, inventory)
    }
}

impl RequestResourceTable {
    pub(crate) fn new(owner_registration: ResourceOwnerRegistration) -> Self {
        Self {
            shared: Arc::new(RequestResourceTableShared {
                owner: RequestResourceOwner::mint(),
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
    pub(crate) fn register(
        &self,
        state: Box<dyn RequestResourceState>,
    ) -> Result<RequestResourceHandle, RequestResourceRegistrationFailure> {
        self.register_with_kind(state, RequestResourceRegistrationKind::General)
    }

    fn register_with_kind(
        &self,
        state: Box<dyn RequestResourceState>,
        kind: RequestResourceRegistrationKind,
    ) -> Result<RequestResourceHandle, RequestResourceRegistrationFailure> {
        let Some(table_owner) = self.shared.owner else {
            return Err(RequestResourceRegistrationFailure {
                reason: RequestResourceRegistrationError::OwnerSpaceExhausted,
                state,
            });
        };
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
        if kind == RequestResourceRegistrationKind::ServerResponseStream
            && table.server_response_stream_registered
        {
            drop(table);
            drop(owner);
            return Err(RequestResourceRegistrationFailure {
                reason: RequestResourceRegistrationError::ServerResponseStreamAlreadyRegistered,
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
        if kind == RequestResourceRegistrationKind::ServerResponseStream {
            table.server_response_stream_registered = true;
        }
        let lease = owner.commit();
        table
            .entries
            .get_mut(&key.slot)
            .expect("the freshly inserted resource entry remains present")
            .1
            .arm(lease);
        drop(table);
        Ok(RequestResourceHandle::pack(table_owner, key))
    }

    /// Registers one typed byte-stream source as the table's sole strong
    /// provider owner. Admission failure terminates the uninstalled source
    /// exactly once and returns the closed registration reason.
    pub fn register_byte_stream(
        &self,
        source: Box<dyn RequestByteStreamSource>,
    ) -> Result<RequestResourceHandle, RequestResourceRegistrationError> {
        match self.register(Box::new(RequestByteStreamResource { source })) {
            Ok(handle) => Ok(handle),
            Err(failure) => {
                let reason = failure.reason();
                failure
                    .into_state()
                    .terminate(RequestResourceTermination::HostError);
                Err(reason)
            }
        }
    }

    /// Installs the request's sole server-response stream state. The table,
    /// not the transport writer, owns event order, sequence allocation,
    /// response-byte accounting and the capacity-one in-flight permit.
    pub fn register_server_response_stream(
        &self,
        max_response_bytes: NonZeroUsize,
    ) -> Result<RequestResourceHandle, RequestResourceRegistrationError> {
        match self.register_with_kind(
            Box::new(RequestServerStreamResource::new(max_response_bytes.get())),
            RequestResourceRegistrationKind::ServerResponseStream,
        ) {
            Ok(handle) => Ok(handle),
            Err(failure) => {
                let reason = failure.reason();
                failure
                    .into_state()
                    .terminate(RequestResourceTermination::HostError);
                Err(reason)
            }
        }
    }

    /// Validates that the route denotes this table's currently live exact
    /// generation, without exposing or cloning provider state.
    pub fn validate(
        &self,
        handle: &RequestResourceHandle,
    ) -> Result<(), RequestResourceLookupError> {
        let key = self.shared.key_for(*handle)?;
        self.shared.lock().validate_key(key)
    }

    /// Validates one opaque VM route against this table and returns the exact
    /// packed handle without consulting any side registry.
    pub fn validate_vm_route(
        &self,
        route: VmHandle,
    ) -> Result<RequestResourceHandle, RequestResourceLookupError> {
        let handle = RequestResourceHandle::from_vm_handle(route)
            .ok_or(RequestResourceLookupError::UnknownSlot)?;
        self.validate(&handle)?;
        Ok(handle)
    }

    /// Claims the capability-context carrier for one newly registered route.
    ///
    /// This is a one-shot admission bit stored on the same table entry, not a
    /// numeric side map. Normal VM `StreamNext` routing remains repeatable
    /// after the carrier has been claimed.
    pub fn claim_vm_route(
        &self,
        route: VmHandle,
    ) -> Result<RequestResourceHandle, RequestResourceLookupError> {
        let handle = RequestResourceHandle::from_vm_handle(route)
            .ok_or(RequestResourceLookupError::UnknownSlot)?;
        let key = self.shared.key_for(handle)?;
        let mut table = self.shared.lock();
        table.validate_key(key)?;
        let (_, entry) = table
            .entries
            .get_mut(&key.slot)
            .expect("a validated resource entry remains live");
        if entry.route_claimed {
            return Err(RequestResourceLookupError::RouteAlreadyClaimed);
        }
        entry.route_claimed = true;
        Ok(handle)
    }

    /// Admits one claimed packed route as the sole VM `ResourceRef` owner.
    ///
    /// The type tag and flags are recorded on the existing table entry rather
    /// than in a heap or side registry. Admission is one-shot: constructing a
    /// second semantic VM owner from the same packed bits fails closed even if
    /// its metadata is identical.
    pub fn admit_vm_route(
        &self,
        route: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<RequestResourceHandle, RequestResourceLookupError> {
        let handle = RequestResourceHandle::from_vm_handle(route)
            .ok_or(RequestResourceLookupError::UnknownSlot)?;
        let key = self.shared.key_for(handle)?;
        let mut table = self.shared.lock();
        table.validate_key(key)?;
        let (_, entry) = table
            .entries
            .get_mut(&key.slot)
            .expect("a validated resource entry remains installed");
        if entry.vm_metadata.is_some() {
            return Err(RequestResourceLookupError::VmRouteAlreadyAdmitted);
        }
        entry.vm_metadata = Some((compact_type_tag, flags));
        Ok(handle)
    }

    /// Validates the complete fixed-width VM slot against the same table entry
    /// that owns its provider and inventory lease.
    pub fn validate_vm_route_metadata(
        &self,
        route: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<RequestResourceHandle, RequestResourceLookupError> {
        let handle = RequestResourceHandle::from_vm_handle(route)
            .ok_or(RequestResourceLookupError::UnknownSlot)?;
        let key = self.shared.key_for(handle)?;
        let table = self.shared.lock();
        table.validate_key(key)?;
        let (_, entry) = table
            .entries
            .get(&key.slot)
            .expect("a validated resource entry remains installed");
        if entry.vm_metadata != Some((compact_type_tag, flags)) {
            return Err(RequestResourceLookupError::VmMetadataMismatch);
        }
        Ok(handle)
    }

    /// Starts one typed pull from the exact live stream entry.
    ///
    /// The table lock is held only while selecting the entry and preparing the
    /// owned future; it is released before the caller performs the first poll.
    pub fn start_byte_stream_pull(
        &self,
        handle: &RequestResourceHandle,
    ) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
        let key = self
            .shared
            .key_for(*handle)
            .map_err(RequestByteStreamPullStartError::Lookup)?;
        let table = self.shared.lock();
        table
            .validate_key(key)
            .map_err(RequestByteStreamPullStartError::Lookup)?;
        table
            .entries
            .get(&key.slot)
            .expect("a validated resource entry remains live")
            .1
            .start_byte_stream_pull()
    }

    /// Reserves the single server-response flush permit and, for a chunk,
    /// allocates its exact sequence after response-cap preflight. No bytes are
    /// counted and no protocol state advances until the matching writer ACK
    /// is committed.
    pub fn reserve_server_stream_event(
        &self,
        handle: &RequestResourceHandle,
        kind: RequestServerStreamEventKind,
    ) -> Result<RequestServerStreamReservation, RequestServerStreamReserveError> {
        let key = self
            .shared
            .key_for(*handle)
            .map_err(RequestServerStreamReserveError::Lookup)?;
        let mut table = self.shared.lock();
        table
            .validate_key(key)
            .map_err(RequestServerStreamReserveError::Lookup)?;
        let facts = table
            .entries
            .get_mut(&key.slot)
            .expect("a validated resource entry remains live")
            .1
            .reserve_server_stream(kind)?;
        Ok(RequestServerStreamReservation {
            handle: *handle,
            facts,
        })
    }

    /// Commits the exact event only after the transport reports a real flush
    /// acknowledgement. A duplicate of the immediately committed reservation
    /// is idempotent; any other drift fails closed.
    pub fn complete_server_stream_flush(
        &self,
        reservation: RequestServerStreamReservation,
    ) -> Result<RequestServerStreamFlushCompletion, RequestServerStreamFlushError> {
        let key = self
            .shared
            .key_for(reservation.handle)
            .map_err(RequestServerStreamFlushError::Lookup)?;
        let mut table = self.shared.lock();
        if table.closed.contains_key(&key) {
            return Err(RequestServerStreamFlushError::Terminated);
        }
        table
            .validate_key(key)
            .map_err(RequestServerStreamFlushError::Lookup)?;
        table
            .entries
            .get_mut(&key.slot)
            .expect("a validated resource entry remains live")
            .1
            .complete_server_stream_flush(reservation.facts)
    }

    pub fn server_stream_snapshot(
        &self,
        handle: &RequestResourceHandle,
    ) -> Result<RequestServerStreamSnapshot, RequestServerStreamFlushError> {
        let key = self
            .shared
            .key_for(*handle)
            .map_err(RequestServerStreamFlushError::Lookup)?;
        let table = self.shared.lock();
        if table.closed.contains_key(&key) {
            return Err(RequestServerStreamFlushError::Terminated);
        }
        table
            .validate_key(key)
            .map_err(RequestServerStreamFlushError::Lookup)?;
        table
            .entries
            .get(&key.slot)
            .expect("a validated resource entry remains live")
            .1
            .server_stream_snapshot()
    }

    /// Consumes provider state exactly once while retaining the affine owner
    /// lease and exact terminal entry until the VM later drops its handle.
    pub fn finish(
        &self,
        handle: &RequestResourceHandle,
        reason: RequestResourceFinishReason,
    ) -> Result<RequestResourceFinish, RequestResourceLookupError> {
        let termination = reason.termination();
        let key = self.shared.key_for(*handle)?;
        let state = {
            let mut table = self.shared.lock();
            table.validate_key(key)?;
            let (_, entry) = table
                .entries
                .get_mut(&key.slot)
                .expect("a validated resource entry remains installed");
            entry.finish(termination)
        };
        let Some(terminating) = state else {
            return Ok(RequestResourceFinish::AlreadyFinished);
        };
        terminating.terminate(termination);
        Ok(RequestResourceFinish::Finished)
    }

    /// Removes and tombstones an exact live entry under the table lock, then
    /// terminates it and releases its private inventory lease outside it.
    pub fn release(
        &self,
        handle: &RequestResourceHandle,
    ) -> Result<RequestResourceRelease, RequestResourceLookupError> {
        self.terminate(handle, RequestResourceTermination::VmDrop)
    }

    /// Runs the one table-owned terminator for an exact live entry.
    pub fn terminate(
        &self,
        handle: &RequestResourceHandle,
        termination: RequestResourceTermination,
    ) -> Result<RequestResourceRelease, RequestResourceLookupError> {
        let key = self.shared.key_for(*handle)?;
        let entry = {
            let mut table = self.shared.lock();
            if table.closed.contains_key(&key) {
                return Ok(RequestResourceRelease::AlreadyReleased);
            }
            table.validate_key(key)?;
            let (generation, entry) = table
                .entries
                .remove(&key.slot)
                .expect("a validated resource entry remains live");
            assert_eq!(generation, key.generation);
            table.closed.insert(key, entry.vm_metadata);
            if key.generation.0 < u16::MAX {
                table.free_slots.push(key.slot);
            }
            entry
        };
        entry.terminate_and_release(termination);
        Ok(RequestResourceRelease::Released)
    }

    /// Releases an exact VM `ResourceRef` route without minting or looking up
    /// a second handle authority.
    pub fn release_vm_route(
        &self,
        route: VmHandle,
    ) -> Result<RequestResourceRelease, RequestResourceLookupError> {
        let handle = RequestResourceHandle::from_vm_handle(route)
            .ok_or(RequestResourceLookupError::UnknownSlot)?;
        self.release(&handle)
    }

    /// Atomically validates VM metadata and releases the exact affine route.
    /// A duplicate release observes the same tombstone and is an idempotent
    /// no-op; it can never target a later generation reusing the same slot.
    pub fn release_vm_route_metadata(
        &self,
        route: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<RequestResourceRelease, RequestResourceLookupError> {
        let handle = RequestResourceHandle::from_vm_handle(route)
            .ok_or(RequestResourceLookupError::UnknownSlot)?;
        let key = self.shared.key_for(handle)?;
        let entry = {
            let mut table = self.shared.lock();
            if let Some(metadata) = table.closed.get(&key) {
                return if *metadata == Some((compact_type_tag, flags)) {
                    Ok(RequestResourceRelease::AlreadyReleased)
                } else {
                    Err(RequestResourceLookupError::VmMetadataMismatch)
                };
            }
            table.validate_key(key)?;
            let (_, current) = table
                .entries
                .get(&key.slot)
                .expect("a validated resource entry remains installed");
            if current.vm_metadata != Some((compact_type_tag, flags)) {
                return Err(RequestResourceLookupError::VmMetadataMismatch);
            }
            let (generation, entry) = table
                .entries
                .remove(&key.slot)
                .expect("a validated resource entry remains installed");
            assert_eq!(generation, key.generation);
            table.closed.insert(key, entry.vm_metadata);
            if key.generation.0 < u16::MAX {
                table.free_slots.push(key.slot);
            }
            entry
        };
        entry.terminate_and_release(RequestResourceTermination::VmDrop);
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
            entry.terminate_and_release(termination);
        }
        count
    }

    pub fn live_count(&self) -> usize {
        self.snapshot().live
    }

    /// Mints a root-only pin bound to this exact table authority.
    pub fn root_pin(&self) -> RequestResourceRootPin {
        RequestResourceRootPin {
            shared: Arc::clone(&self.shared),
            inventory: self.owner_registration.root_inventory_identity(),
        }
    }

    pub fn snapshot(&self) -> RequestResourceTableSnapshot {
        let table = self.shared.lock();
        let live = table
            .entries
            .values()
            .filter(|(_, entry)| entry.state.is_some())
            .count();
        let terminal = table
            .entries
            .values()
            .filter(|(_, entry)| entry.terminal.is_some())
            .count();
        debug_assert_eq!(table.entries.len(), live + terminal);
        RequestResourceTableSnapshot { live, terminal }
    }

    #[cfg(test)]
    fn owner_weak_for_test(&self) -> Weak<RequestResourceTableShared> {
        Arc::downgrade(&self.shared)
    }

    #[cfg(test)]
    fn current_owner_count_for_test(&self) -> u64 {
        self.owner_registration.current_for_test()
    }
}

impl VmRootSource for RequestResourceTable {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        let table = self.shared.lock();
        for (_, entry) in table.entries.values() {
            if let Some(state) = &entry.state {
                state.visit_roots(visitor)?;
            }
        }
        Ok(())
    }
}

impl VmRootSource for RequestResourceRootPin {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        let table = self.shared.lock();
        for (_, entry) in table.entries.values() {
            if let Some(state) = &entry.state {
                state.visit_roots(visitor)?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for RequestResourceRootPin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestResourceRootPin")
            .finish_non_exhaustive()
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
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
        task::{Context, Poll, Wake, Waker},
        time::Duration,
    };

    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
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

    struct RecordingByteStream {
        events: Arc<Mutex<Vec<Event>>>,
        bytes: Vec<u8>,
    }

    impl VmRootSource for RecordingByteStream {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RequestByteStreamSource for RecordingByteStream {
        fn start_pull(
            &self,
        ) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
            let bytes = self.bytes.clone();
            Ok(Box::pin(async move { Ok(Some(bytes)) }))
        }

        fn terminate(self: Box<Self>, termination: RequestResourceTermination) {
            self.events
                .lock()
                .unwrap()
                .push(Event::Terminated(termination));
        }
    }

    struct BlockingTerminationResource {
        entered: mpsc::Sender<RequestResourceTermination>,
        unblock: mpsc::Receiver<()>,
        termination_count: Arc<AtomicUsize>,
    }

    impl VmRootSource for BlockingTerminationResource {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RequestResourceState for BlockingTerminationResource {
        fn terminate(self: Box<Self>, termination: RequestResourceTermination) {
            self.termination_count.fetch_add(1, Ordering::SeqCst);
            self.entered.send(termination).unwrap();
            self.unblock.recv().unwrap();
        }
    }

    fn register_blocking_termination(
        table: &RequestResourceTable,
    ) -> (
        RequestResourceHandle,
        mpsc::Receiver<RequestResourceTermination>,
        mpsc::Sender<()>,
        Arc<AtomicUsize>,
    ) {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (unblock_tx, unblock_rx) = mpsc::channel();
        let termination_count = Arc::new(AtomicUsize::new(0));
        let handle = table
            .register(Box::new(BlockingTerminationResource {
                entered: entered_tx,
                unblock: unblock_rx,
                termination_count: Arc::clone(&termination_count),
            }))
            .unwrap();
        (handle, entered_rx, unblock_tx, termination_count)
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
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
        assert_eq!(
            left.validate_vm_route(handle.vm_handle()),
            Err(RequestResourceLookupError::WrongOwner)
        );

        left.close_all(RequestResourceTermination::RequestCompleted);
        right.close_all(RequestResourceTermination::RequestCompleted);
        assert_eq!(left_freeze.freeze().resource.current, 0);
        assert_eq!(right_freeze.freeze().resource.current, 0);
    }

    #[test]
    fn packed_handle_is_copy_embeds_directly_and_claims_once() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<RequestResourceHandle>();

        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = table.register(resource(&events)).unwrap();
        let copied = handle;
        let slot = ValueSlot::resource_ref(
            copied.vm_handle(),
            skiff_runtime_model::vm_value::CompactTypeTag::new(0),
            skiff_runtime_model::vm_value::ValueFlags::new(0),
        );

        assert_eq!(
            table.validate_vm_route(slot.as_resource_ref().unwrap()),
            Ok(handle)
        );
        assert_eq!(table.claim_vm_route(handle.vm_handle()), Ok(handle));
        assert_eq!(
            table.claim_vm_route(handle.vm_handle()),
            Err(RequestResourceLookupError::RouteAlreadyClaimed)
        );
        assert_eq!(
            table.release_vm_route(handle.vm_handle()),
            Ok(RequestResourceRelease::Released)
        );
        assert_eq!(
            table.release_vm_route(handle.vm_handle()),
            Ok(RequestResourceRelease::AlreadyReleased)
        );

        table.close_all(RequestResourceTermination::RequestCompleted);
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn phase_5_resource_vm_admission_is_one_shot_metadata_bound_and_idempotent_on_drop() {
        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = table.register(resource(&events)).unwrap();
        let tag = CompactTypeTag::new(17);
        let flags = ValueFlags::new(3);

        assert_eq!(
            table.admit_vm_route(handle.vm_handle(), tag, flags),
            Ok(handle)
        );
        assert_eq!(
            table.admit_vm_route(handle.vm_handle(), tag, flags),
            Err(RequestResourceLookupError::VmRouteAlreadyAdmitted)
        );
        assert_eq!(
            table.validate_vm_route_metadata(handle.vm_handle(), tag, flags),
            Ok(handle)
        );
        assert_eq!(
            table.validate_vm_route_metadata(handle.vm_handle(), CompactTypeTag::new(18), flags,),
            Err(RequestResourceLookupError::VmMetadataMismatch)
        );
        assert_eq!(
            table.release_vm_route_metadata(handle.vm_handle(), tag, flags),
            Ok(RequestResourceRelease::Released)
        );
        assert_eq!(
            table.release_vm_route_metadata(handle.vm_handle(), tag, flags),
            Ok(RequestResourceRelease::AlreadyReleased)
        );
        assert_eq!(
            table.release_vm_route_metadata(
                handle.vm_handle(),
                CompactTypeTag::new(tag.get() + 1),
                flags,
            ),
            Err(RequestResourceLookupError::VmMetadataMismatch)
        );
        assert_eq!(
            *events.lock().unwrap(),
            [Event::Terminated(RequestResourceTermination::VmDrop)]
        );
        assert_eq!(freeze.freeze().resource.current, 0);
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

        table.close_all(RequestResourceTermination::RequestCompleted);
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
            [Event::Terminated(RequestResourceTermination::VmDrop)]
        );

        table.close_all(RequestResourceTermination::RequestCompleted);
        let snapshot = freeze.freeze();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
    }

    #[test]
    fn natural_end_terminates_once_and_retains_lease_until_vm_drop() {
        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = table.register(resource(&events)).unwrap();

        assert_eq!(
            table
                .finish(&handle, RequestResourceFinishReason::Exhausted)
                .unwrap(),
            RequestResourceFinish::Finished
        );
        assert_eq!(
            table
                .finish(&handle, RequestResourceFinishReason::Exhausted)
                .unwrap(),
            RequestResourceFinish::AlreadyFinished
        );
        assert_eq!(
            table.snapshot(),
            RequestResourceTableSnapshot {
                live: 0,
                terminal: 1,
            }
        );
        assert_eq!(
            *events.lock().unwrap(),
            [Event::Terminated(RequestResourceTermination::Exhausted)]
        );

        assert_eq!(
            table.release(&handle).unwrap(),
            RequestResourceRelease::Released
        );
        assert_eq!(table.snapshot().total(), 0);
        assert_eq!(
            *events.lock().unwrap(),
            [Event::Terminated(RequestResourceTermination::Exhausted)],
            "VM drop releases the retained lease without a second provider terminal"
        );
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn phase_5_resource_concurrent_vm_release_cannot_outpace_provider_termination() {
        let (table, freeze) = table();
        let (handle, entered, unblock, termination_count) = register_blocking_termination(&table);
        let finish_table = table.clone();
        let finishing = std::thread::spawn(move || {
            finish_table.finish(&handle, RequestResourceFinishReason::Exhausted)
        });

        assert_eq!(
            entered.recv_timeout(Duration::from_secs(1)).unwrap(),
            RequestResourceTermination::Exhausted
        );
        assert_eq!(termination_count.load(Ordering::SeqCst), 1);
        assert_eq!(table.current_owner_count_for_test(), 1);
        assert_eq!(table.release(&handle), Ok(RequestResourceRelease::Released));
        assert_eq!(
            table.current_owner_count_for_test(),
            1,
            "the in-flight terminator pins the entry's one lease"
        );

        unblock.send(()).unwrap();
        assert_eq!(
            finishing.join().unwrap(),
            Ok(RequestResourceFinish::Finished)
        );
        assert_eq!(termination_count.load(Ordering::SeqCst), 1);
        assert_eq!(table.current_owner_count_for_test(), 0);
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn phase_5_resource_concurrent_close_cannot_outpace_provider_termination() {
        let (table, freeze) = table();
        let (handle, entered, unblock, termination_count) = register_blocking_termination(&table);
        let finish_table = table.clone();
        let finishing = std::thread::spawn(move || {
            finish_table.finish(&handle, RequestResourceFinishReason::HostError)
        });

        assert_eq!(
            entered.recv_timeout(Duration::from_secs(1)).unwrap(),
            RequestResourceTermination::HostError
        );
        assert_eq!(termination_count.load(Ordering::SeqCst), 1);
        assert_eq!(table.current_owner_count_for_test(), 1);
        assert_eq!(
            table.close_all(RequestResourceTermination::RequestFailed),
            1
        );
        assert_eq!(
            table.release(&handle),
            Ok(RequestResourceRelease::AlreadyReleased)
        );
        assert_eq!(
            table.current_owner_count_for_test(),
            1,
            "request close cannot release the lease under a running terminator"
        );

        unblock.send(()).unwrap();
        assert_eq!(
            finishing.join().unwrap(),
            Ok(RequestResourceFinish::Finished)
        );
        assert_eq!(termination_count.load(Ordering::SeqCst), 1);
        assert_eq!(table.current_owner_count_for_test(), 0);
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn typed_byte_stream_pull_is_heap_free_and_uses_two_stage_terminal() {
        let (table, freeze) = table();
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = table
            .register_byte_stream(Box::new(RecordingByteStream {
                events: Arc::clone(&events),
                bytes: vec![1, 2, 3],
            }))
            .unwrap();
        let mut pull = table.start_byte_stream_pull(&handle).unwrap();
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);

        match pull.as_mut().poll(&mut context) {
            Poll::Ready(Ok(Some(bytes))) => assert_eq!(bytes, [1, 2, 3]),
            other => panic!("typed byte-stream pull was not ready: {other:?}"),
        }
        assert_eq!(
            table
                .finish(&handle, RequestResourceFinishReason::Exhausted)
                .unwrap(),
            RequestResourceFinish::Finished
        );
        assert!(matches!(
            table.start_byte_stream_pull(&handle),
            Err(RequestByteStreamPullStartError::Terminated)
        ));
        assert_eq!(table.snapshot().terminal, 1);
        assert_eq!(
            table.release(&handle).unwrap(),
            RequestResourceRelease::Released
        );
        assert_eq!(
            *events.lock().unwrap(),
            [Event::Terminated(RequestResourceTermination::Exhausted)]
        );
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn phase_5_stream_server_response_capacity_one_commits_only_after_flush_ack() {
        let (table, freeze) = table();
        let handle = table
            .register_server_response_stream(NonZeroUsize::new(5).unwrap())
            .unwrap();

        let start = table
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();
        assert_eq!(start.sequence(), None);
        assert_eq!(
            table.server_stream_snapshot(&handle).unwrap(),
            RequestServerStreamSnapshot {
                phase: RequestServerStreamPhase::AwaitingStart,
                next_sequence: 0,
                flush_in_progress: true,
                emitted_bytes: 0,
                max_response_bytes: 5,
            }
        );
        assert_eq!(
            table.reserve_server_stream_event(
                &handle,
                RequestServerStreamEventKind::Chunk { payload_bytes: 1 },
            ),
            Err(RequestServerStreamReserveError::FlushInProgress)
        );
        assert_eq!(
            table.complete_server_stream_flush(start),
            Ok(RequestServerStreamFlushCompletion::Committed)
        );

        let first = table
            .reserve_server_stream_event(
                &handle,
                RequestServerStreamEventKind::Chunk { payload_bytes: 3 },
            )
            .unwrap();
        assert_eq!(first.sequence(), Some(0));
        assert_eq!(
            table.server_stream_snapshot(&handle).unwrap().emitted_bytes,
            0,
            "unacknowledged bytes are reserved but not committed"
        );
        assert_eq!(
            table.complete_server_stream_flush(first),
            Ok(RequestServerStreamFlushCompletion::Committed)
        );
        assert_eq!(
            table.complete_server_stream_flush(first),
            Ok(RequestServerStreamFlushCompletion::AlreadyCommitted)
        );
        assert_eq!(
            table.server_stream_snapshot(&handle).unwrap(),
            RequestServerStreamSnapshot {
                phase: RequestServerStreamPhase::Streaming,
                next_sequence: 1,
                flush_in_progress: false,
                emitted_bytes: 3,
                max_response_bytes: 5,
            }
        );

        assert_eq!(
            table.reserve_server_stream_event(
                &handle,
                RequestServerStreamEventKind::Chunk { payload_bytes: 3 },
            ),
            Err(RequestServerStreamReserveError::ResponseLimitExceeded {
                limit_bytes: 5,
                emitted_bytes: 3,
                chunk_bytes: 3,
            })
        );
        assert_eq!(
            table.terminate(&handle, RequestResourceTermination::ResponseLimitExceeded,),
            Ok(RequestResourceRelease::Released)
        );
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn phase_5_resource_server_response_stream_registration_is_one_shot_per_request() {
        let (table, freeze) = table();
        let first = table
            .register_server_response_stream(NonZeroUsize::new(8).unwrap())
            .unwrap();

        assert_eq!(
            table.register_server_response_stream(NonZeroUsize::new(16).unwrap()),
            Err(RequestResourceRegistrationError::ServerResponseStreamAlreadyRegistered)
        );
        assert_eq!(table.snapshot().live, 1);
        assert_eq!(table.current_owner_count_for_test(), 1);

        assert_eq!(table.release(&first), Ok(RequestResourceRelease::Released));
        assert_eq!(
            table.register_server_response_stream(NonZeroUsize::new(16).unwrap()),
            Err(RequestResourceRegistrationError::ServerResponseStreamAlreadyRegistered),
            "releasing the sole stream cannot mint a second production owner"
        );
        assert_eq!(table.current_owner_count_for_test(), 0);
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn phase_5_stream_server_response_orders_start_chunks_and_end_exactly() {
        let (table, freeze) = table();
        let handle = table
            .register_server_response_stream(NonZeroUsize::new(usize::MAX).unwrap())
            .unwrap();

        assert_eq!(
            table.reserve_server_stream_event(
                &handle,
                RequestServerStreamEventKind::Chunk { payload_bytes: 0 },
            ),
            Err(RequestServerStreamReserveError::InvalidTransition)
        );
        let start = table
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::Start)
            .unwrap();
        table.complete_server_stream_flush(start).unwrap();
        let chunk = table
            .reserve_server_stream_event(
                &handle,
                RequestServerStreamEventKind::Chunk { payload_bytes: 0 },
            )
            .unwrap();
        assert_eq!(chunk.sequence(), Some(0));
        let drift = RequestServerStreamReservation {
            handle,
            facts: RequestServerStreamReservationFacts {
                operation: chunk.facts.operation + 1,
                ..chunk.facts
            },
        };
        assert_eq!(
            table.complete_server_stream_flush(drift),
            Err(RequestServerStreamFlushError::WrongOperation)
        );
        table.complete_server_stream_flush(chunk).unwrap();
        let end = table
            .reserve_server_stream_event(&handle, RequestServerStreamEventKind::End)
            .unwrap();
        table.complete_server_stream_flush(end).unwrap();
        assert_eq!(
            table.server_stream_snapshot(&handle).unwrap().phase,
            RequestServerStreamPhase::Ended
        );
        assert_eq!(
            table.reserve_server_stream_event(&handle, RequestServerStreamEventKind::End),
            Err(RequestServerStreamReserveError::Terminated)
        );

        table.close_all(RequestResourceTermination::RequestCompleted);
        assert_eq!(freeze.freeze().resource.current, 0);
    }

    #[test]
    fn phase_5_stream_two_exact_handles_route_without_cross_talk() {
        let (table, freeze) = table();
        let left_events = Arc::new(Mutex::new(Vec::new()));
        let right_events = Arc::new(Mutex::new(Vec::new()));
        let left = table
            .register_byte_stream(Box::new(RecordingByteStream {
                events: Arc::clone(&left_events),
                bytes: b"left".to_vec(),
            }))
            .unwrap();
        let right = table
            .register_byte_stream(Box::new(RecordingByteStream {
                events: Arc::clone(&right_events),
                bytes: b"right".to_vec(),
            }))
            .unwrap();
        let tag = CompactTypeTag::new(21);
        let flags = ValueFlags::new(0);
        for handle in [left, right] {
            assert_eq!(table.claim_vm_route(handle.vm_handle()), Ok(handle));
            assert_eq!(
                table.admit_vm_route(handle.vm_handle(), tag, flags),
                Ok(handle)
            );
        }

        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        let mut left_pull = table.start_byte_stream_pull(&left).unwrap();
        let mut right_pull = table.start_byte_stream_pull(&right).unwrap();
        assert!(matches!(
            left_pull.as_mut().poll(&mut context),
            Poll::Ready(Ok(Some(bytes))) if bytes == b"left"
        ));
        assert!(matches!(
            right_pull.as_mut().poll(&mut context),
            Poll::Ready(Ok(Some(bytes))) if bytes == b"right"
        ));

        assert_eq!(
            table.release_vm_route_metadata(left.vm_handle(), tag, flags),
            Ok(RequestResourceRelease::Released)
        );
        assert_eq!(
            table.validate_vm_route_metadata(right.vm_handle(), tag, flags),
            Ok(right),
            "dropping the left route must not observe or terminate the right route"
        );
        assert!(right_events.lock().unwrap().is_empty());

        assert_eq!(
            table.finish(&right, RequestResourceFinishReason::Exhausted),
            Ok(RequestResourceFinish::Finished)
        );
        assert_eq!(
            table.release_vm_route_metadata(right.vm_handle(), tag, flags),
            Ok(RequestResourceRelease::Released)
        );
        assert_eq!(
            *left_events.lock().unwrap(),
            [Event::Terminated(RequestResourceTermination::VmDrop)]
        );
        assert_eq!(
            *right_events.lock().unwrap(),
            [Event::Terminated(RequestResourceTermination::Exhausted)]
        );
        assert_eq!(table.snapshot().total(), 0);
        assert_eq!(freeze.freeze().resource.current, 0);
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
            table.close_all(RequestResourceTermination::RequestCompleted),
            2
        );
        assert_eq!(
            table.close_all(RequestResourceTermination::RequestCompleted),
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
                Event::Terminated(RequestResourceTermination::RequestCompleted),
                Event::TableUnlocked,
            ]
        );
        assert_eq!(
            *second_events.lock().unwrap(),
            [
                Event::Terminated(RequestResourceTermination::RequestCompleted),
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
        table.close_all(RequestResourceTermination::RequestCompleted);

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

        table.close_all(RequestResourceTermination::RequestCompleted);
        assert_eq!(freeze.freeze().resource.current, 0);
    }
}
