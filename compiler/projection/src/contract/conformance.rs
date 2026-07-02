use skiff_artifact_model::{ExecutableIr, ExecutableKind, ParamIr, TypeRefIr};
use skiff_compiler_projection_input::ProjectionView;

use super::{
    ContractInterfaceOperationProjection, ContractOperationBindingProjection, ContractProjection,
    ContractProjectionIndex, ContractTypeCanonicalizationError, ContractTypeKey,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractConformanceError {
    violations: Vec<ContractConformanceViolation>,
}

impl ContractConformanceError {
    pub fn violations(&self) -> &[ContractConformanceViolation] {
        &self.violations
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractConformanceViolation {
    MissingInterfaceProjection {
        interface_public_name: String,
    },
    MissingOperationBinding {
        interface_public_name: String,
        operation_name: String,
    },
    MissingExecutable {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        executable_symbol: String,
    },
    UnexpectedExecutableKind {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        executable_symbol: String,
        expected: ExecutableKind,
        found: ExecutableKind,
    },
    MissingImplementationType {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        type_name: String,
    },
    ReturnTypeMismatch {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        executable_symbol: String,
        expected: ContractTypeKey,
        found: ContractTypeKey,
    },
    ParameterCountMismatch {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        executable_symbol: String,
        expected: usize,
        found: usize,
    },
    ParameterNameMismatch {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        executable_symbol: String,
        parameter_index: usize,
        expected: String,
        found: String,
    },
    ParameterTypeMismatch {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        executable_symbol: String,
        parameter_index: usize,
        parameter_name: String,
        expected: ContractTypeKey,
        found: ContractTypeKey,
    },
    TypeCanonicalization {
        interface_public_name: String,
        operation_name: String,
        module_path: String,
        executable_symbol: String,
        context: ContractConformanceTypeContext,
        source: ContractTypeCanonicalizationError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractConformanceTypeContext {
    ImplementationType,
    ReturnType,
    Parameter {
        parameter_index: usize,
        parameter_name: String,
    },
    ExplicitReceiver,
}

pub fn validate_contract_projection_conformance(
    input: ProjectionView<'_>,
    projection: &ContractProjection,
) -> Result<(), ContractConformanceError> {
    let index = ContractProjectionIndex::from_projection_input_with_prelude(
        input,
        Some(projection.prelude()),
    );
    let mut violations = Vec::new();

    for (interface_public_name, api_binding) in &projection.api_bindings {
        let Some(interface) = projection.interfaces.get(interface_public_name) else {
            violations.push(ContractConformanceViolation::MissingInterfaceProjection {
                interface_public_name: interface_public_name.clone(),
            });
            continue;
        };

        for operation in &interface.operations {
            let Some(operation_binding) = api_binding.operations.get(&operation.name) else {
                violations.push(ContractConformanceViolation::MissingOperationBinding {
                    interface_public_name: interface_public_name.clone(),
                    operation_name: operation.name.clone(),
                });
                continue;
            };

            validate_bound_operation(
                &index,
                interface_public_name,
                operation,
                operation_binding,
                &mut violations,
            );
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(ContractConformanceError { violations })
    }
}

fn validate_bound_operation(
    index: &ContractProjectionIndex<'_>,
    interface_public_name: &str,
    operation: &ContractInterfaceOperationProjection,
    operation_binding: &ContractOperationBindingProjection,
    violations: &mut Vec<ContractConformanceViolation>,
) {
    let Some(executable) = index.executable_by_module_symbol(
        &operation_binding.module_path,
        &operation_binding.executable_symbol,
    ) else {
        violations.push(ContractConformanceViolation::MissingExecutable {
            interface_public_name: interface_public_name.to_string(),
            operation_name: operation.name.clone(),
            module_path: operation_binding.module_path.clone(),
            executable_symbol: operation_binding.executable_symbol.clone(),
        });
        return;
    };

    if executable.kind != ExecutableKind::ImplMethod {
        violations.push(ContractConformanceViolation::UnexpectedExecutableKind {
            interface_public_name: interface_public_name.to_string(),
            operation_name: operation.name.clone(),
            module_path: operation_binding.module_path.clone(),
            executable_symbol: operation_binding.executable_symbol.clone(),
            expected: ExecutableKind::ImplMethod,
            found: executable.kind,
        });
        return;
    }

    validate_return_type(
        index,
        interface_public_name,
        operation,
        operation_binding,
        executable,
        violations,
    );
    validate_params(
        index,
        interface_public_name,
        operation,
        operation_binding,
        executable,
        violations,
    );
}

fn validate_return_type(
    index: &ContractProjectionIndex<'_>,
    interface_public_name: &str,
    operation: &ContractInterfaceOperationProjection,
    operation_binding: &ContractOperationBindingProjection,
    executable: &ExecutableIr,
    violations: &mut Vec<ContractConformanceViolation>,
) {
    let Some(found) = canonical_type_or_violation(
        index,
        interface_public_name,
        operation,
        operation_binding,
        &executable.return_type,
        ContractConformanceTypeContext::ReturnType,
        violations,
    ) else {
        return;
    };

    if found != operation.return_type {
        violations.push(ContractConformanceViolation::ReturnTypeMismatch {
            interface_public_name: interface_public_name.to_string(),
            operation_name: operation.name.clone(),
            module_path: operation_binding.module_path.clone(),
            executable_symbol: operation_binding.executable_symbol.clone(),
            expected: operation.return_type.clone(),
            found,
        });
    }
}

fn validate_params(
    index: &ContractProjectionIndex<'_>,
    interface_public_name: &str,
    operation: &ContractInterfaceOperationProjection,
    operation_binding: &ContractOperationBindingProjection,
    executable: &ExecutableIr,
    violations: &mut Vec<ContractConformanceViolation>,
) {
    let Some(found_params) = executable_params_without_receiver(
        index,
        interface_public_name,
        operation,
        operation_binding,
        executable,
        violations,
    ) else {
        return;
    };

    if found_params.len() != operation.params.len() {
        violations.push(ContractConformanceViolation::ParameterCountMismatch {
            interface_public_name: interface_public_name.to_string(),
            operation_name: operation.name.clone(),
            module_path: operation_binding.module_path.clone(),
            executable_symbol: operation_binding.executable_symbol.clone(),
            expected: operation.params.len(),
            found: found_params.len(),
        });
        return;
    }

    for (index_in_params, (expected, found)) in
        operation.params.iter().zip(found_params.iter()).enumerate()
    {
        let parameter_index = index_in_params + 1;
        if expected.name != found.name {
            violations.push(ContractConformanceViolation::ParameterNameMismatch {
                interface_public_name: interface_public_name.to_string(),
                operation_name: operation.name.clone(),
                module_path: operation_binding.module_path.clone(),
                executable_symbol: operation_binding.executable_symbol.clone(),
                parameter_index,
                expected: expected.name.clone(),
                found: found.name.clone(),
            });
        }

        let Some(found_ty) = canonical_type_or_violation(
            index,
            interface_public_name,
            operation,
            operation_binding,
            &found.ty,
            ContractConformanceTypeContext::Parameter {
                parameter_index,
                parameter_name: found.name.clone(),
            },
            violations,
        ) else {
            continue;
        };
        if found_ty != expected.ty {
            violations.push(ContractConformanceViolation::ParameterTypeMismatch {
                interface_public_name: interface_public_name.to_string(),
                operation_name: operation.name.clone(),
                module_path: operation_binding.module_path.clone(),
                executable_symbol: operation_binding.executable_symbol.clone(),
                parameter_index,
                parameter_name: expected.name.clone(),
                expected: expected.ty.clone(),
                found: found_ty,
            });
        }
    }
}

fn executable_params_without_receiver<'a>(
    index: &ContractProjectionIndex<'_>,
    interface_public_name: &str,
    operation: &ContractInterfaceOperationProjection,
    operation_binding: &ContractOperationBindingProjection,
    executable: &'a ExecutableIr,
    violations: &mut Vec<ContractConformanceViolation>,
) -> Option<&'a [ParamIr]> {
    let Some(first_param) = executable.params.first() else {
        return Some(&executable.params);
    };
    if first_param.name != "self" {
        return Some(&executable.params);
    }

    let implementation_type = match implementation_type_key(index, operation_binding) {
        Ok(Some(key)) => key,
        Ok(None) => {
            violations.push(ContractConformanceViolation::MissingImplementationType {
                interface_public_name: interface_public_name.to_string(),
                operation_name: operation.name.clone(),
                module_path: operation_binding.module_path.clone(),
                type_name: operation_binding.type_name.clone(),
            });
            return None;
        }
        Err(source) => {
            violations.push(ContractConformanceViolation::TypeCanonicalization {
                interface_public_name: interface_public_name.to_string(),
                operation_name: operation.name.clone(),
                module_path: operation_binding.module_path.clone(),
                executable_symbol: operation_binding.executable_symbol.clone(),
                context: ContractConformanceTypeContext::ImplementationType,
                source,
            });
            return None;
        }
    };
    let Some(receiver_type) = canonical_type_or_violation(
        index,
        interface_public_name,
        operation,
        operation_binding,
        &first_param.ty,
        ContractConformanceTypeContext::ExplicitReceiver,
        violations,
    ) else {
        return None;
    };

    if receiver_type == implementation_type {
        Some(&executable.params[1..])
    } else {
        Some(&executable.params)
    }
}

fn implementation_type_key(
    index: &ContractProjectionIndex<'_>,
    operation_binding: &ContractOperationBindingProjection,
) -> Result<Option<ContractTypeKey>, ContractTypeCanonicalizationError> {
    let Some(unit) = index.unit_by_module_path(&operation_binding.module_path) else {
        return Ok(None);
    };
    let Some(declaration) = unit.declarations.types.get(&operation_binding.type_name) else {
        return Ok(None);
    };
    index
        .canonical_type_ref(
            &operation_binding.module_path,
            &TypeRefIr::LocalType {
                type_index: declaration.type_index,
            },
        )
        .map(Some)
}

fn canonical_type_or_violation(
    index: &ContractProjectionIndex<'_>,
    interface_public_name: &str,
    operation: &ContractInterfaceOperationProjection,
    operation_binding: &ContractOperationBindingProjection,
    ty: &TypeRefIr,
    context: ContractConformanceTypeContext,
    violations: &mut Vec<ContractConformanceViolation>,
) -> Option<ContractTypeKey> {
    match index.canonical_type_ref(&operation_binding.module_path, ty) {
        Ok(key) => Some(key),
        Err(source) => {
            violations.push(ContractConformanceViolation::TypeCanonicalization {
                interface_public_name: interface_public_name.to_string(),
                operation_name: operation.name.clone(),
                module_path: operation_binding.module_path.clone(),
                executable_symbol: operation_binding.executable_symbol.clone(),
                context,
                source,
            });
            None
        }
    }
}
