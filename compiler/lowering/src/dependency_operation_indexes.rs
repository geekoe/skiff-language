use std::collections::BTreeMap;

use skiff_artifact_model::OperationAbiRef;
use skiff_compiler_source::{PackageSourceModel, SourceCompileError as PublicationError};
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

    /// Legacy operation-ABI indexes stay empty on the canonical package path.
    /// Contract calls are materialized separately from typed contract facts.
    pub(crate) fn build_for_contract_service_calls(
        _model: &PackageSourceModel,
    ) -> std::result::Result<Self, PublicationError> {
        Ok(Self::new(
            PackageOperationIndex::default(),
            ServiceDependencyOperationIndex::default(),
        ))
    }
}
