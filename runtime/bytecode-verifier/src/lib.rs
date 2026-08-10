//! Independent semantic verification boundary for linked bytecode.
//!
//! [`LinkedBytecodeCandidate`](skiff_runtime_linked_bytecode::LinkedBytecodeCandidate)
//! is deliberately untrusted and carries no deployment provenance. The only
//! public transition to [`VerifiedLinkedBytecodeImage`] is [`verify`], which
//! consumes the exact opaque deployment hydration and candidate together.

mod admission;
mod attribution;
mod concrete_values;
mod control_flow;
mod effects;
mod error;
mod limits;
mod resume;
mod verifier;

pub use attribution::{VerifiedStatementEvent, VerifiedStatementSchedule};
pub use effects::{VerifiedCallableEffects, VerifiedFunctionEffects};
pub use error::{
    VerificationError, VerificationLimit, VerificationLocation, VerificationObligation,
};
pub use limits::VerificationLimits;
pub use resume::{VerifiedResumeKind, VerifiedResumeSite, VerifiedResumeSites};
pub use verifier::{
    verify, CodeEntryLookupError, VerifiedCodeEntry, VerifiedCodeEntryKind, VerifiedConstantHeap,
    VerifiedLinkedBytecodeImage,
};

#[cfg(test)]
mod tests;
