use std::collections::BTreeMap;

use skiff_artifact_model::{PackageConfigRequirement, PackageRuntimeRequirements};
use skiff_compiler_projection_input::{ConfigRequirementAccessProjection, ConfigRequirementsSeed};

use crate::error::ProjectionError;

pub(super) fn project_runtime_requirements(
    package_id: &str,
    requirements: &ConfigRequirementsSeed,
) -> Result<PackageRuntimeRequirements, ProjectionError> {
    let mut config_by_path = BTreeMap::<String, PackageConfigRequirement>::new();
    for requirement in requirements.effective().requirements() {
        let (value_type, required) = match requirement.access() {
            ConfigRequirementAccessProjection::Require { ty } => (ty.clone(), true),
            ConfigRequirementAccessProjection::Optional { ty } => (ty.clone(), false),
            ConfigRequirementAccessProjection::Has => continue,
        };
        let projected = PackageConfigRequirement {
            path: requirement.path().to_string(),
            value_type,
            required,
        };
        match config_by_path.get(&projected.path) {
            Some(existing) if existing == &projected => {}
            Some(existing) => {
                return Err(ProjectionError::InvalidPackageArtifact {
                    message: format!(
                        "package {package_id} config requirement {} conflicts: {:?} vs {:?}",
                        projected.path, existing, projected
                    ),
                });
            }
            None => {
                config_by_path.insert(projected.path.clone(), projected);
            }
        }
    }
    Ok(PackageRuntimeRequirements {
        config: config_by_path.into_values().collect(),
        resources: Vec::new(),
        runtime_capabilities: Vec::new(),
    })
}
