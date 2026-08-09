use std::{fmt, sync::Arc};

use crate::{DeploymentImage, DeploymentOwnerIdentity};

/// Read-only proof that an entry belongs to one immutable program allocation.
pub trait DeploymentProgramEntry<P> {
    fn owner(&self) -> &DeploymentOwnerIdentity;

    fn program(&self) -> &Arc<P>;
}

/// One exact deployment image and one entry proven to belong to its program.
#[derive(Debug)]
pub struct PinnedDeploymentEntry<P, E> {
    image: Arc<DeploymentImage<P>>,
    entry: E,
}

impl<P, E> PinnedDeploymentEntry<P, E>
where
    E: DeploymentProgramEntry<P>,
{
    pub fn try_new(
        image: Arc<DeploymentImage<P>>,
        entry: E,
    ) -> Result<Self, PinnedDeploymentEntryError> {
        if image.owner() != entry.owner() {
            return Err(PinnedDeploymentEntryError::OwnerMismatch);
        }
        if !Arc::ptr_eq(image.program(), entry.program()) {
            return Err(PinnedDeploymentEntryError::ProgramMismatch);
        }
        Ok(Self { image, entry })
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        self.image.owner()
    }

    pub fn image(&self) -> &Arc<DeploymentImage<P>> {
        &self.image
    }

    pub fn entry(&self) -> &E {
        &self.entry
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinnedDeploymentEntryError {
    OwnerMismatch,
    ProgramMismatch,
}

impl fmt::Display for PinnedDeploymentEntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerMismatch => {
                formatter.write_str("deployment entry belongs to a different exact owner")
            }
            Self::ProgramMismatch => {
                formatter.write_str("deployment entry belongs to a different program allocation")
            }
        }
    }
}

impl std::error::Error for PinnedDeploymentEntryError {}
