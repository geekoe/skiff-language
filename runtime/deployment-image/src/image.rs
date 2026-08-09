use std::{collections::BTreeMap, fmt, sync::Arc};

use skiff_artifact_model::ServiceRequirementKey;

use crate::{DeploymentOwnerIdentity, ServiceDependencySlot};

/// Canonical deployment facts sealed into one program payload.
pub trait DeploymentProgramFacts {
    fn owner(&self) -> &DeploymentOwnerIdentity;

    fn dependency_slots(&self) -> &[ServiceDependencySlot];
}

/// Immutable exact-deployment image with a generic verified program payload.
#[derive(Debug)]
pub struct DeploymentImage<P> {
    owner: DeploymentOwnerIdentity,
    program: Arc<P>,
    dependency_slots: BTreeMap<ServiceRequirementKey, ServiceDependencySlot>,
}

impl<P> DeploymentImage<P>
where
    P: DeploymentProgramFacts,
{
    pub fn try_new(program: Arc<P>) -> Result<Self, DeploymentImageError> {
        let owner = program.owner().clone();
        let mut slots_by_key = BTreeMap::new();
        for slot in program.dependency_slots().iter().cloned() {
            let key = slot.key().clone();
            if slots_by_key.insert(key.clone(), slot).is_some() {
                return Err(DeploymentImageError::DuplicateDependencyKey { key });
            }
        }

        Ok(Self {
            owner,
            program,
            dependency_slots: slots_by_key,
        })
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    pub fn program(&self) -> &Arc<P> {
        &self.program
    }

    pub fn dependency_slots(&self) -> impl ExactSizeIterator<Item = &ServiceDependencySlot> {
        self.dependency_slots.values()
    }

    pub fn dependency_slot(&self, key: &ServiceRequirementKey) -> Option<&ServiceDependencySlot> {
        self.dependency_slots.get(key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentImageError {
    DuplicateDependencyKey { key: ServiceRequirementKey },
}

impl fmt::Display for DeploymentImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDependencyKey { key } => write!(
                formatter,
                "duplicate service dependency key for package build {} slot {}",
                key.caller_package_build_id, key.service_requirement_slot
            ),
        }
    }
}

impl std::error::Error for DeploymentImageError {}
