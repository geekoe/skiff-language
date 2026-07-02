use std::collections::BTreeMap;

use skiff_artifact_model::{OperationAbiRef, ServiceDependencyConstraint};
use skiff_compiler_source::{
    DependencyPackageOperationFacts, ResolvedDependencies, SourceCompileError as PublicationError,
    SourceCompileModel,
};
use skiff_syntax::error::{CompileError, Result};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackageOperationIndex {
    operations: BTreeMap<PackageOperationKey, OperationAbiRef>,
    duplicate_keys: BTreeMap<PackageOperationKey, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PackageOperationKey {
    package_ref: String,
    source_call_path: String,
}

impl PackageOperationIndex {
    pub fn insert_operation(
        &mut self,
        package_ref: impl Into<String>,
        source_call_path: impl Into<String>,
        operation: OperationAbiRef,
    ) {
        let key = PackageOperationKey {
            package_ref: package_ref.into(),
            source_call_path: source_call_path.into(),
        };
        if let Some(existing) = self.operations.get(&key) {
            let duplicate_ids = self
                .duplicate_keys
                .entry(key)
                .or_insert_with(|| vec![existing.operation_abi_id.clone()]);
            duplicate_ids.push(operation.operation_abi_id);
            return;
        }
        self.operations.insert(key, operation);
    }

    pub(super) fn resolve(
        &self,
        package_ref: &str,
        source_call_path: &str,
    ) -> Result<Option<&OperationAbiRef>> {
        let key = PackageOperationKey {
            package_ref: package_ref.to_string(),
            source_call_path: source_call_path.to_string(),
        };
        if let Some(operation_abi_ids) = self.duplicate_keys.get(&key) {
            return Err(CompileError::Semantic(format!(
                "package dependency `{package_ref}` publication ABI has duplicate source-call path `{source_call_path}` for operation ABI ids {:?}",
                operation_abi_ids
            )));
        }
        Ok(self.operations.get(&key))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceDependencyOperationIndex {
    operations: BTreeMap<ServiceDependencyOperationKey, OperationAbiRef>,
    duplicate_keys: BTreeMap<ServiceDependencyOperationKey, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ServiceDependencyOperationKey {
    dependency_ref: String,
    source_call_path: String,
}

impl ServiceDependencyOperationIndex {
    pub fn insert_operation(
        &mut self,
        dependency_ref: impl Into<String>,
        source_call_path: impl Into<String>,
        operation: OperationAbiRef,
    ) {
        let key = ServiceDependencyOperationKey {
            dependency_ref: dependency_ref.into(),
            source_call_path: source_call_path.into(),
        };
        if let Some(existing) = self.operations.get(&key) {
            let duplicate_ids = self
                .duplicate_keys
                .entry(key)
                .or_insert_with(|| vec![existing.operation_abi_id.clone()]);
            duplicate_ids.push(operation.operation_abi_id);
            return;
        }
        self.operations.insert(key, operation);
    }

    pub(super) fn resolve(
        &self,
        dependency_ref: &str,
        source_call_path: &str,
    ) -> Result<Option<&OperationAbiRef>> {
        let key = ServiceDependencyOperationKey {
            dependency_ref: dependency_ref.to_string(),
            source_call_path: source_call_path.to_string(),
        };
        if let Some(operation_abi_ids) = self.duplicate_keys.get(&key) {
            return Err(CompileError::Semantic(format!(
                "service dependency `{dependency_ref}` publication ABI has duplicate source-call path `{source_call_path}` for operation ABI ids {:?}",
                operation_abi_ids
            )));
        }
        Ok(self.operations.get(&key))
    }
}

#[derive(Debug)]
pub(crate) struct LoweringDependencyOperationIndexes {
    package_operations: PackageOperationIndex,
    service_dependency_operations: ServiceDependencyOperationIndex,
}

impl LoweringDependencyOperationIndexes {
    pub(crate) fn new(
        package_operations: PackageOperationIndex,
        service_dependency_operations: ServiceDependencyOperationIndex,
    ) -> Self {
        Self {
            package_operations,
            service_dependency_operations,
        }
    }

    pub(crate) fn package_operations(&self) -> &PackageOperationIndex {
        &self.package_operations
    }

    pub(crate) fn service_dependency_operations(&self) -> &ServiceDependencyOperationIndex {
        &self.service_dependency_operations
    }

    pub(crate) fn build(model: &SourceCompileModel) -> std::result::Result<Self, PublicationError> {
        let dependencies = model.dependencies();
        Ok(Self::new(
            package_operation_index(
                dependencies,
                dependencies.dependency_package_operation_facts(),
            )?,
            service_dependency_operation_index(dependencies.service_dependencies().constraints()),
        ))
    }
}

fn package_operation_index(
    dependencies: &ResolvedDependencies,
    package_operation_facts: &[DependencyPackageOperationFacts],
) -> std::result::Result<PackageOperationIndex, PublicationError> {
    let mut index = PackageOperationIndex::default();
    for package in package_operation_facts {
        let package_refs = package_dependency_refs_for_operation_index(package, dependencies);
        for source_call in package.source_call_operations() {
            for package_ref in &package_refs {
                index.insert_operation(
                    package_ref.clone(),
                    source_call.source_call_path.clone(),
                    source_call.operation.clone(),
                );
            }
        }
    }
    Ok(index)
}

fn service_dependency_operation_index(
    dependencies: &[ServiceDependencyConstraint],
) -> ServiceDependencyOperationIndex {
    let mut index = ServiceDependencyOperationIndex::default();
    for dependency in dependencies {
        for source_call in &dependency.publication_abi.source_call_operation_index {
            index.insert_operation(
                dependency.alias.clone(),
                source_call.source_call_path.clone(),
                source_call.operation.clone(),
            );
        }
    }
    index
}

fn package_dependency_refs_for_operation_index(
    package: &DependencyPackageOperationFacts,
    dependencies: &ResolvedDependencies,
) -> Vec<String> {
    dependencies.package_operation_refs(package.package_id(), package.version())
}
