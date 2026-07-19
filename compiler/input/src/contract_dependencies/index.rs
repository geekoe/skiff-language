use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractOperationId, ContractRequirement, ContractSchemaType,
    ContractTypeId, ContractTypeNameability, ServiceContract,
};

use super::{ContractDependencyError, ResolvedContractDependency};

#[derive(Debug, Clone)]
struct IndexedContractDependency {
    dependency: ResolvedContractDependency,
    operations_by_stable_key: BTreeMap<String, ContractOperationId>,
    types_by_stable_key: BTreeMap<String, ContractTypeId>,
}

/// Strict alias/type/operation index over already validated ServiceContracts.
/// It contains no provider package, build, deployment, route, or executable
/// facts.
#[derive(Debug, Clone, Default)]
pub struct ContractDependencyIndex {
    dependencies: BTreeMap<String, IndexedContractDependency>,
}

impl ContractDependencyIndex {
    pub fn build(
        dependencies: impl IntoIterator<Item = ResolvedContractDependency>,
    ) -> Result<Self, ContractDependencyError> {
        let mut indexed = BTreeMap::new();
        for dependency in dependencies {
            let alias = dependency.requirement().alias.clone();
            let entry = IndexedContractDependency::new(dependency);
            if indexed.insert(alias.clone(), entry).is_some() {
                return Err(ContractDependencyError::DuplicateAlias { alias });
            }
        }
        Ok(Self {
            dependencies: indexed,
        })
    }

    pub fn len(&self) -> usize {
        self.dependencies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }

    pub fn dependencies(&self) -> impl Iterator<Item = &ResolvedContractDependency> {
        self.dependencies.values().map(|entry| &entry.dependency)
    }

    pub fn requirement(
        &self,
        alias: &str,
    ) -> Result<&ContractRequirement, ContractDependencyError> {
        Ok(self.entry(alias)?.dependency.requirement())
    }

    pub fn contract(&self, alias: &str) -> Result<&ServiceContract, ContractDependencyError> {
        Ok(self.entry(alias)?.dependency.contract())
    }

    pub fn operation(
        &self,
        alias: &str,
        operation_id: &ContractOperationId,
    ) -> Result<&BoundaryOperationDescriptor, ContractDependencyError> {
        self.contract(alias)?
            .operations
            .get(operation_id)
            .ok_or_else(|| ContractDependencyError::UnknownOperation {
                alias: alias.to_string(),
                operation_id: operation_id.clone(),
            })
    }

    pub fn operation_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&BoundaryOperationDescriptor, ContractDependencyError> {
        let entry = self.entry(alias)?;
        let operation_id = entry
            .operations_by_stable_key
            .get(stable_key)
            .ok_or_else(|| ContractDependencyError::UnknownOperationStableKey {
                alias: alias.to_string(),
                stable_key: stable_key.to_string(),
            })?;
        Ok(&entry.dependency.contract().operations[operation_id])
    }

    pub fn contract_type(
        &self,
        alias: &str,
        contract_type_id: &ContractTypeId,
    ) -> Result<&ContractSchemaType, ContractDependencyError> {
        self.contract(alias)?
            .boundary_schema
            .get(contract_type_id)
            .ok_or_else(|| ContractDependencyError::UnknownType {
                alias: alias.to_string(),
                contract_type_id: contract_type_id.clone(),
            })
    }

    pub(super) fn contract_schema_type_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&ContractSchemaType, ContractDependencyError> {
        let entry = self.entry(alias)?;
        let type_id = entry.types_by_stable_key.get(stable_key).ok_or_else(|| {
            ContractDependencyError::UnknownTypeStableKey {
                alias: alias.to_string(),
                stable_key: stable_key.to_string(),
            }
        })?;
        Ok(&entry.dependency.contract().boundary_schema[type_id])
    }

    /// Resolves only source-nameable contract nominal types. Closure-only
    /// schema entries remain available to descriptor closure validation but
    /// can never be selected by a qualified source name.
    pub fn public_contract_type_id_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&ContractTypeId, ContractDependencyError> {
        let schema_type = self.contract_schema_type_by_stable_key(alias, stable_key)?;
        if schema_type.shape.nameability != ContractTypeNameability::PublicNameable {
            return Err(ContractDependencyError::ContractTypeNotPublicNameable {
                alias: alias.to_string(),
                stable_key: stable_key.to_string(),
                contract_type_id: schema_type.contract_type_id.clone(),
            });
        }
        Ok(&schema_type.contract_type_id)
    }

    fn entry(&self, alias: &str) -> Result<&IndexedContractDependency, ContractDependencyError> {
        self.dependencies
            .get(alias)
            .ok_or_else(|| ContractDependencyError::UnknownAlias {
                alias: alias.to_string(),
            })
    }
}

impl IndexedContractDependency {
    fn new(dependency: ResolvedContractDependency) -> Self {
        let operations_by_stable_key = dependency
            .contract()
            .operations
            .values()
            .map(|operation| (operation.stable_key.clone(), operation.operation_id.clone()))
            .collect();
        let types_by_stable_key = dependency
            .contract()
            .boundary_schema
            .values()
            .map(|schema_type| {
                (
                    schema_type.stable_key.clone(),
                    schema_type.contract_type_id.clone(),
                )
            })
            .collect();
        Self {
            dependency,
            operations_by_stable_key,
            types_by_stable_key,
        }
    }
}
