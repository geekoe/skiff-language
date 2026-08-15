//! Synchronous, deployment-pinned bytecode VM core.
//!
//! The only production admission type is [`DeploymentExecutionEntry`], an
//! image-owned exact entry pin into the sole deployment execution image. This
//! crate exposes no candidate/raw-image execution path and no
//! callback capable of performing host effects. External work leaves the core
//! exclusively through typed [`VmControl`] values.

#![forbid(unsafe_code)]

mod admission;
mod budget;
mod control;
mod error;
mod fiber;
mod frame;
mod lifecycle;
mod limits;
mod local_interface;
mod projection;
mod statement;
mod terminal_ownership;

pub use budget::{
    VmBudget, VmBudgetClosed, VmBudgetTerminal, VmSemanticCharge, VmSemanticChargeKind,
};
pub use control::{
    AdapterControl, AdapterInvocation, BoundaryStart, ChildInvocation, ChildTarget, EffectStart,
    InterfaceCallPlan, PendingOperation, PendingTicket, RemoteInterfaceCallPlan, ResumeOutcome,
    StreamEndpointRef, StreamInvocation, StreamItem, StreamItemReleaseError, TaskDispatchIndex,
    TaskDispatchRequest, VmCompletion, VmControl, VmHostEffectArguments,
    VmHostEffectArgumentsReleaseError, VmInternalTerminal, VmLifecycleSite, VmOwnedException,
    VmOwnedExceptionRejected, VmOwnedValues, VmOwnedValuesRejected, VmResult, VmResumeFailure,
    VmResumeKind, VmResumeToken, VmTerminalCause, VmTerminalEscrow, VmThrownDiagnostic,
};
pub use error::{VmEntryArgumentRejection, VmError, VmValueLocation, VmVerifiedInvariant};
pub use fiber::{Vm, VmFiber, VmFiberState};
pub use limits::VmLimits;
pub use local_interface::{
    materialize_local_interface_value, release_local_interface_source,
    LocalInterfaceMaterializeError,
};
pub use projection::VmProjectionHandoff;
pub use skiff_runtime_linker::DeploymentExecutionEntry;
