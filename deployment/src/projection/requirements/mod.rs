mod activation;
mod config;
mod service_selectors;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryConfigRequirement, BoundaryStateKind, PackageArtifact, PackageConfigRequirement,
    PackageResourceRequirement, ServiceDeploymentInput, StateBindingKind,
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
    let mut resources = BTreeMap::<String, PackageResourceRequirement>::new();
    let mut capabilities = BTreeMap::<String, String>::new();
    for artifact in closure.artifacts() {
        let package_requirements = package_runtime_requirements(artifact)?;
        for requirement in package_requirements.config.values() {
            merge_config_requirement(&mut config, requirement)?;
        }
        for requirement in package_requirements.resources.values() {
            merge_resource_requirement(&mut resources, requirement)?;
        }
        for (capability, required_version) in package_requirements.capabilities {
            match capabilities.get(&capability) {
                Some(version) if version != &required_version => {
                    return Err(ProjectionError::ConflictingRequirement {
                        kind: "runtime capability",
                        key: capability,
                        message: format!(
                            "required versions {version} and {required_version} differ"
                        ),
                    });
                }
                Some(_) => {}
                None => {
                    capabilities.insert(capability, required_version);
                }
            }
        }
    }

    let mut states = BTreeMap::new();
    let mut native_capabilities = BTreeSet::new();
    for callable in selected {
        validate_selected_requirements(
            callable,
            &implementation_requirements.config,
            &implementation_requirements.resources,
            &implementation_requirements.capabilities,
            &mut states,
            &mut native_capabilities,
        )?;
    }

    config::validate_bindings(input, &config)?;
    activation::validate_state_bindings(input, &states)?;
    activation::validate_resource_bindings(input, &resources, &native_capabilities)?;
    activation::validate_runtime_capability_bindings(input, &capabilities)
}

struct RuntimeRequirements {
    config: BTreeMap<String, PackageConfigRequirement>,
    resources: BTreeMap<String, PackageResourceRequirement>,
    capabilities: BTreeMap<String, String>,
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
    let mut resources = BTreeMap::new();
    for requirement in &artifact.runtime_requirements.resources {
        if resources
            .insert(requirement.key.clone(), requirement.clone())
            .is_some()
        {
            return Err(repeated_package_requirement(
                artifact,
                "resource",
                &requirement.key,
            ));
        }
    }
    let mut capabilities = BTreeMap::new();
    for requirement in &artifact.runtime_requirements.runtime_capabilities {
        if capabilities
            .insert(
                requirement.capability.clone(),
                requirement.required_version.clone(),
            )
            .is_some()
        {
            return Err(repeated_package_requirement(
                artifact,
                "runtime capability",
                &requirement.capability,
            ));
        }
    }
    Ok(RuntimeRequirements {
        config,
        resources,
        capabilities,
    })
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

fn merge_resource_requirement(
    resources: &mut BTreeMap<String, PackageResourceRequirement>,
    requirement: &PackageResourceRequirement,
) -> ProjectionResult<()> {
    match resources.get(&requirement.key) {
        Some(existing) if existing.capability != requirement.capability => {
            Err(ProjectionError::ConflictingRequirement {
                kind: "resource",
                key: requirement.key.clone(),
                message: format!(
                    "capabilities {} and {} differ",
                    existing.capability, requirement.capability
                ),
            })
        }
        Some(_) => Ok(()),
        None => {
            resources.insert(requirement.key.clone(), requirement.clone());
            Ok(())
        }
    }
}

fn validate_selected_requirements(
    callable: &SelectedCallable<'_>,
    config: &BTreeMap<String, PackageConfigRequirement>,
    resources: &BTreeMap<String, PackageResourceRequirement>,
    capabilities: &BTreeMap<String, String>,
    states: &mut BTreeMap<String, StateBindingKind>,
    native_capabilities: &mut BTreeSet<String>,
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
            BoundaryStateKind::ExternalResource => {
                if !resources.contains_key(&requirement.key) {
                    return Err(ProjectionError::CallableFactsMismatch {
                        callable_id: callable.callable_id.clone(),
                        message: format!(
                            "external resource {} is absent from package runtime requirements",
                            requirement.key
                        ),
                    });
                }
            }
            BoundaryStateKind::Database | BoundaryStateKind::Actor | BoundaryStateKind::Queue => {
                let kind = match requirement.kind {
                    BoundaryStateKind::Database => StateBindingKind::Database,
                    BoundaryStateKind::Actor => StateBindingKind::Actor,
                    BoundaryStateKind::Queue => StateBindingKind::Queue,
                    BoundaryStateKind::ExternalResource => unreachable!(),
                };
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
        native_capabilities.insert(capability.clone());
    }
    let mut selected_runtime = BTreeSet::new();
    for capability in &callable.requirements.runtime_capabilities {
        if !selected_runtime.insert(capability.as_str()) {
            return Err(duplicate_callable_requirement(
                callable,
                "runtime capability",
                capability,
            ));
        }
        if !capabilities.contains_key(capability) {
            return Err(ProjectionError::CallableFactsMismatch {
                callable_id: callable.callable_id.clone(),
                message: format!(
                    "runtime capability {capability} is absent from package runtime requirements"
                ),
            });
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
