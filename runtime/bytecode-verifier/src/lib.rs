//! Independent semantic verification boundary for linked bytecode.
//!
//! [`LinkedBytecodeCandidate`](skiff_runtime_linked_bytecode::LinkedBytecodeCandidate)
//! is deliberately untrusted and carries no deployment provenance. The only
//! public transition to [`VerifiedLinkedBytecodeImage`] is [`verify`], which
//! consumes the exact opaque deployment hydration and candidate together.

mod error;
mod limits;
mod verifier;

pub use error::{
    VerificationError, VerificationLimit, VerificationLocation, VerificationObligation,
};
pub use limits::VerificationLimits;
pub use verifier::{
    verify, CodeEntryLookupError, VerifiedCodeEntry, VerifiedCodeEntryKind, VerifiedConstantHeap,
    VerifiedLinkedBytecodeImage,
};

#[cfg(test)]
mod tests;
