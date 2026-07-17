use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{ContractOperationId, ServiceCallRef, ServiceRequirement};
use skiff_compiler_source::{ExpressionKey, ResolvedCallTarget, ResolvedCallTargetFacts};

use crate::{ContractDependencyOperationIndex, ServiceCallLoweringError};

/// Call-site association retained until canonical File IR materialization. It
/// deliberately carries ServiceCallRef rather than a legacy OperationAbiRef or
/// provider executable target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredServiceCallSite {
    expression: ExpressionKey,
    call_ref: ServiceCallRef,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoweredServiceCalls {
    service_requirements: Vec<ServiceRequirement>,
    call_sites: Vec<LoweredServiceCallSite>,
}

impl LoweredServiceCallSite {
    pub fn expression(&self) -> &ExpressionKey {
        &self.expression
    }

    pub fn call_ref(&self) -> &ServiceCallRef {
        &self.call_ref
    }
}

impl LoweredServiceCalls {
    pub fn service_requirements(&self) -> &[ServiceRequirement] {
        &self.service_requirements
    }

    pub fn call_sites(&self) -> &[LoweredServiceCallSite] {
        &self.call_sites
    }

    pub fn service_call_refs(&self) -> impl Iterator<Item = &ServiceCallRef> {
        self.call_sites.iter().map(LoweredServiceCallSite::call_ref)
    }
}

/// Lowers only typed contract call targets. Local and dependency package call
/// targets are borrowed and left untouched by construction.
pub fn lower_service_calls(
    targets: &ResolvedCallTargetFacts,
    operations: &ContractDependencyOperationIndex,
) -> Result<LoweredServiceCalls, ServiceCallLoweringError> {
    let mut used_operations = BTreeMap::<String, BTreeSet<ContractOperationId>>::new();
    for (_, target) in targets.iter() {
        let ResolvedCallTarget::ContractOperation {
            contract_requirement_alias,
            contract_operation_id,
            expected_protocol_identity,
        } = target
        else {
            continue;
        };
        operations.operation(
            contract_requirement_alias,
            contract_operation_id,
            expected_protocol_identity,
        )?;
        used_operations
            .entry(contract_requirement_alias.clone())
            .or_default()
            .insert(contract_operation_id.clone());
    }

    // Slots are dense over used requirements and ordered by validated alias.
    // Therefore declaration order, call-site order, operation order, duplicate
    // calls, and unused declarations cannot perturb the slot assignment.
    let mut slots = BTreeMap::new();
    let mut service_requirements = Vec::with_capacity(used_operations.len());
    for (slot_index, (alias, operation_ids)) in used_operations.into_iter().enumerate() {
        let slot = u32::try_from(slot_index)
            .map_err(|_| ServiceCallLoweringError::TooManyServiceRequirements)?;
        slots.insert(alias.clone(), slot);
        service_requirements.push(ServiceRequirement {
            contract_requirement: operations.requirement(&alias)?.clone(),
            service_binding_slot: slot,
            used_operations: operation_ids,
        });
    }

    let mut call_sites = Vec::new();
    for (expression, target) in targets.iter() {
        let ResolvedCallTarget::ContractOperation {
            contract_requirement_alias,
            contract_operation_id,
            expected_protocol_identity,
        } = target
        else {
            continue;
        };
        let slot = slots[contract_requirement_alias];
        call_sites.push(LoweredServiceCallSite {
            expression: expression.clone(),
            call_ref: ServiceCallRef {
                service_requirement_slot: slot,
                contract_operation_id: contract_operation_id.clone(),
                expected_protocol_identity: expected_protocol_identity.clone(),
            },
        });
    }

    Ok(LoweredServiceCalls {
        service_requirements,
        call_sites,
    })
}

#[cfg(test)]
mod tests;
