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
mod execution_image;
mod limits;
mod link;
mod stack_map;
mod types;
mod worklist;

pub(crate) use entry::link_deployment;
#[cfg(test)]
use entry::link_deployment_backend_for_test;
pub use error::{
    BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation, BytecodeLinkObligation,
    Phase1LinkedCapability,
};
pub use execution_image::{
    link_deployment_execution_image, CodeEntryLookupError, DeploymentExecutionEntry,
    DeploymentExecutionImage, DeploymentExecutionImageError, DeploymentExecutionLimits,
    DeploymentHostEffectTarget,
};
pub use limits::LinkLimits;

#[cfg(test)]
mod tests;

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

// Keep the exact consumer-to-candidate boundary visible to rustc independently
// of which link obligations the current fail-closed implementation supports.
const _: fn(
    &HydratedDeploymentBytecode,
    &LinkLimits,
) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> = link_deployment;
