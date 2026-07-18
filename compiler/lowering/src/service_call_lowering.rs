use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ContractOperationId, ServiceCallRef, ServiceCallRefIndex, ServiceRequirement,
};
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
    file_refs: BTreeMap<String, Vec<ServiceCallRef>>,
    call_ref_indices: BTreeMap<ExpressionKey, ServiceCallRefIndex>,
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

    /// Exact package-level union contributed by the owner-local File IR
    /// tables. T04/T05 can compare this typed closure with the PackageArtifact
    /// aggregate without inspecting instructions or rebuilding slot rules.
    pub fn service_call_ref_closure(&self) -> BTreeSet<ServiceCallRef> {
        self.file_refs
            .values()
            .flat_map(|refs| refs.iter().cloned())
            .collect()
    }

    pub fn file_service_call_refs(&self, module_path: &str) -> &[ServiceCallRef] {
        self.file_refs
            .get(module_path)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn service_call_ref_index(
        &self,
        expression: &ExpressionKey,
    ) -> Option<ServiceCallRefIndex> {
        self.call_ref_indices.get(expression).copied()
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
            contract_requirement,
            contract_operation_id,
        } = target
        else {
            continue;
        };
        operations.operation(
            &contract_requirement.alias,
            contract_operation_id,
            &contract_requirement.expected_protocol_identity,
        )?;
        used_operations
            .entry(contract_requirement.alias.clone())
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
            contract_requirement,
            contract_operation_id,
        } = target
        else {
            continue;
        };
        let slot = slots[&contract_requirement.alias];
        call_sites.push(LoweredServiceCallSite {
            expression: expression.clone(),
            call_ref: ServiceCallRef {
                service_requirement_slot: slot,
                contract_operation_id: contract_operation_id.clone(),
                expected_protocol_identity: contract_requirement.expected_protocol_identity.clone(),
            },
        });
    }

    let (file_refs, call_ref_indices) = index_file_service_call_refs(&call_sites)?;
    Ok(LoweredServiceCalls {
        service_requirements,
        call_sites,
        file_refs,
        call_ref_indices,
    })
}

fn index_file_service_call_refs(
    call_sites: &[LoweredServiceCallSite],
) -> Result<
    (
        BTreeMap<String, Vec<ServiceCallRef>>,
        BTreeMap<ExpressionKey, ServiceCallRefIndex>,
    ),
    ServiceCallLoweringError,
> {
    let refs_by_module = call_sites.iter().fold(
        BTreeMap::<String, BTreeSet<ServiceCallRef>>::new(),
        |mut by_module, site| {
            by_module
                .entry(site.expression.module_path().to_string())
                .or_default()
                .insert(site.call_ref.clone());
            by_module
        },
    );
    let mut file_refs = BTreeMap::new();
    let mut indices_by_ref = BTreeMap::new();
    for (module_path, refs) in refs_by_module {
        let refs = refs.into_iter().collect::<Vec<_>>();
        for (index, call_ref) in refs.iter().enumerate() {
            let index = ServiceCallRefIndex::try_from(index).map_err(|_| {
                ServiceCallLoweringError::TooManyFileServiceCallRefs {
                    module_path: module_path.clone(),
                }
            })?;
            indices_by_ref.insert((module_path.clone(), call_ref.clone()), index);
        }
        file_refs.insert(module_path, refs);
    }
    let call_ref_indices = call_sites
        .iter()
        .map(|site| {
            (
                site.expression.clone(),
                indices_by_ref[&(
                    site.expression.module_path().to_string(),
                    site.call_ref.clone(),
                )],
            )
        })
        .collect();
    Ok((file_refs, call_ref_indices))
}

#[cfg(test)]
mod tests;
