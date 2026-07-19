use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryErrorContract,
    BoundaryStreamContract, PackageTypeRef,
};

use crate::{
    contract_type_resolution::package_type_ref_from_validated_contract_ref, ResolvedTypeRef,
    SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
};

mod type_projection;

pub(super) use type_projection::contract_source_assignability;
use type_projection::{
    package_type_assignable, package_type_contains_contract, resolved_contract_type,
    ContractCallTypeProjection,
};

pub(super) enum ContractCallOutcome {
    NotContract,
    Typed {
        return_type: ResolvedTypeRef,
        projected_return_type: PackageTypeRef,
    },
    Invalid(Vec<String>),
}

pub(super) struct ContractCallTyping<'a, 'ctx> {
    dependency_analysis: &'a SourceDependencyAnalysisInput,
    type_projection: ContractCallTypeProjection<'a, 'ctx>,
}

impl<'a, 'ctx> ContractCallTyping<'a, 'ctx> {
    pub(super) fn new(
        type_resolution: &'a TypeResolutionModel,
        dependency_analysis: &'a SourceDependencyAnalysisInput,
        type_context: &'a TypeResolutionContext<'ctx>,
    ) -> Self {
        Self {
            dependency_analysis,
            type_projection: ContractCallTypeProjection::new(
                type_resolution,
                dependency_analysis,
                type_context,
            ),
        }
    }

    pub(super) fn check_call(
        &self,
        path: &str,
        type_arg_count: usize,
        arg_types: &[(super::ExpressionKey, Option<ResolvedTypeRef>)],
        projected_types: &std::collections::BTreeMap<super::ExpressionKey, PackageTypeRef>,
    ) -> ContractCallOutcome {
        let (alias, stable_key) = match path.split_once('.') {
            Some(parts) => parts,
            // T03A owns unknown-member diagnostics and their exact source
            // owner/location. Leave unresolved members for that pass.
            None => return ContractCallOutcome::NotContract,
        };
        let Ok(contract) = self.dependency_analysis.contract(alias) else {
            return ContractCallOutcome::NotContract;
        };
        let operation = match self
            .dependency_analysis
            .contract_operation_by_stable_key(alias, stable_key)
        {
            Ok(operation) => operation,
            // T03A owns unknown-member diagnostics and rejects the call before
            // any real contract-operation target can be published.
            Err(_) => return ContractCallOutcome::NotContract,
        };

        let mut diagnostics = operation_shape_diagnostics(path, &operation.contract);
        if type_arg_count != 0 {
            diagnostics.push(format!(
                "contract call `{path}` does not accept source type arguments"
            ));
        }
        if operation.contract.parameters.len() != arg_types.len() {
            diagnostics.push(format!(
                "contract call `{path}` arity mismatch: expected {} arguments, found {}",
                operation.contract.parameters.len(),
                arg_types.len()
            ));
        }

        for (index, (parameter, (argument_key, actual))) in operation
            .contract
            .parameters
            .iter()
            .zip(arg_types)
            .enumerate()
        {
            let expected = match package_type_ref_from_validated_contract_ref(&parameter.ty) {
                Ok(expected) => expected,
                Err(error) => {
                    diagnostics.push(format!(
                        "contract call `{path}` argument {} uses unsupported inline contract shape: {error}",
                        index + 1
                    ));
                    continue;
                }
            };
            let Some(actual) = actual else {
                diagnostics.push(format!(
                    "contract call `{path}` argument {} has no resolved source type",
                    index + 1
                ));
                continue;
            };
            let actual_projected = projected_types
                .get(argument_key)
                .cloned()
                .unwrap_or_else(|| self.type_projection.source_package_type(actual));
            if !package_type_assignable(&actual_projected, &expected) {
                let expected_label = resolved_contract_type(&parameter.ty, alias, contract)
                    .map(|ty| ty.source_text)
                    .unwrap_or_else(|_| format!("{:?}", parameter.ty));
                diagnostics.push(format!(
                    "contract call `{path}` argument {} type mismatch: expected {expected_label}, found {}",
                    index + 1,
                    actual.source_text
                ));
            }
        }

        let projected_return_type =
            match package_type_ref_from_validated_contract_ref(&operation.contract.return_value.ty)
            {
                Ok(projected_return_type) => projected_return_type,
                Err(error) => {
                    diagnostics.push(format!(
                    "contract call `{path}` return uses unsupported inline contract shape: {error}"
                ));
                    return ContractCallOutcome::Invalid(diagnostics);
                }
            };
        let return_type =
            match resolved_contract_type(&operation.contract.return_value.ty, alias, contract) {
                Ok(return_type) => return_type,
                Err(error) => {
                    diagnostics.push(error);
                    return ContractCallOutcome::Invalid(diagnostics);
                }
            };
        if diagnostics.is_empty() {
            ContractCallOutcome::Typed {
                return_type,
                projected_return_type,
            }
        } else {
            ContractCallOutcome::Invalid(diagnostics)
        }
    }
}

pub(super) fn project_source_package_type(
    ty: &ResolvedTypeRef,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
    type_context: &TypeResolutionContext<'_>,
) -> PackageTypeRef {
    ContractCallTypeProjection::new(type_resolution, dependency_analysis, type_context)
        .source_package_type(ty)
}

pub(super) fn projected_type_contains_contract(ty: &PackageTypeRef) -> bool {
    package_type_contains_contract(ty)
}

fn operation_shape_diagnostics(
    path: &str,
    operation: &skiff_artifact_model::BoundaryOperationContract,
) -> Vec<String> {
    let mut diagnostics = Vec::new();
    if !matches!(operation.errors, BoundaryErrorContract::None) {
        diagnostics.push(format!(
            "contract call `{path}` uses an error contract unsupported by source calls"
        ));
    }
    if !matches!(operation.stream, BoundaryStreamContract::Unary) {
        diagnostics.push(format!(
            "contract call `{path}` uses a stream contract unsupported by unary source calls"
        ));
    }
    if !matches!(operation.callbacks, BoundaryCallbackContract::None) {
        diagnostics.push(format!(
            "contract call `{path}` uses a callback contract unsupported by source calls"
        ));
    }
    if matches!(
        operation.cancellation,
        BoundaryCancellationContract::Unsupported { .. }
    ) {
        diagnostics.push(format!(
            "contract call `{path}` uses unsupported cancellation semantics"
        ));
    }
    diagnostics
}

#[cfg(test)]
mod tests;
