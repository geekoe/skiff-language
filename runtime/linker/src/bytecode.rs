//! Deployment bytecode link boundary.
//!
//! [`link_deployment`] is the only bytecode-link entry point. It accepts the
//! loader's opaque, exact consumer input and produces an unverified
//! [`LinkedBytecodeCandidate`]. The input is borrowed deliberately: the
//! deployment owner and independent verifier must retain the same hydrated
//! owner, contract and validated-artifact facts and must not trust facts copied
//! into the candidate as authority.
//!
//! This module never accepts a raw bytecode artifact, never resolves a service
//! provider executable and never creates a verified-image token.

mod entry;
mod error;
mod limits;

pub use entry::link_deployment;
pub use error::{
    BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation, BytecodeLinkObligation,
};
pub use limits::LinkLimits;

#[cfg(test)]
mod tests;

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

// Keep this exact consumer-to-candidate boundary visible to rustc even when
// the implementation remains fail-closed at the interface checkpoint.
const _: fn(
    &HydratedDeploymentBytecode,
    &LinkLimits,
) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> = link_deployment;
