use std::collections::BTreeMap as StdBTreeMap;

use serde_json::Value;

use crate::error::ProjectionError;
use crate::package_unit_artifacts::ProjectedPackageIrArtifacts;
use skiff_artifact_model::ConfigShapeValueType;
use skiff_compiler_projection_input::PackageProjectionInput;

pub struct PackageConfigValidationInput<'a> {
    pub package_publications: &'a [PackageProjectionInput],
    pub package_artifacts: &'a [ProjectedPackageIrArtifacts],
}

pub fn validate_package_dependency_configs(
    input: PackageConfigValidationInput<'_>,
) -> Result<(), ProjectionError> {
    validate_package_dependency_configs_inner(input.package_publications, input.package_artifacts)
}

fn validate_package_dependency_configs_inner(
    package_publications: &[PackageProjectionInput],
    packages: &[ProjectedPackageIrArtifacts],
) -> Result<(), ProjectionError> {
    let config_shape_by_id = packages
        .iter()
        .map(|package| {
            (
                package.unit.package_id.as_str(),
                &package.config_projection.shape,
            )
        })
        .collect::<StdBTreeMap<_, _>>();
    let mut violations = Vec::new();

    for package in package_publications {
        let has_config = !crate::context::dependency_config_is_empty(package.config());
        if !has_config {
            continue;
        }
        let Some(shape) = config_shape_by_id.get(package.manifest().id()) else {
            continue;
        };
        for entry in &shape.entries {
            if entry.path.is_empty() {
                continue;
            }
            let value = config_path_value(package.config(), &entry.path);
            match value {
                Some(value) if !value.is_null() && !config_value_matches_type(value, entry.ty) => {
                    violations.push(format!(
                        "package dependency {} config path {} must be {}",
                        package.manifest().id(),
                        entry.path,
                        entry.ty
                    ));
                }
                _ => {}
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(ProjectionError::ContractValidation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

fn config_path_value<'a>(config: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = config;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn config_value_matches_type(value: &Value, ty: ConfigShapeValueType) -> bool {
    match ty {
        ConfigShapeValueType::String => value.is_string(),
        ConfigShapeValueType::Number => value.as_f64().is_some_and(f64::is_finite),
        ConfigShapeValueType::Bool => value.is_boolean(),
        ConfigShapeValueType::Json => true,
        ConfigShapeValueType::JsonObject => value.is_object(),
    }
}
