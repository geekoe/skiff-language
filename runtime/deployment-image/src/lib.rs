mod dependency;
mod image;
mod load;
mod owner;
mod pin;

pub use dependency::{ServiceDependencySlot, ServiceDependencySlotError};
pub use image::{DeploymentImage, DeploymentImageError};
pub use load::{DeploymentLoadFailure, DeploymentLoadResult, LoadAttemptId};
pub use owner::DeploymentOwnerIdentity;
pub use pin::PinnedProviderImage;

#[cfg(test)]
mod tests;
