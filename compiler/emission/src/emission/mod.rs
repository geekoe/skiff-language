pub mod artifact;
pub mod artifact_assembly;
pub mod file_ir_artifacts;
pub mod identity;
pub mod package_artifacts;
pub mod package_test_artifacts;
pub mod package_unit_artifacts;
pub mod service_artifact_assembly;
pub mod service_artifacts;
pub mod service_publication;

#[allow(unused_imports)]
pub use package_artifacts::{emit_package, PackageEmissionContext};

use crate::{
    emission::{
        artifact::PublishedServiceArtifacts,
        artifact_assembly::PublishedPackageArtifacts,
        service_artifact_assembly::{build_service_artifacts, ServiceArtifactAssemblyInput},
    },
    error::Result,
    projection::{package_unit_artifacts::ProjectedPackageIrArtifacts, ServiceProjectionBundle},
};

pub struct ServiceEmissionContext<'a> {
    pub package_artifacts: &'a [PublishedPackageArtifacts],
    pub package_ir_projections: &'a [ProjectedPackageIrArtifacts],
}

pub fn emit_service(
    bundle: &ServiceProjectionBundle<'_>,
    context: ServiceEmissionContext<'_>,
) -> Result<PublishedServiceArtifacts> {
    build_service_artifacts(ServiceArtifactAssemblyInput {
        service_input: bundle.input,
        api_source: bundle.api_source.as_ref(),
        contract_projection: &bundle.contract_projection,
        config_projection: &bundle.config_projection,
        runtime_manifest_projection: &bundle.runtime_manifest_projection,
        package_artifacts: context.package_artifacts,
        package_ir_projections: context.package_ir_projections,
        artifact_projection: &bundle.artifact_projection,
        prelude_metadata: &bundle.prelude_metadata,
        service_http_response_max_bytes: bundle.service_http_response_max_bytes,
    })
}
