use std::collections::BTreeSet;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryImplementationRequirements, CallableEffectSummary,
    DeploymentOperationBinding, PackageArtifact, PackageCallableId, PackageLocalAbiSymbol,
    ServiceContract, ServiceDeploymentInput,
};

use super::{ProjectionError, ProjectionResult};

pub(super) struct ProjectedOperations<'a> {
    pub(super) bindings: Vec<DeploymentOperationBinding>,
    pub(super) selected: Vec<SelectedCallable<'a>>,
}

pub(super) struct SelectedCallable<'a> {
    pub(super) callable_id: PackageCallableId,
    pub(super) requirements: &'a BoundaryImplementationRequirements,
}

pub(super) fn project_operation_bindings<'a>(
    input: &ServiceDeploymentInput,
    contract: &ServiceContract,
    implementation: &'a PackageArtifact,
) -> ProjectionResult<ProjectedOperations<'a>> {
    let mut mapped = BTreeSet::new();
    for binding in &input.operation_bindings {
        if !mapped.insert(binding.contract_operation_id.clone()) {
            return Err(ProjectionError::DuplicateOperationBinding {
                operation_id: binding.contract_operation_id.clone(),
            });
        }
        if !contract
            .operations
            .contains_key(&binding.contract_operation_id)
        {
            return Err(ProjectionError::UnknownOperationBinding {
                operation_id: binding.contract_operation_id.clone(),
            });
        }
    }
    for operation_id in contract.operations.keys() {
        if !mapped.contains(operation_id) {
            return Err(ProjectionError::MissingOperationBinding {
                operation_id: operation_id.clone(),
            });
        }
    }

    let mut projected = Vec::with_capacity(input.operation_bindings.len());
    let mut selected = Vec::with_capacity(input.operation_bindings.len());
    for binding in &input.operation_bindings {
        let symbol = implementation
            .package_local_abi
            .public_symbols
            .get(&binding.package_public_path)
            .ok_or_else(|| ProjectionError::UnknownPublicPath {
                public_path: binding.package_public_path.clone(),
            })?;
        let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
            return Err(ProjectionError::PublicPathNotCallable {
                public_path: binding.package_public_path.clone(),
            });
        };
        let boundary = implementation
            .boundary_projections
            .get(callable_id)
            .ok_or_else(|| ProjectionError::CallableFactsMismatch {
                callable_id: callable_id.clone(),
                message: "boundary projection is absent".to_string(),
            })?;
        let (operation_contract, implementation_requirements) = match boundary {
            BoundaryCallableProjection::Available {
                operation_contract,
                implementation_requirements,
            } => (operation_contract, implementation_requirements),
            BoundaryCallableProjection::Unavailable { reasons } => {
                return Err(ProjectionError::BoundaryUnavailable {
                    operation_id: binding.contract_operation_id.clone(),
                    callable_id: callable_id.clone(),
                    reasons: reasons.clone(),
                });
            }
        };
        let descriptor = &contract.operations[&binding.contract_operation_id];
        if operation_contract != &descriptor.contract {
            return Err(ProjectionError::OperationContractMismatch {
                operation_id: binding.contract_operation_id.clone(),
                callable_id: callable_id.clone(),
            });
        }
        validate_callable_facts(implementation, callable_id, implementation_requirements)?;

        projected.push(DeploymentOperationBinding {
            contract_operation_id: binding.contract_operation_id.clone(),
            package_callable_id: callable_id.clone(),
        });
        selected.push(SelectedCallable {
            callable_id: callable_id.clone(),
            requirements: implementation_requirements,
        });
    }
    projected.sort_by(|left, right| left.contract_operation_id.cmp(&right.contract_operation_id));
    Ok(ProjectedOperations {
        bindings: projected,
        selected,
    })
}

fn validate_callable_facts(
    implementation: &PackageArtifact,
    callable_id: &PackageCallableId,
    requirements: &BoundaryImplementationRequirements,
) -> ProjectionResult<()> {
    let facts = implementation
        .callable_semantic_facts
        .get(callable_id)
        .ok_or_else(|| ProjectionError::CallableFactsMismatch {
            callable_id: callable_id.clone(),
            message: "callable semantic facts are absent".to_string(),
        })?;
    match &facts.effects {
        CallableEffectSummary::Analyzed { effects }
            if effects == &requirements.complete_may_effects => {}
        CallableEffectSummary::Analyzed { .. } => {
            return Err(ProjectionError::CallableFactsMismatch {
                callable_id: callable_id.clone(),
                message: "complete may-effects differ".to_string(),
            });
        }
        CallableEffectSummary::Unknown { .. } => {
            return Err(ProjectionError::CallableFactsMismatch {
                callable_id: callable_id.clone(),
                message: "available projection has unknown effects".to_string(),
            });
        }
    }
    if facts.provenance != requirements.provenance {
        return Err(ProjectionError::CallableFactsMismatch {
            callable_id: callable_id.clone(),
            message: "provenance differs".to_string(),
        });
    }
    Ok(())
}
