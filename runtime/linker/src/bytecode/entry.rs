use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::bytecode::{link::DeploymentLinker, BytecodeLinkError, LinkLimits};

/// Links one exact, consumer-only hydrated deployment into a concrete
/// candidate.
///
/// The input is borrowed so candidate construction uses the exact same opaque
/// hydration as the deployment owner. The linker is not source-semantic
/// authority and does not reconstruct compiler admission.
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
