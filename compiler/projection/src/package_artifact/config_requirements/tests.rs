use super::*;
use skiff_artifact_model::{
    config_shape_from_package_requirements, BoundaryConfigRequirement, PackageConfigAccess,
    PackageConfigRequirement,
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

fn config_requirement(path: &str, value_type: &str, required: bool) -> BoundaryConfigRequirement {
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
        access: PackageConfigAccess::Required {
            value_type: "string".to_string(),
        },
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

    let error = validate_boundary_config_requirements(
        "example.pkg",
        &callable_id,
        &[config_requirement("app.missing", "string", true)],
        &package_requirements,
    )
    .unwrap_err();
    let ProjectionError::InvalidPackageArtifact { message } = error;
    assert!(
        message.starts_with("package example.pkg artifact projection:"),
        "package context must remain attached to the projection error: {message}"
    );

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
