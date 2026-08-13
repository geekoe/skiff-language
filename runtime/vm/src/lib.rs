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
mod limits;
mod projection;
mod statement;

pub use budget::{
    VmBudget, VmBudgetClosed, VmBudgetTerminal, VmSemanticCharge, VmSemanticChargeKind,
};
pub use control::{
    AdapterControl, AdapterInvocation, BoundaryStart, ChildInvocation, ChildTarget, EffectStart,
    PendingOperation, PendingTicket, ResumeOutcome, StreamInvocation, StreamItem, VmControl,
    VmInternalTerminal, VmOwnedValues, VmResult, VmResumeKind, VmResumeToken,
};
pub use error::{VmEntryArgumentRejection, VmError, VmValueLocation, VmVerifiedInvariant};
pub use fiber::{Vm, VmFiber, VmFiberState};
pub use limits::VmLimits;
pub use projection::VmProjectionHandoff;
pub use skiff_runtime_linker::DeploymentExecutionEntry;
