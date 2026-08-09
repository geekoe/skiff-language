use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::bytecode::{
    limits::LinkLimitTracker, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
    LinkLimits,
};

/// Links one exact, consumer-only hydrated deployment into an unverified
/// concrete candidate.
///
/// The input is borrowed so the independent verifier and deployment owner can
/// cross-check the candidate against the exact same opaque hydration. A
/// candidate is never owner, contract, ABI or structural-validation authority.
///
/// At this checkpoint the opaque hydration is cross-checked as an exact,
/// bounded closure, but no deployment is accepted: canonical root derivation
/// is waiting for the final v3 relocation DTO. Returning an empty candidate
/// here would therefore be an unchecked success path.
pub fn link_deployment(
    deployment: &HydratedDeploymentBytecode,
    limits: &LinkLimits,
) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> {
    let location = BytecodeLinkLocation::Deployment {
        deployment: deployment.reference().clone(),
    };
    let tracker = LinkLimitTracker::new(limits);
    tracker.check_packages(deployment.packages().len() as u64, location.clone())?;

    let implementation = &deployment.deployment().implementation;
    if !deployment
        .packages()
        .contains_key(&implementation.package_build_id)
    {
        return Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ExactPackageClosure,
            location,
            detail: format!(
                "hydrated closure does not contain implementation package {}",
                implementation.package_build_id
            ),
        });
    }
    for (package_build_id, package) in deployment.packages() {
        if package_build_id != &package.reference().package_build_id {
            return Err(BytecodeLinkError::UnsatisfiedObligation {
                obligation: BytecodeLinkObligation::ExactPackageClosure,
                location,
                detail: format!(
                    "hydrated package map key {package_build_id} disagrees with exact reference {}",
                    package.reference().package_build_id
                ),
            });
        }
    }

    Err(BytecodeLinkError::ImplementationUnavailable {
        obligation: BytecodeLinkObligation::CanonicalRootSet,
        location,
    })
}
