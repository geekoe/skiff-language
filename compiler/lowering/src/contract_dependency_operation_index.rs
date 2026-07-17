use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractOperationId, ContractRequirement, ServiceProtocolIdentity,
};

use crate::ServiceCallLoweringError;

/// Artifact-model-only handoff from the validated compiler input index. The T05
/// facade constructs this shape; lowering never imports compiler/input or a
/// provider artifact.
#[derive(Debug, Clone)]
pub struct ContractDependencyOperationIndexEntry {
    requirement: ContractRequirement,
    operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor>,
}

impl ContractDependencyOperationIndexEntry {
    pub fn new(
        requirement: ContractRequirement,
        operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor>,
    ) -> Self {
        Self {
            requirement,
            operations,
        }
    }
}

#[derive(Debug, Default)]
pub struct ContractDependencyOperationIndex {
    dependencies: BTreeMap<String, ContractDependencyOperationIndexEntry>,
}

impl ContractDependencyOperationIndex {
    pub fn build(
        entries: impl IntoIterator<Item = ContractDependencyOperationIndexEntry>,
    ) -> Result<Self, ServiceCallLoweringError> {
        let mut dependencies = BTreeMap::new();
        for entry in entries {
            let alias = entry.requirement.alias.clone();
            if alias.is_empty() {
                return Err(ServiceCallLoweringError::EmptyContractAlias);
            }
            for (map_key, operation) in &entry.operations {
                if map_key != &operation.operation_id {
                    return Err(ServiceCallLoweringError::OperationIdentityMismatch {
                        alias,
                        map_key: map_key.clone(),
                        nested_id: operation.operation_id.clone(),
                    });
                }
            }
            if dependencies.insert(alias.clone(), entry).is_some() {
                return Err(ServiceCallLoweringError::DuplicateContractAlias { alias });
            }
        }
        Ok(Self { dependencies })
    }

    pub fn len(&self) -> usize {
        self.dependencies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }

    pub fn requirement(
        &self,
        alias: &str,
    ) -> Result<&ContractRequirement, ServiceCallLoweringError> {
        Ok(&self.entry(alias)?.requirement)
    }

    pub fn operation(
        &self,
        alias: &str,
        operation_id: &ContractOperationId,
        expected_protocol_identity: &ServiceProtocolIdentity,
    ) -> Result<&BoundaryOperationDescriptor, ServiceCallLoweringError> {
        let entry = self.entry(alias)?;
        if &entry.requirement.expected_protocol_identity != expected_protocol_identity {
            return Err(ServiceCallLoweringError::ProtocolIdentityMismatch {
                alias: alias.to_string(),
                expected: entry.requirement.expected_protocol_identity.to_string(),
                actual: expected_protocol_identity.to_string(),
            });
        }
        entry.operations.get(operation_id).ok_or_else(|| {
            ServiceCallLoweringError::UnknownContractOperation {
                alias: alias.to_string(),
                operation_id: operation_id.clone(),
            }
        })
    }

    fn entry(
        &self,
        alias: &str,
    ) -> Result<&ContractDependencyOperationIndexEntry, ServiceCallLoweringError> {
        self.dependencies
            .get(alias)
            .ok_or_else(|| ServiceCallLoweringError::UnknownContractAlias {
                alias: alias.to_string(),
            })
    }
}
