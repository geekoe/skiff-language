use crate::DeploymentOwnerIdentity;

/// Narrow identity fence required by the deployment-build cache.
///
/// The cache stores the final value directly and neither constructs nor wraps
/// it. Production bytecode has exactly one implementation: the linker-owned
/// `DeploymentExecutionImage`.
pub trait DeploymentCacheValue {
    fn owner(&self) -> &DeploymentOwnerIdentity;
}
