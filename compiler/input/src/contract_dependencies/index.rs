use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractOperationId, ContractRequirement, PackageSchemaTypeId,
    PackageSchemaTypeRecord, ServiceContract,
};

use super::{ContractDependencyError, ResolvedContractDependency};

#[derive(Debug, Clone)]
struct IndexedContractDependency {
    dependency: ResolvedContractDependency,
    operations_by_stable_key: BTreeMap<String, ContractOperationId>,
    types_by_stable_key: BTreeMap<String, PackageSchemaTypeId>,
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

    pub fn package_schema_type(
        &self,
        alias: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<&PackageSchemaTypeRecord, ContractDependencyError> {
        self.entry(alias)?
            .dependency
            .schema_records()
            .get(package_schema_type_id)
            .ok_or_else(|| ContractDependencyError::UnknownType {
                alias: alias.to_string(),
                package_schema_type_id: package_schema_type_id.clone(),
            })
    }

    pub(super) fn package_schema_type_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&PackageSchemaTypeRecord, ContractDependencyError> {
        let entry = self.entry(alias)?;
        let type_id = entry.types_by_stable_key.get(stable_key).ok_or_else(|| {
            ContractDependencyError::UnknownTypeStableKey {
                alias: alias.to_string(),
                stable_key: stable_key.to_string(),
            }
        })?;
        Ok(&entry.dependency.schema_records()[type_id])
    }

    /// Resolves only source-nameable contract nominal types. Closure-only
    /// schema entries remain available to descriptor closure validation but
    /// can never be selected by a qualified source name.
    pub fn public_package_type_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&PackageSchemaTypeRecord, ContractDependencyError> {
        self.package_schema_type_by_stable_key(alias, stable_key)
    }

    pub fn package_type_by_owner_and_stable_key(
        &self,
        package_id: &str,
        stable_key: &str,
    ) -> Option<&PackageSchemaTypeRecord> {
        self.dependencies
            .values()
            .filter_map(|entry| {
                entry.dependency.schema_records().values().find(|record| {
                    record.package_id == package_id && record.stable_schema_key == stable_key
                })
            })
            .next()
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
            .schema_records()
            .values()
            .map(|schema_type| {
                (
                    schema_type.stable_schema_key.clone(),
                    schema_type.package_schema_type_id.clone(),
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
