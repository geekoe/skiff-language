mod attempt;
mod cache;
mod dependency;
mod load;
mod owner;
mod state;
mod value;

pub use cache::DeploymentImageCache;
pub use dependency::{ServiceDependencySlot, ServiceDependencySlotError};
pub use load::{
    DeploymentLoadError, DeploymentLoadFailure, DeploymentLoadFailureReason, DeploymentLoadResult,
    DeploymentOwnerConflict, LoadAttemptId,
};
pub use owner::DeploymentOwnerIdentity;
pub use value::DeploymentCacheValue;

#[cfg(test)]
mod tests;
