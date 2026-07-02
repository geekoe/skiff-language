use std::collections::BTreeMap;

use crate::context::ProjectedPackageDependency;
use crate::error::ProjectionError;
use crate::typed_artifacts::ServiceDependencyConstraint;
use skiff_artifact_model::{CallTargetIr, ExprIr, FileIrUnit, ServiceDependencySymbolRef};

pub struct ServiceDependencyAbiResolutionInput<'a> {
    pub dependencies: &'a [ServiceDependencyConstraint],
    pub service_file_units: &'a mut [FileIrUnit],
}

pub fn resolve_service_dependency_symbol_refs(
    input: ServiceDependencyAbiResolutionInput<'_>,
) -> Result<(), ProjectionError> {
    resolve_service_dependency_symbol_refs_inner(input.dependencies, input.service_file_units)
}

pub struct ServiceDependencyAbiValidationInput<'a> {
    pub dependencies: &'a [ServiceDependencyConstraint],
    pub service_file_units: &'a [FileIrUnit],
}

pub fn validate_service_dependency_symbol_refs(
    input: ServiceDependencyAbiValidationInput<'_>,
) -> Result<(), ProjectionError> {
    validate_service_dependency_symbol_refs_inner(input.dependencies, input.service_file_units)
}

pub fn package_dependency_refs(
    dependencies: &[ProjectedPackageDependency],
    package_id: &str,
) -> Vec<String> {
    let mut refs = dependencies
        .iter()
        .filter(|dependency| dependency.id == package_id)
        .flat_map(|dependency| {
            let mut refs = vec![
                dependency.effective_alias().to_string(),
                dependency.id.clone(),
            ];
            if let Some(alias) = &dependency.alias {
                refs.push(alias.clone());
            }
            refs
        })
        .collect::<Vec<_>>();
    refs.sort();
    refs.dedup();
    refs
}

fn validate_service_dependency_symbol_refs_inner(
    dependencies: &[ServiceDependencyConstraint],
    service_file_units: &[FileIrUnit],
) -> Result<(), ProjectionError> {
    let index = ServiceDependencySymbolRefIndex::new(dependencies)?;

    for unit in service_file_units {
        for symbol in &unit.external_refs.service_dependency_symbols {
            validate_resolved_service_dependency_symbol_ref(&index, &unit.module_path, symbol)?;
        }
    }
    Ok(())
}

fn resolve_service_dependency_symbol_refs_inner(
    dependencies: &[ServiceDependencyConstraint],
    service_file_units: &mut [FileIrUnit],
) -> Result<(), ProjectionError> {
    let index = ServiceDependencySymbolRefIndex::new(dependencies)?;

    for unit in service_file_units {
        for symbol in &mut unit.external_refs.service_dependency_symbols {
            resolve_service_dependency_symbol_ref(&index, &unit.module_path, symbol)?;
        }
        for executable in &mut unit.executables {
            for expr in &mut executable.body.expressions {
                if let ExprIr::Call { call } = expr {
                    if let CallTargetIr::ServiceDependencySymbol { symbol } = &mut call.target {
                        resolve_service_dependency_symbol_ref(&index, &unit.module_path, symbol)?;
                    }
                }
            }
        }
    }
    Ok(())
}

struct ServiceDependencySymbolRefIndex<'a> {
    dependencies_by_alias: BTreeMap<&'a str, &'a ServiceDependencyConstraint>,
}

impl<'a> ServiceDependencySymbolRefIndex<'a> {
    fn new(dependencies: &'a [ServiceDependencyConstraint]) -> Result<Self, ProjectionError> {
        let mut dependencies_by_alias = BTreeMap::new();

        for dependency in dependencies {
            if dependencies_by_alias
                .insert(dependency.alias.as_str(), dependency)
                .is_some()
            {
                return Err(ProjectionError::ContractValidation {
                    message: format!(
                        "service dependency alias {} is declared more than once",
                        dependency.alias
                    ),
                });
            }
        }

        Ok(Self {
            dependencies_by_alias,
        })
    }
}

fn resolve_service_dependency_symbol_ref(
    index: &ServiceDependencySymbolRefIndex<'_>,
    module_path: &str,
    symbol: &mut ServiceDependencySymbolRef,
) -> Result<(), ProjectionError> {
    validate_resolved_service_dependency_symbol_ref(index, module_path, symbol)
}

fn validate_resolved_service_dependency_symbol_ref(
    index: &ServiceDependencySymbolRefIndex<'_>,
    module_path: &str,
    symbol: &ServiceDependencySymbolRef,
) -> Result<(), ProjectionError> {
    let source_call_path = symbol.operation.public_path.as_str();
    let operation_abi_id = symbol.operation.operation_abi_id.as_str();
    validate_service_dependency_alias(
        index,
        module_path,
        &symbol.dependency_ref,
        source_call_path,
    )?;
    if operation_abi_id.is_empty() {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "{module_path} references service dependency {} source-call path {} without operationAbiId after dependency ABI resolution",
                symbol.dependency_ref, source_call_path
            ),
        });
    }
    let dependency = index
        .dependencies_by_alias
        .get(symbol.dependency_ref.as_str())
        .expect("validated service dependency alias must exist");
    let Some(exported_operation) = dependency
        .publication_abi
        .operation_exports
        .iter()
        .find(|operation| operation.operation_abi_id == operation_abi_id)
    else {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "{module_path} references service dependency {} source-call path {} resolving to operationAbiId {}, but dependency publicationAbi.operationExports does not declare it",
                symbol.dependency_ref, source_call_path, operation_abi_id
            ),
        });
    };
    if exported_operation != &symbol.operation {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "{module_path} references service dependency {} operationAbiId {} with operation ref that does not match dependency publicationAbi.operationExports",
                symbol.dependency_ref, operation_abi_id
            ),
        });
    }
    let Some(operation_abi) = dependency
        .publication_abi
        .operation_abi
        .iter()
        .find(|operation| operation.operation.operation_abi_id == operation_abi_id)
    else {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "{module_path} references service dependency {} source-call path {} resolving to operationAbiId {}, but dependency publicationAbi.operationAbi does not declare it",
                symbol.dependency_ref, source_call_path, operation_abi_id
            ),
        });
    };
    if operation_abi.operation != symbol.operation {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "{module_path} references service dependency {} operationAbiId {} with operation ref that does not match dependency publicationAbi.operationAbi",
                symbol.dependency_ref, operation_abi_id
            ),
        });
    }
    Ok(())
}

fn validate_service_dependency_alias(
    index: &ServiceDependencySymbolRefIndex<'_>,
    module_path: &str,
    dependency_ref: &str,
    source_call_path: &str,
) -> Result<(), ProjectionError> {
    if index.dependencies_by_alias.contains_key(dependency_ref) {
        return Ok(());
    }
    Err(ProjectionError::ContractValidation {
        message: format!(
            "{module_path} references undeclared service dependency alias {dependency_ref} for source-call path {source_call_path}"
        ),
    })
}
