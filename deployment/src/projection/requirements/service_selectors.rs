use std::collections::BTreeMap;

use skiff_artifact_model::{ServiceContractRef, ServiceDeploymentInput, ServiceRequirementKey};

use super::{PackageClosure, ProjectionError, ProjectionResult};

pub(super) fn validate(
    input: &ServiceDeploymentInput,
    closure: &PackageClosure<'_>,
) -> ProjectionResult<()> {
    let mut expected = BTreeMap::new();
    for artifact in closure.artifacts() {
        for requirement in &artifact.service_requirements {
            let key = ServiceRequirementKey {
                caller_package_build_id: artifact.package_build_id.clone(),
                service_requirement_slot: requirement.service_binding_slot,
            };
            let contract = ServiceContractRef {
                service_id: requirement.contract_requirement.service_id.clone(),
                contract_version: requirement.contract_requirement.contract_version.clone(),
                service_protocol_identity: requirement
                    .contract_requirement
                    .expected_protocol_identity
                    .clone(),
            };
            if expected.insert(key.clone(), contract).is_some() {
                return Err(ProjectionError::ConflictingRequirement {
                    kind: "service selector",
                    key: service_key(&key),
                    message: "typed package repeats a service requirement slot".to_string(),
                });
            }
        }
    }

    let mut actual = BTreeMap::new();
    for binding in &input.service_selectors {
        if actual
            .insert(binding.key.clone(), &binding.contract)
            .is_some()
        {
            return Err(ProjectionError::ConflictingRequirement {
                kind: "service selector",
                key: service_key(&binding.key),
                message: "binding key is repeated".to_string(),
            });
        }
    }
    for (key, contract) in &expected {
        let actual_contract =
            actual
                .get(key)
                .copied()
                .ok_or_else(|| ProjectionError::MissingRequirementBinding {
                    kind: "service selector",
                    key: service_key(key),
                })?;
        if actual_contract != contract {
            return Err(ProjectionError::RequirementBindingMismatch {
                kind: "service selector",
                key: service_key(key),
                message: format!("expected {contract:?}, got {actual_contract:?}"),
            });
        }
    }
    for key in actual.keys() {
        if !expected.contains_key(key) {
            return Err(ProjectionError::ExtraRequirementBinding {
                kind: "service selector",
                key: service_key(key),
            });
        }
    }
    Ok(())
}

fn service_key(key: &ServiceRequirementKey) -> String {
    format!(
        "{}:{}",
        key.caller_package_build_id, key.service_requirement_slot
    )
}
