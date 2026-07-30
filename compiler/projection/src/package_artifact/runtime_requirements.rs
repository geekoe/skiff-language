use skiff_artifact_model::{
    canonicalize_package_config_requirements, PackageConfigAccess, PackageConfigRequirement,
    PackageRuntimeRequirements,
};
use skiff_compiler_projection_input::{
    ConfigRequirementAccessProjection, ConfigRequirementSetProjection,
};

use crate::error::ProjectionError;

pub(super) fn project_runtime_requirements(
    package_id: &str,
    requirements: &ConfigRequirementSetProjection,
) -> Result<PackageRuntimeRequirements, ProjectionError> {
    let projected = requirements.requirements().iter().map(|requirement| {
        let access = match requirement.access() {
            ConfigRequirementAccessProjection::Require { ty } => PackageConfigAccess::Required {
                value_type: ty.clone(),
            },
            ConfigRequirementAccessProjection::Optional { ty } => PackageConfigAccess::Optional {
                value_type: ty.clone(),
            },
            ConfigRequirementAccessProjection::Has => PackageConfigAccess::Presence,
        };
        PackageConfigRequirement {
            path: requirement.path().to_string(),
            access,
        }
    });
    let config = canonicalize_package_config_requirements(projected).map_err(|error| {
        ProjectionError::InvalidPackageArtifact {
            message: format!("package {package_id} config requirements are invalid: {error}"),
        }
    })?;
    Ok(PackageRuntimeRequirements { config })
}
