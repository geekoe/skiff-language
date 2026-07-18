use skiff_artifact_identity::assign_package_artifact_identities;
use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableSemanticFacts, ContractRequirement, FileIrUnit, PackageArtifact, PackageBuildId,
    PackageExportIndex, PackageLocalAbi, PackageLocalAbiIdentity, PackageRequirement,
    PackageRuntimeRequirements, ServiceCallRef, ServiceRequirement,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionPackageCallableSignatureFacts,
};

use crate::error::ProjectionError;

use super::{
    api_exports::project_package_exports,
    assets::{file_ir_refs_from_units, project_package_resources, resource_refs_from_projected},
    callables::project_package_callable_surface,
    export_links::project_package_export_index,
    model::{
        PackageArtifactProjectionInput, PackageExportLinkProjectionInput, ProjectedPackageArtifact,
        ProjectedPackageResource,
    },
    projection_signatures::project_callable_signatures,
    runtime_requirements::project_runtime_requirements,
};

pub fn project_compiled_package_artifact(
    input: PackageArtifactProjectionInput<'_>,
) -> Result<ProjectedPackageArtifact, ProjectionError> {
    let api_exports = project_package_exports(
        input.projection,
        input.package_id,
        &input.package_requirements,
    )?;
    let file_ir_units = input.projection.file_ir_units().to_vec();
    let resources = project_package_resources(input.projection.resources());
    let export_index = project_package_export_index(
        &PackageExportLinkProjectionInput {
            package_id: input.package_id,
            exports: &api_exports,
            file_ir_units: &file_ir_units,
        },
        &input.package_requirements,
    )?;
    let callable_signatures =
        project_callable_signatures(input.package_id, &api_exports, &export_index)?;
    let runtime_requirements = project_runtime_requirements(
        input.package_id,
        input.projection.source().config_requirements(),
    )?;
    project_package_artifact_facts(ProjectedPackageFacts {
        package_id: input.package_id,
        package_version: input.package_version,
        api_exports: &api_exports,
        export_index,
        file_ir_units,
        resources,
        package_requirements: input.package_requirements,
        contract_requirements: input.contract_requirements,
        service_requirements: input.service_requirements,
        runtime_requirements,
        callable_semantic_facts: input.projection.source().callable_semantic_facts().clone(),
        callable_signatures,
        service_call_refs: input.service_call_refs,
    })
}

pub(super) struct ProjectedPackageFacts<'a> {
    pub package_id: &'a str,
    pub package_version: &'a str,
    pub api_exports: &'a super::api_exports::PackageExports,
    pub export_index: PackageExportIndex,
    pub file_ir_units: Vec<FileIrUnit>,
    pub resources: Vec<ProjectedPackageResource>,
    pub package_requirements: Vec<PackageRequirement>,
    pub contract_requirements: Vec<ContractRequirement>,
    pub service_requirements: Vec<ServiceRequirement>,
    pub runtime_requirements: PackageRuntimeRequirements,
    pub callable_semantic_facts: BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
    pub callable_signatures: ProjectionPackageCallableSignatureFacts,
    pub service_call_refs: Vec<ServiceCallRef>,
}

pub(super) fn project_package_artifact_facts(
    mut input: ProjectedPackageFacts<'_>,
) -> Result<ProjectedPackageArtifact, ProjectionError> {
    let callables = project_package_callable_surface(
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
        files: file_ir_refs_from_units(&input.file_ir_units),
        static_resources: resource_refs_from_projected(&input.resources),
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
