mod activation;
mod config;
mod service_selectors;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryConfigRequirement, BoundaryStateKind, PackageArtifact, PackageConfigRequirement,
    PackageStateRequirement, ServiceDeploymentInput, StateBindingKind,
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
    let mut config = BTreeMap::<String, PackageConfigRequirement>::new();
    let mut states = BTreeMap::<String, StateBindingKind>::new();
    for artifact in closure.artifacts() {
        let package_requirements = package_runtime_requirements(artifact)?;
        for requirement in package_requirements.config.values() {
            merge_config_requirement(&mut config, requirement)?;
        }
        for requirement in package_requirements.state.values() {
            merge_state_requirement(&mut states, requirement)?;
        }
    }

    for callable in selected {
        validate_selected_requirements(
            callable,
            &implementation_requirements.config,
            &implementation_requirements.state,
            &mut states,
        )?;
    }

    config::validate_bindings(input, &config)?;
    activation::validate_state_bindings(input, &states)
}

struct RuntimeRequirements {
    config: BTreeMap<String, PackageConfigRequirement>,
    state: BTreeMap<String, PackageStateRequirement>,
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
    let mut state = BTreeMap::new();
    for requirement in &artifact.runtime_requirements.state {
        if state
            .insert(requirement.key.clone(), requirement.clone())
            .is_some()
        {
            return Err(repeated_package_requirement(
                artifact,
                "state",
                &requirement.key,
            ));
        }
    }
    Ok(RuntimeRequirements { config, state })
}

fn merge_state_requirement(
    states: &mut BTreeMap<String, StateBindingKind>,
    requirement: &PackageStateRequirement,
) -> ProjectionResult<()> {
    match states.get(&requirement.key) {
        Some(existing) if existing != &requirement.kind => {
            Err(ProjectionError::ConflictingRequirement {
                kind: "state",
                key: requirement.key.clone(),
                message: format!(
                    "required kinds {existing:?} and {:?} differ",
                    requirement.kind
                ),
            })
        }
        Some(_) => Ok(()),
        None => {
            states.insert(requirement.key.clone(), requirement.kind);
            Ok(())
        }
    }
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

fn merge_config_requirement(
    config: &mut BTreeMap<String, PackageConfigRequirement>,
    requirement: &PackageConfigRequirement,
) -> ProjectionResult<()> {
    match config.get_mut(&requirement.path) {
        Some(existing) if existing.value_type != requirement.value_type => {
            Err(ProjectionError::ConflictingRequirement {
                kind: "config",
                key: requirement.path.clone(),
                message: format!(
                    "value types {} and {} differ",
                    existing.value_type, requirement.value_type
                ),
            })
        }
        Some(existing) => {
            existing.required |= requirement.required;
            Ok(())
        }
        None => {
            config.insert(requirement.path.clone(), requirement.clone());
            Ok(())
        }
    }
}

fn validate_selected_requirements(
    callable: &SelectedCallable<'_>,
    config: &BTreeMap<String, PackageConfigRequirement>,
    declared_states: &BTreeMap<String, PackageStateRequirement>,
    states: &mut BTreeMap<String, StateBindingKind>,
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
    let mut selected_state = BTreeSet::new();
    for requirement in &callable.requirements.state {
        if !selected_state.insert(requirement.key.as_str()) {
            return Err(duplicate_callable_requirement(
                callable,
                "state",
                &requirement.key,
            ));
        }
        match requirement.kind {
            BoundaryStateKind::Database
            | BoundaryStateKind::Redis
            | BoundaryStateKind::Actor
            | BoundaryStateKind::Queue => {
                let kind = match requirement.kind {
                    BoundaryStateKind::Database => StateBindingKind::Database,
                    BoundaryStateKind::Redis => StateBindingKind::Redis,
                    BoundaryStateKind::Actor => StateBindingKind::Actor,
                    BoundaryStateKind::Queue => StateBindingKind::Queue,
                };
                match declared_states.get(&requirement.key) {
                    Some(declared) if declared.kind == kind => {}
                    Some(declared) => {
                        return Err(ProjectionError::CallableFactsMismatch {
                            callable_id: callable.callable_id.clone(),
                            message: format!(
                                "state {} kind {:?} does not match package runtime requirement {:?}",
                                requirement.key, kind, declared.kind
                            ),
                        });
                    }
                    None => {
                        return Err(ProjectionError::CallableFactsMismatch {
                            callable_id: callable.callable_id.clone(),
                            message: format!(
                                "state {} is absent from package runtime requirements",
                                requirement.key
                            ),
                        });
                    }
                }
                match states.get(&requirement.key) {
                    Some(existing) if existing != &kind => {
                        return Err(ProjectionError::ConflictingRequirement {
                            kind: "state",
                            key: requirement.key.clone(),
                            message: format!("required kinds {existing:?} and {kind:?} differ"),
                        });
                    }
                    Some(_) => {}
                    None => {
                        states.insert(requirement.key.clone(), kind);
                    }
                }
            }
        }
    }
    let mut selected_native = BTreeSet::new();
    for capability in &callable.requirements.native_capabilities {
        if !selected_native.insert(capability.as_str()) {
            return Err(duplicate_callable_requirement(
                callable,
                "native capability",
                capability,
            ));
        }
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
    if package.value_type != boundary.value_type || package.required != boundary.required {
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
