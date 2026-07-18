use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    config_shape_from_package_requirements, BoundaryCallableProjection, BoundaryConfigRequirement,
    ConfigShapeEntry, PackageCallableId, PackageRuntimeRequirements,
};

use crate::error::ProjectionError;

pub(super) fn validate_canonical_config_projection(
    package_id: &str,
    runtime_requirements: &PackageRuntimeRequirements,
    boundary_projections: &BTreeMap<PackageCallableId, BoundaryCallableProjection>,
) -> Result<(), ProjectionError> {
    let config_shape = config_shape_from_package_requirements(&runtime_requirements.config)
        .map_err(|error| {
            projection_error(
                package_id,
                format!("canonical runtime config requirements are invalid: {error}"),
            )
        })?;
    let package_requirements = config_shape
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();

    for (callable_id, projection) in boundary_projections {
        let BoundaryCallableProjection::Available {
            implementation_requirements,
            ..
        } = projection
        else {
            continue;
        };
        validate_boundary_config_requirements(
            package_id,
            callable_id,
            &implementation_requirements.config,
            &package_requirements,
        )?;
    }
    Ok(())
}

fn validate_boundary_config_requirements(
    package_id: &str,
    callable_id: &PackageCallableId,
    boundary_requirements: &[BoundaryConfigRequirement],
    package_requirements: &BTreeMap<&str, &ConfigShapeEntry>,
) -> Result<(), ProjectionError> {
    let mut seen_paths = BTreeSet::new();
    for boundary in boundary_requirements {
        if !seen_paths.insert(boundary.path.as_str()) {
            return Err(projection_error(
                package_id,
                format!(
                    "available boundary callable {callable_id} repeats config requirement {}",
                    boundary.path
                ),
            ));
        }
        let Some(package) = package_requirements.get(boundary.path.as_str()) else {
            return Err(projection_error(
                package_id,
                format!(
                    "available boundary callable {callable_id} requires config {}, which is absent from PackageRuntimeRequirements.config",
                    boundary.path
                ),
            ));
        };
        if boundary.value_type != package.ty.as_wire_str() || boundary.required != package.required
        {
            return Err(projection_error(
                package_id,
                format!(
                    "available boundary callable {callable_id} config requirement {} ({}, required={}) conflicts with PackageRuntimeRequirements.config ({}, required={})",
                    boundary.path,
                    boundary.value_type,
                    boundary.required,
                    package.ty,
                    package.required
                ),
            ));
        }
    }
    Ok(())
}

fn projection_error(package_id: &str, message: impl Into<String>) -> ProjectionError {
    ProjectionError::ContractValidation {
        message: format!(
            "package {package_id} artifact projection: {}",
            message.into()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{
        config_shape_from_package_requirements, BoundaryConfigRequirement, PackageConfigRequirement,
    };

    fn package_requirements<'a>(
        shape: &'a skiff_artifact_model::ConfigShape,
    ) -> BTreeMap<&'a str, &'a ConfigShapeEntry> {
        shape
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry))
            .collect()
    }

    fn config_requirement(
        path: &str,
        value_type: &str,
        required: bool,
    ) -> BoundaryConfigRequirement {
        BoundaryConfigRequirement {
            path: path.to_string(),
            value_type: value_type.to_string(),
            required,
        }
    }

    #[test]
    fn available_boundary_config_must_be_an_exact_package_requirement_subset() {
        let shape = config_shape_from_package_requirements(&[PackageConfigRequirement {
            path: "app.token".to_string(),
            value_type: "string".to_string(),
            required: true,
        }])
        .unwrap();
        let package_requirements = package_requirements(&shape);
        let callable_id = PackageCallableId::new("callable:run");

        validate_boundary_config_requirements(
            "example.pkg",
            &callable_id,
            &[config_requirement("app.token", "string", true)],
            &package_requirements,
        )
        .unwrap();

        for boundary_requirements in [
            vec![config_requirement("app.missing", "string", true)],
            vec![config_requirement("app.token", "number", true)],
            vec![config_requirement("app.token", "string", false)],
            vec![
                config_requirement("app.token", "string", true),
                config_requirement("app.token", "string", true),
            ],
        ] {
            assert!(validate_boundary_config_requirements(
                "example.pkg",
                &callable_id,
                &boundary_requirements,
                &package_requirements,
            )
            .is_err());
        }
    }
}
