mod boundary;
mod callables;
mod model;

pub use boundary::project_boundary_callable;
pub use model::{PackageArtifactProjectionInput, ProjectedPackageArtifact};

use skiff_artifact_identity::assign_package_artifact_identities;
use skiff_artifact_model::{
    PackageArtifact, PackageBuildId, PackageLocalAbi, PackageLocalAbiIdentity,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use crate::{error::ProjectionError, package_unit_artifacts::file_ir_refs_for_projected};

pub fn project_package_artifact(
    mut input: PackageArtifactProjectionInput<'_>,
) -> Result<ProjectedPackageArtifact, ProjectionError> {
    let mut files = file_ir_refs_for_projected(&input.file_ir_units);
    files.sort_by(|left, right| {
        (&left.file_ir_identity, &left.module_path)
            .cmp(&(&right.file_ir_identity, &right.module_path))
    });
    let mut static_resources = input
        .resources
        .iter()
        .map(|resource| skiff_artifact_model::PublicationResourceRef {
            path: resource.path.clone(),
            sha256: resource.sha256.clone(),
            byte_len: resource.byte_len,
            content_type: resource.content_type.clone(),
            artifact_path: None,
        })
        .collect::<Vec<_>>();
    static_resources.sort_by(|left, right| left.path.cmp(&right.path));

    let callables = callables::project_package_callable_surface(
        input.package_id,
        input.api_exports,
        &input.export_index,
        &input.file_ir_units,
        &input.callable_semantic_facts,
        &input.callable_signatures,
        &input.runtime_requirements,
    )?;
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: input.package_id.to_string(),
        package_version: input.package_version.to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files,
        static_resources,
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: callables.public_symbols,
        },
        implementation_links: callables.implementation_links,
        callable_links: callables.callable_links,
        package_requirements: std::mem::take(&mut input.package_requirements),
        contract_requirements: std::mem::take(&mut input.contract_requirements),
        service_requirements: std::mem::take(&mut input.service_requirements),
        runtime_requirements: input.runtime_requirements,
        callable_semantic_facts: callables.semantic_facts,
        boundary_projections: callables.boundary_projections,
        service_call_refs: std::mem::take(&mut input.service_call_refs),
    };
    normalize_artifact_lists(&mut artifact);
    assign_package_artifact_identities(&mut artifact).map_err(|error| {
        ProjectionError::ContractValidation {
            message: format!(
                "package {}@{} identity projection failed: {error}",
                input.package_id, input.package_version
            ),
        }
    })?;

    Ok(ProjectedPackageArtifact {
        artifact,
        file_ir_units: input.file_ir_units,
        resources: input.resources,
    })
}

fn normalize_artifact_lists(artifact: &mut PackageArtifact) {
    artifact.package_requirements.sort_by(|left, right| {
        (&left.alias, &left.package_id, &left.exact_version).cmp(&(
            &right.alias,
            &right.package_id,
            &right.exact_version,
        ))
    });
    artifact.contract_requirements.sort();
    artifact
        .service_requirements
        .sort_by_key(|requirement| requirement.service_binding_slot);
    artifact.service_call_refs.sort_by(|left, right| {
        (
            left.service_requirement_slot,
            left.contract_operation_id.as_str(),
        )
            .cmp(&(
                right.service_requirement_slot,
                right.contract_operation_id.as_str(),
            ))
    });
    artifact
        .runtime_requirements
        .config
        .sort_by(|left, right| left.path.cmp(&right.path));
    artifact
        .runtime_requirements
        .resources
        .sort_by(|left, right| left.key.cmp(&right.key));
    artifact
        .runtime_requirements
        .runtime_capabilities
        .sort_by(|left, right| left.capability.cmp(&right.capability));
}

#[cfg(test)]
mod tests;
