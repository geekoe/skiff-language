//! Ownership-safe scheduling primitives for bytecode VM execution.
//!
//! This crate owns the flat trampoline, actual-`Pending` handshake and stream
//! supervision state. It deliberately exposes synchronous ports instead of a
//! concrete async runtime, future, task or host implementation.

#![forbid(unsafe_code)]

mod pending;
mod root_escrow;
mod stream;
mod trampoline;

pub use pending::{
    BeginPendingError, CompletionHandle, PendingCellState, PendingOwner, PendingOwnerDraft,
    PendingPublication, PendingPublicationError, PendingPublicationFailure, PendingRegistry,
    PendingSettlement, PendingWake, PendingWakeQueue, SettleDisposition, SettlementSource,
    VmCompletionHandle, VmPendingOwner, VmPendingRegistry, VmPendingWake,
};
pub use root_escrow::{RootDisposition, RootEscrow, RootEscrowBacking};
pub use skiff_runtime_vm::PendingTicket;
pub use stream::{
    StreamConsumer, StreamEmit, StreamError, StreamEvent, StreamPoll, StreamProducer,
    StreamSupervisor, WakeSignal, STREAM_BUFFER_CAPACITY,
};
pub use trampoline::{
    BlockedUnit, FlatTrampoline, ParentResume, SuspendedTrampoline, TrampolineCompletion,
};
