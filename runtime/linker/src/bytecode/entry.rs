use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::bytecode::{
    BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation, LinkLimits,
};

/// Links one exact, consumer-only hydrated deployment into an unverified
/// concrete candidate.
///
/// The input is borrowed so the independent verifier and deployment owner can
/// cross-check the candidate against the exact same opaque hydration. A
/// candidate is never owner, contract, ABI or structural-validation authority.
///
/// At this interface checkpoint no deployment is accepted, including an
/// apparently empty one: proving that a deployment has no roots itself
/// requires the package-closure and canonical-root obligations. Returning an
/// empty candidate here would therefore be an unchecked success path.
pub fn link_deployment(
    deployment: &HydratedDeploymentBytecode,
    _limits: &LinkLimits,
) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> {
    Err(BytecodeLinkError::ImplementationUnavailable {
        obligation: BytecodeLinkObligation::ExactPackageClosure,
        location: BytecodeLinkLocation::Deployment {
            deployment: deployment.reference().clone(),
        },
    })
}
