//! Ownership-safe scheduling primitives for bytecode VM execution.
//!
//! This crate owns the flat trampoline, actual-`Pending` handshake and stream
//! supervision state. It deliberately exposes synchronous ports instead of a
//! concrete async runtime, future, task or host implementation.
//!
//! # Root-walk safepoint contract
//!
//! Pending-cell and stream-supervisor root sources keep their local state mutex
//! locked while visiting affine payloads; releasing it would let another owner
//! move or drop those roots during the walk. These implementations do not establish the
//! process-wide safepoint themselves. The scheduler must first quiesce root
//! ownership transfers among fibers, pending cells, streams and runnable
//! queues. Every [`VmRootVisitor`](skiff_runtime_model::vm_root::VmRootVisitor)
//! and nested [`VmRootSource`](skiff_runtime_model::vm_root::VmRootSource)
//! used during that safepoint is scheduler-TCB code: it must only enumerate
//! roots, must not block, and must not poll, wake, drop payloads or re-enter a
//! scheduler pending/stream operation.

#![forbid(unsafe_code)]

mod bytecode;
mod owner_inventory;
mod pending;
mod root_escrow;
mod stream;
mod stream_driver;
mod trampoline;

pub use bytecode::{
    BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildStart, BytecodeControl,
    BytecodeHandoff, BytecodeScheduler, BytecodeSchedulerError, BytecodeSchedulerOutcome,
    BytecodeSchedulerPorts, BytecodeStreamHandoff, BytecodeStreamSupervisor, BytecodeUnit,
    BytecodeUnitControl,
};
pub use owner_inventory::{
    OwnerCreationError, OwnerCreationErrorKind, OwnerDomain, PendingOwnerRegistration,
    RequestExecutionContext,
};
pub use pending::{
    BeginPendingError, CompletionHandle, PendingCellState, PendingOwner, PendingOwnerDraft,
    PendingPublication, PendingPublicationError, PendingPublicationFailure, PendingRegistry,
    PendingSettlement, PendingWake, PendingWakeQueue, SettleDisposition, SettlementSource,
    VmCompletionHandle, VmPendingOwner, VmPendingRegistry, VmPendingWake,
};
pub use root_escrow::{RootDisposition, RootEscrow, RootEscrowBacking};
pub use skiff_runtime_model::bytecode_execution_observation::{
    FrozenOwnerDomain, RequestExecutionOwnerInventorySnapshot,
};
pub use skiff_runtime_vm::PendingTicket;
pub use stream::{
    StreamConsumer, StreamEmit, StreamError, StreamEvent, StreamPoll, StreamProducer,
    StreamSupervisor, WakeSignal, STREAM_BUFFER_CAPACITY,
};
pub use stream_driver::{VmStreamConsumerExecutor, VmStreamSupervisor, VmStreamTerminal};
pub use trampoline::{
    BlockedUnit, EnterChildError, FlatTrampoline, ParentResume, SuspendedTrampoline,
    TrampolineCompletion,
};
