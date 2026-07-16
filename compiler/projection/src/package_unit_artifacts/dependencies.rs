use skiff_artifact_model::PackageDependencyConstraint;
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;

use crate::{
    context::ProjectedPackageDependency, package_references::file_ir_units_reference_package,
};

use super::PackageFileIrProjection;

pub fn package_unit_dependency_constraints(
    dependencies: &[ProjectedPackageDependency],
    file_ir_units: &[PackageFileIrProjection],
    package_id: &str,
) -> Vec<PackageDependencyConstraint> {
    let mut constraints = dependencies
        .iter()
        .map(package_dependency_constraint)
        .collect::<Vec<_>>();
    if package_id != SKIFF_STD_PUBLICATION_ID
        && file_ir_units_reference_package(
            file_ir_units.iter().map(|artifact| &artifact.unit),
            SKIFF_STD_PUBLICATION_ID,
        )
        && !constraints
            .iter()
            .any(|dependency| dependency.id == SKIFF_STD_PUBLICATION_ID)
    {
        constraints.push(std_package_dependency_constraint());
    }
    constraints
}

pub fn package_dependency_constraint(
    dependency: &ProjectedPackageDependency,
) -> PackageDependencyConstraint {
    PackageDependencyConstraint {
        id: dependency.id.clone(),
        version: dependency.version.clone(),
        alias: dependency.effective_alias().to_string(),
        config: dependency.config.clone(),
    }
}

pub fn std_package_dependency_constraint() -> PackageDependencyConstraint {
    PackageDependencyConstraint {
        id: SKIFF_STD_PUBLICATION_ID.to_string(),
        version: "1.0.0".to_string(),
        alias: "std".to_string(),
        config: crate::context::empty_dependency_config(),
    }
}
