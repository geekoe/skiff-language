mod dependencies;
mod exports;
mod metadata;
mod model;
mod refs;

pub use dependencies::{
    package_dependency_constraint, package_unit_dependency_constraints,
    std_package_dependency_constraint,
};
pub use exports::project_package_export_index;
pub use metadata::config_metadata_from_config_projection;
pub use model::{
    PackageFileIrProjection, PackageIrProjectionSource, ProjectedPackageIrArtifacts,
    ProjectedPublicationResource,
};
pub use refs::{
    file_ir_refs_for_projected, projected_publication_resources, resource_refs_for_projected,
};

use skiff_artifact_identity::assign_package_unit_identities;
use skiff_artifact_model::{
    ConfigAndEffectMetadata, PackageUnit, RecoverableArtifactMetadata, PACKAGE_UNIT_SCHEMA_VERSION,
};

use crate::context::ProjectedPackageDependency;
use crate::error::ProjectionError;

pub fn project_package_ir_artifacts(
    mut package: PackageIrProjectionSource<'_>,
    dependencies: &[ProjectedPackageDependency],
) -> Result<ProjectedPackageIrArtifacts, ProjectionError> {
    let file_refs = file_ir_refs_for_projected(&package.file_ir_units);
    let resources = projected_publication_resources(package.resources);
    let resource_refs = resource_refs_for_projected(&resources);
    let exports = project_package_export_index(&package, dependencies)?;
    let dependency_constraints = package_unit_dependency_constraints(
        dependencies,
        &package.file_ir_units,
        package.package_id,
    );
    let publication_abi = skiff_compiler_publication_abi::package_publication_abi(
        package.package_id,
        package.version,
        &exports,
    )
    .map_err(|error| ProjectionError::ContractValidation {
        message: error.to_string(),
    })?;
    let implementation_links =
        skiff_compiler_publication_abi::package_implementation_links(&exports, &publication_abi);
    let config_and_effect_metadata = ConfigAndEffectMetadata {
        config: config_metadata_from_config_projection(package.config_projection),
        effects: metadata::package_callable_effect_facts(
            package.callable_effects,
            &publication_abi,
            &implementation_links,
        )?,
    };
    let mut unit = PackageUnit {
        schema_version: PACKAGE_UNIT_SCHEMA_VERSION.to_string(),
        package_id: package.package_id.to_string(),
        version: package.version.to_string(),
        build_identity: String::new(),
        abi_identity: String::new(),
        abi_identity_projection: package.abi_identity_projection.clone(),
        publication_abi,
        files: file_refs,
        resources: resource_refs,
        implementation_links,
        dependencies: dependency_constraints,
        recoverable_metadata: RecoverableArtifactMetadata::default(),
        config_and_effect_metadata,
    };
    assign_package_unit_identities(&mut unit).map_err(|error| {
        ProjectionError::ContractValidation {
            message: format!(
                "package {}@{} identity projection failed: {error}",
                package.package_id, package.version
            ),
        }
    })?;

    Ok(ProjectedPackageIrArtifacts {
        unit,
        config_projection: package.config_projection.clone(),
        file_ir_units: std::mem::take(&mut package.file_ir_units),
        resources,
    })
}
