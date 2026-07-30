mod service_selectors;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryConfigRequirement, PackageArtifact, PackageConfigAccess, PackageConfigRequirement,
    ServiceDeploymentInput,
};

use super::{
    operations::SelectedCallable, package_closure::PackageClosure, ProjectionError,
    ProjectionResult,
};

pub(super) fn validate_requirement_bindings(
    input: &ServiceDeploymentInput,
    closure: &PackageClosure<'_>,
    selected: &[SelectedCallable<'_>],
) -> ProjectionResult<()> {
    service_selectors::validate(input, closure)?;

    let implementation_requirements = package_runtime_requirements(closure.implementation(input))?;
    for callable in selected {
        validate_selected_requirements(callable, &implementation_requirements.config)?;
    }
    Ok(())
}

struct RuntimeRequirements {
    config: BTreeMap<String, PackageConfigRequirement>,
}

fn package_runtime_requirements(
    artifact: &PackageArtifact,
) -> ProjectionResult<RuntimeRequirements> {
    let mut config = BTreeMap::new();
    for requirement in &artifact.runtime_requirements.config {
        if config
            .insert(requirement.path.clone(), requirement.clone())
            .is_some()
        {
            return Err(repeated_package_requirement(
                artifact,
                "config",
                &requirement.path,
            ));
        }
    }
    Ok(RuntimeRequirements { config })
}

fn repeated_package_requirement(
    artifact: &PackageArtifact,
    kind: &'static str,
    key: &str,
) -> ProjectionError {
    ProjectionError::ConflictingRequirement {
        kind,
        key: key.to_string(),
        message: format!(
            "package build {} repeats the requirement",
            artifact.package_build_id
        ),
    }
}

fn validate_selected_requirements(
    callable: &SelectedCallable<'_>,
    config: &BTreeMap<String, PackageConfigRequirement>,
) -> ProjectionResult<()> {
    let mut selected_config = BTreeSet::new();
    for requirement in &callable.requirements.config {
        if !selected_config.insert(requirement.path.as_str()) {
            return Err(duplicate_callable_requirement(
                callable,
                "config",
                &requirement.path,
            ));
        }
        validate_boundary_config(callable, requirement, config)?;
    }
    Ok(())
}

fn duplicate_callable_requirement(
    callable: &SelectedCallable<'_>,
    kind: &str,
    key: &str,
) -> ProjectionError {
    ProjectionError::CallableFactsMismatch {
        callable_id: callable.callable_id.clone(),
        message: format!("{kind} requirement {key} is repeated"),
    }
}

fn validate_boundary_config(
    callable: &SelectedCallable<'_>,
    boundary: &BoundaryConfigRequirement,
    config: &BTreeMap<String, PackageConfigRequirement>,
) -> ProjectionResult<()> {
    let package =
        config
            .get(&boundary.path)
            .ok_or_else(|| ProjectionError::CallableFactsMismatch {
                callable_id: callable.callable_id.clone(),
                message: format!(
                    "config {} is absent from package runtime requirements",
                    boundary.path
                ),
            })?;
    let exact = match &package.access {
        PackageConfigAccess::Presence => false,
        PackageConfigAccess::Optional { value_type } => {
            value_type == &boundary.value_type && !boundary.required
        }
        PackageConfigAccess::Required { value_type } => {
            value_type == &boundary.value_type && boundary.required
        }
    };
    if !exact {
        return Err(ProjectionError::CallableFactsMismatch {
            callable_id: callable.callable_id.clone(),
            message: format!(
                "config {} does not exactly match package runtime requirements",
                boundary.path
            ),
        });
    }
    Ok(())
}
