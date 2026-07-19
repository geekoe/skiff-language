use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryErrorContract,
    BoundaryOperationDescriptor, BoundaryStreamContract, PackageTypeRef, ServiceContract,
};

use crate::{
    contract_type_resolution::package_type_ref_from_validated_contract_ref,
    shared::ast_utils::dependency_source_address_parts, ResolvedTypeRef,
    SourceDependencyAnalysisInput, TypeResolutionContext, TypeResolutionModel,
};

mod projected_environment;
mod type_projection;

pub(super) use projected_environment::ContractProjectionState;
pub(super) use type_projection::{
    contract_source_assignability, contract_source_assignability_with_projections,
};
use type_projection::{
    package_type_assignable, resolved_contract_type, ContractCallTypeProjection,
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

struct ResolvedContractCall<'a> {
    alias: String,
    contract: &'a ServiceContract,
    operation: &'a BoundaryOperationDescriptor,
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
        let Some(call) = self.lookup_call(path) else {
            return ContractCallOutcome::NotContract;
        };
        let mut diagnostics = operation_shape_diagnostics(path, &call.operation.contract);
        self.check_call_shape(
            path,
            type_arg_count,
            arg_types.len(),
            &call,
            &mut diagnostics,
        );
        self.check_arguments(path, arg_types, projected_types, &call, &mut diagnostics);
        let Some((return_type, projected_return_type)) =
            self.resolve_return(path, &call, &mut diagnostics)
        else {
            return ContractCallOutcome::Invalid(diagnostics);
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

    fn lookup_call(&self, path: &str) -> Option<ResolvedContractCall<'_>> {
        let (alias, stable_key) = dependency_source_address_parts(path)?;
        let contract = self.dependency_analysis.contract(alias).ok()?;
        let operation = self
            .dependency_analysis
            .contract_operation_by_stable_key(alias, stable_key)
            .ok()?;
        Some(ResolvedContractCall {
            alias: alias.to_string(),
            contract,
            operation,
        })
    }

    fn check_call_shape(
        &self,
        path: &str,
        type_arg_count: usize,
        arg_count: usize,
        call: &ResolvedContractCall<'_>,
        diagnostics: &mut Vec<String>,
    ) {
        if type_arg_count != 0 {
            diagnostics.push(format!(
                "contract call `{path}` does not accept source type arguments"
            ));
        }
        if call.operation.contract.parameters.len() != arg_count {
            diagnostics.push(format!(
                "contract call `{path}` arity mismatch: expected {} arguments, found {arg_count}",
                call.operation.contract.parameters.len()
            ));
        }
    }

    fn check_arguments(
        &self,
        path: &str,
        arg_types: &[(super::ExpressionKey, Option<ResolvedTypeRef>)],
        projected_types: &std::collections::BTreeMap<super::ExpressionKey, PackageTypeRef>,
        call: &ResolvedContractCall<'_>,
        diagnostics: &mut Vec<String>,
    ) {
        for (index, (parameter, (argument_key, actual))) in call
            .operation
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
            let actual_projected = match projected_types.get(argument_key).cloned() {
                Some(projected) => projected,
                None => match self.type_projection.try_resolved_package_type(actual) {
                    Ok(projected) => projected,
                    Err(error) => {
                        diagnostics.push(format!(
                            "contract call `{path}` argument {} exact source type projection failed: {error}",
                            index + 1
                        ));
                        continue;
                    }
                },
            };
            if !package_type_assignable(&actual_projected, &expected) {
                let expected_label =
                    resolved_contract_type(&parameter.ty, &call.alias, call.contract)
                        .map(|ty| ty.source_text)
                        .unwrap_or_else(|_| format!("{:?}", parameter.ty));
                diagnostics.push(format!(
                    "contract call `{path}` argument {} type mismatch: expected {expected_label}, found {}",
                    index + 1,
                    actual.source_text
                ));
            }
        }
    }

    fn resolve_return(
        &self,
        path: &str,
        call: &ResolvedContractCall<'_>,
        diagnostics: &mut Vec<String>,
    ) -> Option<(ResolvedTypeRef, PackageTypeRef)> {
        let projected = match package_type_ref_from_validated_contract_ref(
            &call.operation.contract.return_value.ty,
        ) {
            Ok(projected) => projected,
            Err(error) => {
                diagnostics.push(format!(
                    "contract call `{path}` return uses unsupported inline contract shape: {error}"
                ));
                return None;
            }
        };
        match resolved_contract_type(
            &call.operation.contract.return_value.ty,
            &call.alias,
            call.contract,
        ) {
            Ok(resolved) => Some((resolved, projected)),
            Err(error) => {
                diagnostics.push(error);
                None
            }
        }
    }
}

pub(super) fn project_resolved_package_type(
    ty: &ResolvedTypeRef,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
    type_context: &TypeResolutionContext<'_>,
) -> Result<PackageTypeRef, String> {
    ContractCallTypeProjection::new(type_resolution, dependency_analysis, type_context)
        .try_resolved_package_type(ty)
}

pub(super) fn project_source_package_type_ref(
    ty: &crate::shared::ast::TypeRef,
    type_resolution: &TypeResolutionModel,
    dependency_analysis: &SourceDependencyAnalysisInput,
    type_context: &TypeResolutionContext<'_>,
) -> Result<PackageTypeRef, String> {
    ContractCallTypeProjection::new(type_resolution, dependency_analysis, type_context)
        .try_source_package_type_ref(ty)
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
