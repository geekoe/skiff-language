//! Independent semantic verification boundary for linked bytecode.
//!
//! [`LinkedBytecodeCandidate`](skiff_runtime_linked_bytecode::LinkedBytecodeCandidate)
//! is deliberately untrusted and carries no deployment provenance. This crate
//! returns construction-only [`ExecutableFacts`]; the linker remains the sole
//! executable-image authority.

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
pub use verifier::{verify_executable_facts, ExecutableFacts, VerifiedConstantHeap};

#[cfg(test)]
pub(crate) fn verify_facts(
    hydrated: skiff_runtime_loader::HydratedDeploymentBytecode,
    candidate: skiff_runtime_linked_bytecode::LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<ExecutableFacts, VerificationError> {
    verify_executable_facts(&hydrated, &candidate, limits)
}

#[cfg(test)]
mod tests;
