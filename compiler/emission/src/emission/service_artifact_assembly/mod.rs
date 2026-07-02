use std::collections::BTreeMap as StdBTreeMap;

use crate::emission::artifact::PublishedServiceArtifacts;
use crate::emission::artifact_assembly::PublishedPackageArtifacts;
use crate::emission::file_ir_artifacts::published_file_ir_artifacts_from_units_with_projection_sources;
use crate::emission::package_unit_artifacts::{
    package_index_with_package_unit, publish_package_ir_artifacts,
};
use crate::emission::service_artifacts::{
    EmissionContext, PublishedArtifacts, ServiceArtifactEmissionInput,
};
use crate::error::{EmissionError, Result};
use crate::projection::context::PackageApiSourceProjection;
use crate::projection::prelude_metadata::PreludeMetadata;
use crate::projection::service::artifacts::ServiceArtifactProjection;
use crate::projection::{
    contract::ContractProjection, package_unit_artifacts::ProjectedPackageIrArtifacts,
    ConfigProjection, ProjectionView, RuntimeManifestProjection,
};

pub(crate) struct ServiceArtifactAssemblyInput<'a> {
    pub(crate) service_input: ProjectionView<'a>,
    pub(crate) api_source: Option<&'a PackageApiSourceProjection>,
    pub(crate) contract_projection: &'a ContractProjection,
    pub(crate) config_projection: &'a ConfigProjection,
    pub(crate) runtime_manifest_projection: &'a RuntimeManifestProjection,
    pub(crate) package_artifacts: &'a [PublishedPackageArtifacts],
    pub(crate) package_ir_projections: &'a [ProjectedPackageIrArtifacts],
    pub(crate) artifact_projection: &'a ServiceArtifactProjection,
    pub(crate) prelude_metadata: &'a PreludeMetadata,
    pub(crate) service_http_response_max_bytes: Option<u64>,
}

pub(crate) fn build_service_artifacts(
    input: ServiceArtifactAssemblyInput<'_>,
) -> Result<PublishedServiceArtifacts> {
    let runtime_manifest_projection = input.runtime_manifest_projection;
    let contract_projection = input.contract_projection;
    let config_projection = input.config_projection;
    let package_artifacts = input.package_artifacts;
    let package_ir_projection_by_id = input
        .package_ir_projections
        .iter()
        .map(|projection| (projection.unit.package_id.as_str(), projection))
        .collect::<StdBTreeMap<_, _>>();
    let package_ir_artifacts = package_artifacts
        .iter()
        .map(|package| {
            let Some(projection) = package_ir_projection_by_id.get(package.package_id.as_str())
            else {
                return Err(EmissionError::ContractValidation {
                    message: format!(
                        "package {} has published artifacts but no package IR projection",
                        package.package_id
                    ),
                });
            };
            publish_package_ir_artifacts(package, projection)
        })
        .collect::<Result<Vec<_>>>()?;
    let package_file_ir_units = package_ir_artifacts
        .iter()
        .flat_map(|package| package.file_ir_units.iter().cloned())
        .collect::<Vec<_>>();
    let package_units = package_ir_artifacts
        .iter()
        .map(|package| package.package_unit.clone())
        .collect::<Vec<_>>();
    let package_units_typed = package_ir_artifacts
        .iter()
        .map(|package| package.unit.clone())
        .collect::<Vec<_>>();
    let package_assemblies = package_artifacts
        .iter()
        .map(|package| package.assembly.clone())
        .collect::<Vec<_>>();
    let package_indexes = package_artifacts
        .iter()
        .zip(package_ir_artifacts.iter())
        .map(|(package, package_ir)| {
            package_index_with_package_unit(package, &package_ir.unit, &package_ir.package_unit)
        })
        .collect::<Vec<_>>();
    debug_assert_eq!(
        input.artifact_projection.package_units_typed.len(),
        package_units_typed.len()
    );
    let artifact_emission = ServiceArtifactEmissionInput {
        manifest: &runtime_manifest_projection.manifest,
        api_source: input.api_source,
        service_http_response_max_bytes: input.service_http_response_max_bytes,
        contract: contract_projection,
        prelude_metadata: input.prelude_metadata,
        config_shape: &config_projection.shape,
        config_uses: &config_projection.uses,
        config_activation: &config_projection.activation,
        config_requirements: &config_projection.requirements,
        canonical_contract_schema: &runtime_manifest_projection.canonical_contract_schema,
        artifact_projection: input.artifact_projection,
    };
    let file_ir_units = published_file_ir_artifacts_from_units_with_projection_sources(
        &input.artifact_projection.file_ir_units,
        input.service_input,
    )?;
    let projections = PublishedArtifacts::emit(
        artifact_emission,
        EmissionContext::for_manifest(
            &runtime_manifest_projection.manifest,
            &file_ir_units,
            &package_units,
        ),
    )?;

    Ok(PublishedServiceArtifacts {
        file_ir_units,
        package_file_ir_units,
        package_assemblies,
        package_indexes,
        package_units,
        service_assembly: projections.service_assembly,
        service_unit: projections.service_unit,
        contract_schema: projections.contract_schema,
        bundle: projections.bundle,
        index: projections.index,
    })
}
