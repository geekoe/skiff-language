use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::bytecode::{link::DeploymentLinker, BytecodeLinkError, LinkLimits};

/// Links one exact, consumer-only hydrated deployment into an unverified
/// concrete candidate.
///
/// The input is borrowed so the independent verifier and deployment owner can
/// cross-check the candidate against the exact same opaque hydration. A
/// candidate is never owner, contract, ABI or structural-validation authority.
///
/// The current implementation deliberately recognizes only the exact local,
/// non-generic closure for which it can construct every candidate fact. Any
/// graph requiring an unimplemented authority or target family fails before
/// candidate construction; there is no partial or placeholder success path.
pub(crate) fn link_deployment(
    deployment: &HydratedDeploymentBytecode,
    limits: &LinkLimits,
) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> {
    DeploymentLinker::new(deployment, limits).link()
}

#[cfg(test)]
pub(super) fn link_deployment_backend_for_test(
    deployment: &HydratedDeploymentBytecode,
    limits: &LinkLimits,
) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> {
    DeploymentLinker::new(deployment, limits).link_backend_for_test()
}
