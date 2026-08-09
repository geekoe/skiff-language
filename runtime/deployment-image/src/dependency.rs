use std::fmt;

use skiff_artifact_model::{ContractOperationId, ServiceContractRef, ServiceRequirementKey};

/// Contract-only facts for one deployment-owned service dependency edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDependencySlot {
    key: ServiceRequirementKey,
    contract: ServiceContractRef,
    used_operations: Box<[ContractOperationId]>,
}

impl ServiceDependencySlot {
    pub fn try_new(
        key: ServiceRequirementKey,
        contract: ServiceContractRef,
        used_operations: impl IntoIterator<Item = ContractOperationId>,
    ) -> Result<Self, ServiceDependencySlotError> {
        let mut used_operations = used_operations.into_iter().collect::<Vec<_>>();
        used_operations.sort_unstable();

        if let Some(duplicate) = used_operations
            .windows(2)
            .find(|operations| operations[0] == operations[1])
        {
            return Err(ServiceDependencySlotError::DuplicateOperation {
                operation_id: duplicate[0].clone(),
            });
        }

        Ok(Self {
            key,
            contract,
            used_operations: used_operations.into_boxed_slice(),
        })
    }

    pub fn key(&self) -> &ServiceRequirementKey {
        &self.key
    }

    pub fn contract(&self) -> &ServiceContractRef {
        &self.contract
    }

    pub fn used_operations(&self) -> &[ContractOperationId] {
        &self.used_operations
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceDependencySlotError {
    DuplicateOperation { operation_id: ContractOperationId },
}

impl fmt::Display for ServiceDependencySlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOperation { operation_id } => {
                write!(
                    formatter,
                    "duplicate service dependency operation {operation_id}"
                )
            }
        }
    }
}

impl std::error::Error for ServiceDependencySlotError {}
