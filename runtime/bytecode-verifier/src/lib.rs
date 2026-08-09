//! Independent semantic verification boundary for linked bytecode.
//!
//! [`LinkedBytecodeCandidate`](skiff_runtime_linked_bytecode::LinkedBytecodeCandidate)
//! is deliberately untrusted. The only public transition to
//! [`VerifiedLinkedBytecodeImage`] is [`verify`], and the verified image exposes
//! only shared views of the candidate it seals.

mod error;
mod limits;
mod verifier;

pub use error::{
    VerificationError, VerificationLimit, VerificationLocation, VerificationObligation,
};
pub use limits::VerificationLimits;
pub use verifier::{verify, VerifiedLinkedBytecodeImage};

#[cfg(test)]
mod tests;
