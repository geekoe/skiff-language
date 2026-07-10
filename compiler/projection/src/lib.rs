mod config;
pub mod context;
pub mod contract;
pub mod contract_schema;
pub mod error;
pub mod package_exports;
pub mod package_unit_artifacts;
pub mod prelude;
pub mod prelude_metadata;
mod publication_visible_types;
pub mod recoverable_boundary;
pub mod runtime;
pub mod runtime_manifest;
pub mod runtime_manifest_model;
pub mod schema_metadata;
pub mod service;
pub mod service_config;
pub mod source_map;
mod source_symbol;
pub mod std_type_refs;
pub mod typed_artifacts;

use crate::{
    context::{
        PackageApiEntryProjection, PackageApiSourceProjection, PackageProjectionContextInput,
        ProjectedPackageDependency,
    },
    error::ProjectionError,
    package_unit_artifacts::{
        project_package_ir_artifacts, PackageFileIrProjection, PackageIrProjectionSource,
        ProjectedPackageIrArtifacts,
    },
    prelude::PreludeProjection,
};
use skiff_artifact_model::AbiIdentityFacts;
use skiff_compiler_core::prelude_registry::PRELUDE_REGISTRY_ID;
use skiff_compiler_projection_input::{PackageProjectionInput, PackagePublicationProjectionInfo};

pub use config::{
    project_config_projection, ConfigActivation, ConfigProjection, ConfigRequirementsProjection,
    ConfigShape, ConfigUseEntry,
};
#[allow(unused_imports)]
pub use context::{
    PackageProjectionContext, ProjectionContext, ProjectionPolicy, ServiceProjectionContext,
};
#[allow(unused_imports)]
pub use contract::{ContractProjectionIndex, ContractProjectionTypeBinding, ContractTypeKind};
pub use package_exports::package_exports_projection;
pub use runtime_manifest::{project_runtime_manifest_projection, RuntimeManifestProjection};
pub use service_config::{
    ServiceAccessProjectionConfig, TimeoutProjectionConfig, WebSocketContextProjectionConfig,
    WebSocketGatewayProjectionConfig, WebSocketOperationProjectionConfig,
};
pub use skiff_compiler_projection_input::{ProjectionSourceMetadata, ProjectionView};
pub use source_symbol::projection_source_symbol_text;

pub struct PackageProjectionBundle<'a> {
    pub input: ProjectionView<'a>,
    pub exports: package_exports::PackageExports,
    pub abi_identity_projection: AbiIdentityFacts,
    pub config_projection: ConfigProjection,
    pub source_map: source_map::PublicationSourceMap,
}

pub struct ProjectedPackagePublication<'a> {
    pub source: &'a PackageProjectionInput,
    pub bundle: PackageProjectionBundle<'a>,
    pub package_ir: ProjectedPackageIrArtifacts,
}

impl<'a> ProjectedPackagePublication<'a> {
    pub fn manifest(&self) -> &PackagePublicationProjectionInfo {
        self.source.manifest()
    }
}

pub struct ServiceProjectionBundle<'a> {
    pub input: ProjectionView<'a>,
    pub api_source: Option<PackageApiSourceProjection>,
    pub contract_projection: contract::ContractProjection,
    pub config_projection: ConfigProjection,
    pub runtime_manifest_projection: RuntimeManifestProjection,
    pub artifact_projection: service::artifacts::ServiceArtifactProjection,
    pub prelude_metadata: prelude_metadata::PreludeMetadata,
    pub service_http_response_max_bytes: Option<u64>,
}

pub fn project_service<'a>(
    input: ProjectionView<'a>,
    context: ServiceProjectionContext<'a>,
) -> Result<ServiceProjectionBundle<'a>, ProjectionError> {
    let projection_context = ProjectionContext::Service(context.clone());
    let service_ingress =
        input
            .service_ingress()
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: "service projection requires compiled service ingress".to_string(),
            })?;
    let package_gateway_projection =
        runtime::PackageGatewayProjection::build(service_ingress, context.package_publications())?;
    let contract_projection_bundle = service::contract::build_service_contract_projection(
        context.publication_api_has_entries(),
        context.timeout(),
        context.websocket_gateway(),
        input,
        context.prelude(),
    )?;
    let contract_projection_index =
        contract::ContractProjectionIndex::from_projection_input_with_prelude(
            input,
            Some(context.prelude()),
        );
    let public_instances = service::service_unit::service_unit_public_instances(
        input,
        &contract_projection_bundle.contract_projection,
        &contract_projection_index,
        context.package_dependencies(),
    )?;
    let runtime_manifest_projection = project_runtime_manifest_projection(
        input,
        &contract_projection_bundle.contract_projection,
        context.service_version(),
        &public_instances,
        &projection_context,
        &package_gateway_projection,
    )?;
    runtime::validate_service_storage_projection_namespace(
        service_has_storage_metadata(input),
        context.service_id(),
    )?;
    let prelude_metadata = prelude_metadata::prelude_metadata_json(context.prelude());
    let artifact_projection = service::artifact_assembly::project_service_artifact_projection(
        service::artifact_assembly::ServiceArtifactProjectionInput {
            service_input: input,
            package_dependencies: context.package_dependencies(),
            service_version: context.service_version(),
            contract_projection: &contract_projection_bundle.contract_projection,
            runtime_manifest_projection: &runtime_manifest_projection,
            public_instances: &public_instances,
            package_publications: context.package_publications(),
            package_artifacts: context.package_artifacts(),
        },
    )?;

    Ok(ServiceProjectionBundle {
        input,
        api_source: context.api_source().cloned(),
        contract_projection: contract_projection_bundle.contract_projection,
        config_projection: contract_projection_bundle.config_projection,
        runtime_manifest_projection,
        artifact_projection,
        prelude_metadata,
        service_http_response_max_bytes: context.service_http_response_max_bytes(),
    })
}

pub fn project_package<'a>(
    input: ProjectionView<'a>,
    context: PackageProjectionContext<'_>,
) -> Result<PackageProjectionBundle<'a>, ProjectionError> {
    let projection_context = ProjectionContext::Package(context);
    let exports = package_exports_projection(input, &projection_context)?;
    let contract_projection =
        contract::project_contract_projection(input, projection_context.prelude()).map_err(
            |error| ProjectionError::ContractValidation {
                message: format!("package ABI contract projection: {error:?}"),
            },
        )?;
    let contract_projection_index =
        contract::ContractProjectionIndex::from_projection_input_with_prelude(
            input,
            Some(projection_context.prelude()),
        );
    let abi_identity_projection =
        contract::project_abi_identity(&contract_projection, &contract_projection_index)
            .to_artifact_facts();
    let config_projection = project_config_projection(input.source().config_requirements())?;
    let source_map = source_map::publication_source_map_from_file_ir_units(input.file_ir_units())?;

    Ok(PackageProjectionBundle {
        input,
        exports,
        abi_identity_projection,
        config_projection,
        source_map,
    })
}

pub fn project_package_publications<'a>(
    package_publications: &'a [PackageProjectionInput],
    prelude: &PreludeProjection,
) -> Result<Vec<ProjectedPackagePublication<'a>>, ProjectionError> {
    package_publications
        .iter()
        .map(|package| project_package_publication(package, prelude))
        .collect()
}

pub fn project_package_ir_publications(
    package_publications: &[ProjectedPackagePublication<'_>],
) -> Result<Vec<ProjectedPackageIrArtifacts>, ProjectionError> {
    Ok(package_publications
        .iter()
        .filter(|package| package_projection_artifact_is_published(package))
        .map(|package| package.package_ir.clone())
        .collect())
}

fn package_projection_artifact_is_published(package: &ProjectedPackagePublication<'_>) -> bool {
    package.manifest().id() != PRELUDE_REGISTRY_ID && !package.manifest().provenance().synthetic()
}

fn project_package_ir_publication(
    package_publication: &PackageProjectionInput,
    bundle: &PackageProjectionBundle<'_>,
) -> Result<ProjectedPackageIrArtifacts, ProjectionError> {
    let dependencies = projected_package_dependencies(package_publication);
    project_package_ir_artifacts(
        PackageIrProjectionSource {
            package_id: package_publication.id(),
            version: package_publication.version(),
            exports: &bundle.exports,
            abi_identity_projection: &bundle.abi_identity_projection,
            config_projection: &bundle.config_projection,
            resources: package_publication.compiled().resources(),
            file_ir_units: bundle
                .input
                .file_ir_units()
                .iter()
                .cloned()
                .map(PackageFileIrProjection::from_unit)
                .collect(),
        },
        &dependencies,
    )
}

fn project_package_publication<'a>(
    package_publication: &'a PackageProjectionInput,
    prelude: &PreludeProjection,
) -> Result<ProjectedPackagePublication<'a>, ProjectionError> {
    let context = PackageProjectionContext::new(PackageProjectionContextInput {
        package_id: package_publication.id(),
        version: package_publication.version(),
        dependencies: projected_package_dependencies(package_publication),
        api_entries: package_publication
            .api_entries()
            .iter()
            .map(|entry| PackageApiEntryProjection {
                path: entry.path().to_string(),
                module: entry.module().to_string(),
            })
            .collect(),
        api_source: package_publication
            .api_source()
            .map(|source| PackageApiSourceProjection {
                relative_path: source.relative_path().to_path_buf(),
                content_hash: source.content_hash().to_string(),
            }),
        package_root: package_publication.source_root(),
        prelude,
    });
    let bundle = project_package(package_publication.compiled(), context)?;
    let package_ir = project_package_ir_publication(package_publication, &bundle)?;
    Ok(ProjectedPackagePublication {
        source: package_publication,
        bundle,
        package_ir,
    })
}

fn projected_package_dependencies(
    package_publication: &PackageProjectionInput,
) -> Vec<ProjectedPackageDependency> {
    package_publication
        .dependencies()
        .iter()
        .map(|dependency| ProjectedPackageDependency {
            id: dependency.id().to_string(),
            version: dependency.version().to_string(),
            alias: dependency.alias().map(str::to_string),
            config: dependency.config().clone(),
            collection_name_mapping: dependency.collection_name_mapping().clone(),
        })
        .collect()
}

fn service_has_storage_metadata(input: ProjectionView<'_>) -> bool {
    let lowering = input.lowering();
    !lowering.service_db_metadata().is_empty() || !lowering.service_actor_metadata().is_empty()
}
