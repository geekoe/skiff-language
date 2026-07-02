use crate::input::{
    PackageDependency, PublicationApiSource, ServiceAccessConfig as InputServiceAccessConfig,
    ServiceJobSeeds, ServiceOrganizationRole as InputServiceOrganizationRole,
    ServiceVisibility as InputServiceVisibility, TimeoutConfig as InputTimeoutConfig,
};
use skiff_compiler_projection::{
    context::{
        PackageApiSourceProjection, ProjectedPackageDependency, ServiceProjectionContextInput,
    },
    package_unit_artifacts::ProjectedPackageIrArtifacts,
    prelude::PreludeProjection,
    runtime_manifest_model::{RuntimeServiceOrganizationRole, RuntimeServiceVisibility},
    ServiceAccessProjectionConfig, ServiceProjectionContext, TimeoutProjectionConfig,
};
use skiff_compiler_projection_input::PackageProjectionInput;

pub(crate) struct ServiceProjectionContextSeed {
    service_id: String,
    service_target_component: String,
    access: ServiceAccessProjectionConfig,
    timeout: TimeoutProjectionConfig,
    publication_api_has_entries: bool,
    service_version: String,
    service_http_response_max_bytes: Option<u64>,
}

pub(crate) fn service_projection_context_seed_from_service_job_seeds(
    seeds: &ServiceJobSeeds,
) -> ServiceProjectionContextSeed {
    ServiceProjectionContextSeed {
        service_id: seeds.service_id.clone(),
        service_target_component: seeds.service_target_component.clone(),
        access: service_access_projection_config(&seeds.access),
        timeout: timeout_projection_config(&seeds.timeout),
        publication_api_has_entries: seeds.publication_api_has_entries,
        service_version: seeds.service_version.clone(),
        service_http_response_max_bytes: seeds.service_http_response_max_bytes,
    }
}

pub(crate) fn service_projection_context_from_job<'a>(
    seed: &'a ServiceProjectionContextSeed,
    package_publications: &'a [PackageProjectionInput],
    package_artifacts: &'a [ProjectedPackageIrArtifacts],
    package_dependencies: &'a [PackageDependency],
    api_source: Option<&'a PublicationApiSource>,
    prelude: &'a PreludeProjection,
) -> ServiceProjectionContext<'a> {
    ServiceProjectionContext::new(ServiceProjectionContextInput {
        service_id: &seed.service_id,
        service_target_component: &seed.service_target_component,
        access: &seed.access,
        timeout: &seed.timeout,
        publication_api_has_entries: seed.publication_api_has_entries,
        websocket_gateway: None,
        service_version: &seed.service_version,
        service_http_response_max_bytes: seed.service_http_response_max_bytes,
        package_publications,
        package_artifacts,
        package_dependencies: package_dependencies
            .iter()
            .map(projected_package_dependency_from_input)
            .collect(),
        api_source: api_source.map(package_api_source_projection_from_input),
        prelude,
    })
}

pub(crate) fn projected_package_dependency_from_input(
    dependency: &PackageDependency,
) -> ProjectedPackageDependency {
    ProjectedPackageDependency {
        id: dependency.id.clone(),
        version: dependency.version.clone(),
        alias: dependency.alias.clone(),
        config: dependency.config.clone(),
        collection_name_mapping: dependency.collection_name_mapping.clone(),
    }
}

fn package_api_source_projection_from_input(
    source: &PublicationApiSource,
) -> PackageApiSourceProjection {
    PackageApiSourceProjection {
        relative_path: source.relative_path.clone(),
        content_hash: source.content_hash.clone(),
    }
}

fn service_access_projection_config(
    access: &InputServiceAccessConfig,
) -> ServiceAccessProjectionConfig {
    ServiceAccessProjectionConfig {
        visibility: match access.visibility {
            InputServiceVisibility::Public => RuntimeServiceVisibility::Public,
            InputServiceVisibility::Internal => RuntimeServiceVisibility::Internal,
        },
        organization_role: access.organization_role.map(|role| match role {
            InputServiceOrganizationRole::Viewer => RuntimeServiceOrganizationRole::Viewer,
            InputServiceOrganizationRole::Maintainer => RuntimeServiceOrganizationRole::Maintainer,
            InputServiceOrganizationRole::Owner => RuntimeServiceOrganizationRole::Owner,
        }),
    }
}

fn timeout_projection_config(timeout: &InputTimeoutConfig) -> TimeoutProjectionConfig {
    TimeoutProjectionConfig {
        default: timeout.default,
        methods: timeout.methods.clone(),
    }
}
