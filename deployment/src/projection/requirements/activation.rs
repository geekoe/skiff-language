use std::collections::BTreeMap;

use skiff_artifact_model::{ServiceDeploymentInput, StateBindingKind};

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
