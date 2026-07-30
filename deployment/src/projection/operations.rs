use std::collections::BTreeSet;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryImplementationRequirements, CallableEffectSummary,
    DeploymentOperationBinding, OperationCallableKind, PackageArtifact, PackageCallableId,
    PackageLocalAbiSymbol, ServiceContract, ServiceDeploymentInput,
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
        let callable_id = &binding.package_callable_id;
        validate_public_callable(implementation, callable_id)?;
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
        let facts = implementation
            .callable_semantic_facts
            .get(callable_id)
            .ok_or_else(|| ProjectionError::CallableFactsMismatch {
                callable_id: callable_id.clone(),
                message: "callable semantic facts are absent".to_string(),
            })?;
        validate_callable_facts(callable_id, facts, implementation_requirements)?;

        projected.push(DeploymentOperationBinding {
            contract_operation_id: binding.contract_operation_id.clone(),
            package_callable_id: binding.package_callable_id.clone(),
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

fn validate_public_callable(
    implementation: &PackageArtifact,
    callable_id: &PackageCallableId,
) -> ProjectionResult<()> {
    let public_callable_exists = implementation
        .package_local_abi
        .public_symbols
        .values()
        .any(|symbol| {
            matches!(
                symbol,
                PackageLocalAbiSymbol::Callable {
                    callable_id: public_callable_id,
                    ..
                } if public_callable_id == callable_id
            )
        });
    if !public_callable_exists {
        let exists_outside_public_surface = implementation
            .package_local_abi
            .implementation_symbols
            .values()
            .any(|symbol| {
                matches!(
                    symbol,
                    PackageLocalAbiSymbol::Callable {
                        callable_id: implementation_callable_id,
                        ..
                    } if implementation_callable_id == callable_id
                )
            })
            || implementation.callable_links.contains_key(callable_id);
        return Err(if exists_outside_public_surface {
            ProjectionError::NonPublicPackageCallable {
                callable_id: callable_id.clone(),
            }
        } else {
            ProjectionError::UnknownPackageCallable {
                callable_id: callable_id.clone(),
            }
        });
    }

    let link = implementation
        .callable_links
        .get(callable_id)
        .ok_or_else(|| ProjectionError::CallableLinkMismatch {
            callable_id: callable_id.clone(),
            message: "callable link is absent".to_string(),
        })?;
    if link.callable_id != *callable_id {
        return Err(ProjectionError::CallableLinkMismatch {
            callable_id: callable_id.clone(),
            message: format!("nested callable id is {}", link.callable_id),
        });
    }
    if link.target.callable_abi_id != callable_id.as_str() {
        return Err(ProjectionError::CallableLinkMismatch {
            callable_id: callable_id.clone(),
            message: format!("target callable ABI id is {}", link.target.callable_abi_id),
        });
    }

    let is_public_instance_method = implementation
        .package_local_abi
        .public_symbols
        .values()
        .any(|symbol| {
            matches!(
                symbol,
                PackageLocalAbiSymbol::PublicInstance { methods, .. }
                    if methods.values().any(|method_id| method_id == callable_id)
            )
        });
    match (link.target.callable_kind, is_public_instance_method) {
        (OperationCallableKind::PublicFunction, false)
        | (OperationCallableKind::ImplMethod, true) => Ok(()),
        _ => Err(ProjectionError::CallableLinkMismatch {
            callable_id: callable_id.clone(),
            message: format!(
                "target kind {:?} disagrees with public-instance membership {is_public_instance_method}",
                link.target.callable_kind
            ),
        }),
    }
}

fn validate_callable_facts(
    callable_id: &PackageCallableId,
    facts: &skiff_artifact_model::CallableSemanticFacts,
    requirements: &BoundaryImplementationRequirements,
) -> ProjectionResult<()> {
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
