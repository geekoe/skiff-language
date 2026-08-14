//! Deployment bytecode link boundary.
//!
//! The public [`link_deployment_execution_image`] entry point accepts the
//! loader's opaque, exact consumer input and atomically constructs the only
//! executable image. The raw [`link_deployment`] candidate remains private to
//! that constructor.
//!
//! This module never accepts a raw bytecode artifact and never resolves a
//! service provider executable.

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
    DeploymentExecutionImage, DeploymentExecutionImageError, DeploymentHostEffectTarget,
    ExecutionConstantHeap, ExecutionImageConstructionError, ExecutionResumeKind,
    ExecutionResumeSite, ExecutionResumeSites, ExecutionStatementEvent, ExecutionStatementSchedule,
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
