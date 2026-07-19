use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ConfigShapeValueType, MetadataValue, PackageConfigRequirement, ServiceDeploymentInput,
};

use super::{ProjectionError, ProjectionResult};

pub(super) fn validate_bindings(
    input: &ServiceDeploymentInput,
    expected: &BTreeMap<String, PackageConfigRequirement>,
) -> ProjectionResult<()> {
    let literal_by_path = input
        .config_literals
        .iter()
        .map(|binding| (binding.path.as_str(), &binding.value))
        .collect::<BTreeMap<_, _>>();
    let secret_paths = input
        .secret_refs
        .iter()
        .map(|binding| binding.path.as_str())
        .collect::<BTreeSet<_>>();
    for requirement in expected.values() {
        let bound = literal_by_path.contains_key(requirement.path.as_str())
            || secret_paths.contains(requirement.path.as_str());
        if requirement.required && !bound {
            return Err(ProjectionError::MissingRequirementBinding {
                kind: "config",
                key: requirement.path.clone(),
            });
        }
        if let Some(value) = literal_by_path.get(requirement.path.as_str()) {
            validate_literal_type(requirement, value)?;
        }
    }
    for path in literal_by_path
        .keys()
        .copied()
        .chain(secret_paths.iter().copied())
    {
        if !expected.contains_key(path) {
            return Err(ProjectionError::ExtraRequirementBinding {
                kind: "config",
                key: path.to_string(),
            });
        }
    }
    Ok(())
}

fn validate_literal_type(
    requirement: &PackageConfigRequirement,
    value: &MetadataValue,
) -> ProjectionResult<()> {
    let ty = ConfigShapeValueType::try_from(requirement.value_type.as_str()).map_err(|error| {
        ProjectionError::ConflictingRequirement {
            kind: "config",
            key: requirement.path.clone(),
            message: error.to_string(),
        }
    })?;
    let valid = match ty {
        ConfigShapeValueType::String => matches!(value, MetadataValue::String(_)),
        ConfigShapeValueType::Number => matches!(value, MetadataValue::Number(_)),
        ConfigShapeValueType::Bool => matches!(value, MetadataValue::Bool(_)),
        ConfigShapeValueType::Json => true,
        ConfigShapeValueType::JsonObject => matches!(value, MetadataValue::Object(_)),
    };
    if !valid {
        return Err(ProjectionError::RequirementBindingMismatch {
            kind: "config",
            key: requirement.path.clone(),
            message: format!("literal is not {}", requirement.value_type),
        });
    }
    Ok(())
}
