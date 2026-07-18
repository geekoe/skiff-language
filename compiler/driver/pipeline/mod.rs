use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{
    ContractRequirement, FileIrUnit, PackageArtifact, PackageRefIr, PackageRequirement,
    ServiceContract,
};
use skiff_compiler_contract::{
    compile_service_contract_definition, ContractDefinitionError, ServiceContractDefinition,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_emission::{
    file_ir_artifacts::publish_file_ir_artifacts,
    package_artifact::{publish_projected_package_artifact, PublishedPackageArtifact},
};
use skiff_compiler_projection::package_artifact::{
    project_compiled_package_artifact, PackageArtifactProjectionInput,
};
use skiff_compiler_projection_input::PublicationResourceProjectionInput;

use crate::{
    input::{PackageCompileInput, PackageDependency, PublicationResourceInput},
    shared::package_compile_error::{package_projection_error, PackageCompileError},
    source_compile,
};

/// Compiles one independent package all the way to its canonical artifact.
///
/// The driver supplies only coordinates, typed dependency requirements and the
/// compiled ProjectionView. Export resolution, exact links, callable
/// signatures, runtime requirements, File IR projection and resource
/// projection remain owned by the PackageArtifact projection leaf.
pub fn compile_package(
    input: PackageCompileInput<'_>,
) -> Result<PublishedPackageArtifact, PackageCompileError> {
    let package_id = input.package_id.to_string();
    let package_version = input.package.manifest.version.clone();
    let declared_package_requirements = package_requirements(&input)?;
    let contract_requirements = contract_requirements(&input);
    let compiled = source_compile::compile(&input)?;
    let service_requirements = compiled
        .lowered()
        .service_calls()
        .service_requirements()
        .to_vec();
    let service_call_refs = compiled
        .lowered()
        .service_calls()
        .service_call_ref_closure()
        .into_iter()
        .collect::<Vec<_>>();
    let projection = skiff_compiler_compiled::projection_input::build_projection_input(&compiled)
        .with_resources(resource_projection_inputs(&input.package.resources));
    let package_requirements = complete_package_requirement_closure(
        &package_id,
        declared_package_requirements,
        projection.view().file_ir_units(),
        input.available_packages,
    )?;
    let projected = project_compiled_package_artifact(PackageArtifactProjectionInput {
        package_id: &package_id,
        package_version: &package_version,
        projection: projection.view(),
        package_requirements,
        contract_requirements,
        service_requirements,
        service_call_refs,
    })
    .map_err(package_projection_error)?;
    let file_ir_units = publish_file_ir_artifacts(projection.view())?;
    Ok(publish_projected_package_artifact(
        &projected,
        &file_ir_units,
    )?)
}

/// The code-free contract pipeline. No package/provider source is accepted.
pub fn compile_contract(
    definition: ServiceContractDefinition,
) -> Result<ServiceContract, ContractDefinitionError> {
    compile_service_contract_definition(definition)
}

fn package_requirements(
    input: &PackageCompileInput<'_>,
) -> Result<Vec<PackageRequirement>, PackageCompileError> {
    input
        .package_dependencies
        .iter()
        .map(|dependency| {
            package_requirement(input.package_id, dependency, input.dependency_packages)
        })
        .collect()
}

fn complete_package_requirement_closure(
    owner_package_id: &str,
    mut requirements: Vec<PackageRequirement>,
    file_ir_units: &[FileIrUnit],
    available_artifacts: &[PackageArtifact],
) -> Result<Vec<PackageRequirement>, PackageCompileError> {
    if owner_package_id == SKIFF_STD_PUBLICATION_ID
        || !file_ir_units
            .iter()
            .any(file_ir_unit_references_platform_std)
    {
        return Ok(requirements);
    }

    if requirements.iter().any(|requirement| {
        requirement.alias == "std" || requirement.package_id == SKIFF_STD_PUBLICATION_ID
    }) {
        return Err(validation_error(format!(
            "package {owner_package_id} declares platform std as a package dependency; std requirements are compiler-owned"
        )));
    }

    let mut std_artifact = None;
    for artifact in available_artifacts {
        if artifact.package_id != SKIFF_STD_PUBLICATION_ID {
            continue;
        }
        if std_artifact.replace(artifact).is_some() {
            return Err(validation_error(format!(
                "canonical package graph contains duplicate artifact id {SKIFF_STD_PUBLICATION_ID}"
            )));
        }
    }
    let std_artifact = std_artifact.ok_or_else(|| {
        validation_error(format!(
            "package {owner_package_id} references platform std, but the same compile graph has no canonical PackageArtifact for {SKIFF_STD_PUBLICATION_ID}"
        ))
    })?;
    validate_package_artifact_identities(std_artifact).map_err(|error| {
        validation_error(format!(
            "canonical platform std artifact {}@{} identity validation failed: {error}",
            std_artifact.package_id, std_artifact.package_version
        ))
    })?;

    requirements.push(PackageRequirement {
        alias: "std".to_string(),
        package_id: std_artifact.package_id.clone(),
        exact_version: std_artifact.package_version.clone(),
        expected_local_abi: std_artifact.package_local_abi.local_abi_identity.clone(),
    });
    Ok(requirements)
}

fn file_ir_unit_references_platform_std(file: &FileIrUnit) -> bool {
    file.external_refs
        .package_symbols
        .iter()
        .map(|symbol| &symbol.package)
        .chain(
            file.external_refs
                .package_callables
                .iter()
                .map(|callable| &callable.package_ref),
        )
        .any(package_ref_references_platform_std)
}

fn package_ref_references_platform_std(package_ref: &PackageRefIr) -> bool {
    match package_ref {
        PackageRefIr::PackageId { package_id } => package_id == SKIFF_STD_PUBLICATION_ID,
        PackageRefIr::Dependency { dependency_ref } => dependency_ref == "std",
    }
}

fn package_requirement(
    package_id: &str,
    dependency: &PackageDependency,
    dependency_packages: &[PackageArtifact],
) -> Result<PackageRequirement, PackageCompileError> {
    let artifact = dependency_packages
        .iter()
        .find(|artifact| {
            artifact.package_id == dependency.id && artifact.package_version == dependency.version
        })
        .ok_or_else(|| PackageCompileError::ContractValidation {
            message: format!(
                "package {package_id} dependency {}@{} has no canonical PackageArtifact",
                dependency.id, dependency.version
            ),
        })?;
    Ok(PackageRequirement {
        alias: dependency.effective_alias().to_string(),
        package_id: dependency.id.clone(),
        exact_version: dependency.version.clone(),
        expected_local_abi: artifact.package_local_abi.local_abi_identity.clone(),
    })
}

fn contract_requirements(input: &PackageCompileInput<'_>) -> Vec<ContractRequirement> {
    input
        .contract_dependencies
        .iter()
        .map(|dependency| dependency.requirement.clone())
        .collect()
}

fn resource_projection_inputs(
    resources: &[PublicationResourceInput],
) -> Vec<PublicationResourceProjectionInput> {
    resources
        .iter()
        .map(|resource| {
            PublicationResourceProjectionInput::new(
                resource.path.clone(),
                resource.absolute_path.clone(),
                resource.byte_len,
                resource.sha256.clone(),
                resource.content_type.clone(),
            )
        })
        .collect()
}

fn validation_error(message: String) -> PackageCompileError {
    PackageCompileError::ContractValidation { message }
}

#[cfg(test)]
mod tests;
