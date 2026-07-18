use skiff_artifact_model::{
    ContractRequirement, PackageArtifact, PackageRequirement, ServiceContract,
};
use skiff_compiler_contract::{
    compile_service_contract_definition, ContractDefinitionError, ServiceContractDefinition,
};
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
    shared::package_compile_error::PackageCompileError,
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
    let package_requirements = package_requirements(&input)?;
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
    let projected = project_compiled_package_artifact(PackageArtifactProjectionInput {
        package_id: &package_id,
        package_version: &package_version,
        projection: projection.view(),
        package_requirements,
        contract_requirements,
        service_requirements,
        service_call_refs,
    })?;
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
