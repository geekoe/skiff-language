use std::sync::Arc;

use crate::{DeploymentImage, DeploymentOwnerIdentity};

/// Strong pin held for the full lifetime of one provider invocation or carrier.
#[derive(Debug)]
pub struct PinnedProviderImage<P> {
    image: Arc<DeploymentImage<P>>,
}

impl<P> Clone for PinnedProviderImage<P> {
    fn clone(&self) -> Self {
        Self {
            image: Arc::clone(&self.image),
        }
    }
}

impl<P> PinnedProviderImage<P> {
    pub fn new(image: Arc<DeploymentImage<P>>) -> Self {
        Self { image }
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        self.image.owner()
    }

    pub fn image(&self) -> &Arc<DeploymentImage<P>> {
        &self.image
    }
}
