use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{PackageResourceRequirement, ServiceDeploymentInput, StateBindingKind};

use super::{ProjectionError, ProjectionResult};

pub(super) fn validate_state_bindings(
    input: &ServiceDeploymentInput,
    expected: &BTreeMap<String, StateBindingKind>,
) -> ProjectionResult<()> {
    let actual = input
        .state_bindings
        .iter()
        .map(|binding| (binding.requirement_key.as_str(), binding.kind))
        .collect::<BTreeMap<_, _>>();
    for (key, kind) in expected {
        match actual.get(key.as_str()) {
            None => {
                return Err(ProjectionError::MissingRequirementBinding {
                    kind: "state",
                    key: key.clone(),
                });
            }
            Some(actual_kind) if actual_kind != kind => {
                return Err(ProjectionError::RequirementBindingMismatch {
                    kind: "state",
                    key: key.clone(),
                    message: format!("expected {kind:?}, got {actual_kind:?}"),
                });
            }
            Some(_) => {}
        }
    }
    for key in actual.keys() {
        if !expected.contains_key(*key) {
            return Err(ProjectionError::ExtraRequirementBinding {
                kind: "state",
                key: (*key).to_string(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_resource_bindings(
    input: &ServiceDeploymentInput,
    expected: &BTreeMap<String, PackageResourceRequirement>,
    native_capabilities: &BTreeSet<String>,
) -> ProjectionResult<()> {
    let actual = input
        .resource_bindings
        .iter()
        .map(|binding| (binding.requirement_key.as_str(), binding))
        .collect::<BTreeMap<_, _>>();
    for (key, requirement) in expected {
        let binding = actual.get(key.as_str()).copied().ok_or_else(|| {
            ProjectionError::MissingRequirementBinding {
                kind: "resource",
                key: key.clone(),
            }
        })?;
        if binding.capability != requirement.capability {
            return Err(ProjectionError::RequirementBindingMismatch {
                kind: "resource",
                key: key.clone(),
                message: format!(
                    "expected capability {}, got {}",
                    requirement.capability, binding.capability
                ),
            });
        }
    }
    for capability in native_capabilities {
        let count = input
            .resource_bindings
            .iter()
            .filter(|binding| &binding.capability == capability)
            .count();
        if count == 0 {
            return Err(ProjectionError::MissingRequirementBinding {
                kind: "native capability",
                key: capability.clone(),
            });
        }
        if count > 1 {
            return Err(ProjectionError::ConflictingRequirement {
                kind: "native capability",
                key: capability.clone(),
                message: "more than one resource binding provides the capability".to_string(),
            });
        }
    }
    for binding in &input.resource_bindings {
        if !expected.contains_key(&binding.requirement_key)
            && !native_capabilities.contains(&binding.capability)
        {
            return Err(ProjectionError::ExtraRequirementBinding {
                kind: "resource",
                key: binding.requirement_key.clone(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_runtime_capability_bindings(
    input: &ServiceDeploymentInput,
    expected: &BTreeMap<String, String>,
) -> ProjectionResult<()> {
    let actual = input
        .runtime_capability_bindings
        .iter()
        .map(|binding| (binding.capability.as_str(), binding.version.as_str()))
        .collect::<BTreeMap<_, _>>();
    for (capability, version) in expected {
        match actual.get(capability.as_str()) {
            None => {
                return Err(ProjectionError::MissingRequirementBinding {
                    kind: "runtime capability",
                    key: capability.clone(),
                });
            }
            Some(actual_version) if *actual_version != version => {
                return Err(ProjectionError::RequirementBindingMismatch {
                    kind: "runtime capability",
                    key: capability.clone(),
                    message: format!("expected version {version}, got {actual_version}"),
                });
            }
            Some(_) => {}
        }
    }
    for capability in actual.keys() {
        if !expected.contains_key(*capability) {
            return Err(ProjectionError::ExtraRequirementBinding {
                kind: "runtime capability",
                key: (*capability).to_string(),
            });
        }
    }
    Ok(())
}
