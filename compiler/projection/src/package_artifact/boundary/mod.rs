mod eligibility;
pub(super) mod ordering;
mod requirements;
mod types;

use skiff_artifact_model::{
    BoundaryCallableProjection, CallableEffectSummary, CallableSemanticFacts,
    PackageCallableSignature, PackageRuntimeRequirements,
};

use crate::error::ProjectionError;

pub fn project_boundary_callable(
    owner_module: &str,
    signature: &PackageCallableSignature,
    facts: &CallableSemanticFacts,
    runtime_requirements: &PackageRuntimeRequirements,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
) -> Result<BoundaryCallableProjection, ProjectionError> {
    let mut reasons = eligibility::semantic_unavailable_reasons(facts);
    let operation_contract =
        types::project_operation_contract(owner_module, signature, file_ir_units, &mut reasons);
    eligibility::normalize_reasons(&mut reasons);
    if !reasons.is_empty() {
        return Ok(BoundaryCallableProjection::Unavailable { reasons });
    }

    let effects = match &facts.effects {
        CallableEffectSummary::Analyzed { effects } => *effects,
        CallableEffectSummary::Unknown { .. } => {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "boundary callable in module {owner_module} reached Available with unknown effect facts"
                ),
            });
        }
    };
    let Some(operation_contract) = operation_contract else {
        return Err(ProjectionError::ContractValidation {
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
