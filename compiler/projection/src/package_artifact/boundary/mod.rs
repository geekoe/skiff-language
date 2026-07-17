mod eligibility;
mod requirements;
mod types;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryOperationDescriptor, CallableEffectSummary,
    CallableSemanticFacts, ContractOperationId, PackageCallableId, PackageCallableSignature,
    PackageRuntimeRequirements,
};

use crate::error::ProjectionError;

pub fn project_boundary_callable(
    callable_id: &PackageCallableId,
    stable_key: &str,
    owner_module: &str,
    signature: &PackageCallableSignature,
    facts: &CallableSemanticFacts,
    runtime_requirements: &PackageRuntimeRequirements,
    file_ir_units: &[skiff_artifact_model::FileIrUnit],
) -> Result<BoundaryCallableProjection, ProjectionError> {
    let mut reasons = eligibility::semantic_unavailable_reasons(facts);
    let descriptor_contract =
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
                    "boundary callable {stable_key} reached Available with unknown effect facts"
                ),
            });
        }
    };
    let Some(contract) = descriptor_contract else {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "boundary callable {stable_key} reached Available without a complete descriptor"
            ),
        });
    };
    Ok(BoundaryCallableProjection::Available {
        descriptor: BoundaryOperationDescriptor {
            // A package callable is not bound to a service operation yet. Its
            // stable package callable identity occupies the descriptor leaf;
            // deployment later compares the contract body under its explicit
            // operation mapping.
            operation_id: ContractOperationId::new(callable_id.as_str()),
            stable_key: stable_key.to_string(),
            contract,
        },
        implementation_requirements: requirements::implementation_requirements(
            runtime_requirements,
            effects,
            facts.provenance.clone(),
        ),
    })
}
