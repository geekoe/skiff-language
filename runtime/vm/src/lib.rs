//! Synchronous, deployment-pinned bytecode VM core.
//!
//! The only production admission type is [`VerifiedVmEntry`], which combines
//! an opaque verifier seal, a concrete typed entry and an exact deployment
//! image pin. This crate exposes no candidate/raw-image execution path and no
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

pub use budget::{VmBudget, VmBudgetError, VmSemanticCharge, VmSemanticChargeKind};
pub use control::{
    AdapterControl, AdapterInvocation, BoundaryStart, ChildInvocation, ChildTarget, EffectStart,
    PendingOperation, PendingTicket, ResumeOutcome, StreamInvocation, StreamItem, VmControl,
    VmInternalTerminal, VmOwnedValues, VmResult, VmResumeKind, VmResumeToken,
};
pub use error::{VmEntryArgumentRejection, VmError, VmValueLocation, VmVerifiedInvariant};
pub use fiber::{VerifiedVmEntry, Vm, VmFiber, VmFiberState};
pub use limits::VmLimits;
