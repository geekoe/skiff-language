mod eligibility;
pub(super) mod ordering;
mod requirements;
mod types;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, CallableEffectSummary,
    CallableSemanticFacts, PackageCallableSignature, PackageRuntimeRequirements,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;
use std::collections::BTreeMap;

use crate::error::ProjectionError;

use eligibility::push_reason;

pub fn project_boundary_callable(
    owner_module: &str,
    signature: &PackageCallableSignature,
    facts: &CallableSemanticFacts,
    runtime_requirements: &PackageRuntimeRequirements,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), skiff_artifact_model::ContractTypeRef>,
) -> Result<BoundaryCallableProjection, ProjectionError> {
    project_boundary_callable_with_package_schemas(
        owner_module,
        signature,
        facts,
        runtime_requirements,
        file_ir_units,
        public_type_ids,
        &[],
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_boundary_callable_with_package_schemas(
    owner_module: &str,
    signature: &PackageCallableSignature,
    facts: &CallableSemanticFacts,
    runtime_requirements: &PackageRuntimeRequirements,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
    public_type_ids: &BTreeMap<(String, String), skiff_artifact_model::ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
    executable_index: Option<u32>,
) -> Result<BoundaryCallableProjection, ProjectionError> {
    let mut reasons = Vec::new();
    if let Some(inout_indices) = executable_index.and_then(|index| {
        file_ir_units
            .iter()
            .find(|unit| unit.module_path == owner_module)
            .and_then(|unit| unit.executables.get(index as usize))
            .map(|executable| {
                executable
                    .params
                    .iter()
                    .enumerate()
                    .filter(|(_, parameter)| {
                        parameter.mode == skiff_artifact_model::ParamModeIr::InOut
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>()
            })
    }) {
        if !inout_indices.is_empty() {
            // An inout loan cannot cross the service boundary; the callable
            // stays usable only through exact Package-direct calls.
            push_reason(
                &mut reasons,
                BoundaryUnavailableReason::InOutNotAllowedAtServiceBoundary,
            );
        }
    }
    let operation_contract = types::project_operation_contract(
        owner_module,
        signature,
        file_ir_units,
        public_type_ids,
        resolved_package_schemas,
        &mut reasons,
    );
    reasons.extend(eligibility::semantic_unavailable_reasons(
        facts,
        operation_contract.as_ref(),
    ));
    eligibility::normalize_reasons(&mut reasons);
    if !reasons.is_empty() {
        return Ok(BoundaryCallableProjection::Unavailable { reasons });
    }

    let effects = match &facts.effects {
        CallableEffectSummary::Analyzed { effects } => effects.clone(),
        CallableEffectSummary::Unknown { .. } => {
            return Err(ProjectionError::InvalidPackageArtifact {
                message: format!(
                    "boundary callable in module {owner_module} reached Available with unknown effect facts"
                ),
            });
        }
    };
    let Some(operation_contract) = operation_contract else {
        return Err(ProjectionError::InvalidPackageArtifact {
            message: format!(
                "boundary callable in module {owner_module} reached Available without a complete operation contract"
            ),
        });
    };
    Ok(BoundaryCallableProjection::Available {
        operation_contract,
        implementation_requirements: requirements::implementation_requirements(
            runtime_requirements,
            effects,
            facts.provenance.clone(),
        ),
    })
}
