use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skiff_artifact_model::ServiceUnit;
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_input_model::RawPublicationSourceGraph;

use crate::{
    assemble_publication,
    error::InputAssemblyError,
    package_config::{
        package_alias_bindings, PackageDependency, PackageManifest, PackageManifestKey,
    },
    package_job::RawPackagePublicationJob,
    service_config::{ServiceAccessConfig, ServiceConfig, TimeoutConfig},
    service_dependencies::resolve_service_dependencies,
    service_ingress::{service_ingress_seed_from_config, ServiceIngressSeed},
    service_packages::{
        resolve_service_packages, PackageManifestDiscoveryResult, ServiceSourcePackageFacts,
    },
    source_tree::SourceTree,
    RawPublication, ResolvedServiceDependencies,
};

pub struct RawServicePublicationJob {
    pub publication: RawPublication,
    pub package_aliases: BTreeMap<String, Vec<String>>,
    pub service_id: String,
    pub service_dependencies: ResolvedServiceDependencies,
    pub service_ingress: ServiceIngressSeed,
    pub package_jobs: Vec<RawPackagePublicationJob>,
    pub seeds: ServiceJobSeeds,
}

pub struct ServiceJobSeeds {
    pub service_id: String,
    pub service_target_component: String,
    pub package_manifests: BTreeMap<PackageManifestKey, PackageManifest>,
    pub access: ServiceAccessConfig,
    pub timeout: TimeoutConfig,
    pub publication_api_has_entries: bool,
    pub service_version: String,
    pub service_http_response_max_bytes: Option<u64>,
}

pub fn build_service_job(
    config: &ServiceConfig,
    source_tree: &SourceTree,
    service_id_override: Option<&str>,
    raw_source_graph: RawPublicationSourceGraph,
    source_package_facts: &ServiceSourcePackageFacts,
    service_dependency_artifact_roots: &[PathBuf],
    build_id_for_root: impl Fn(&Path, &ServiceUnit) -> Result<String, String>,
    discover_manifests: impl FnOnce(&Path, &[PackageDependency]) -> PackageManifestDiscoveryResult,
) -> Result<RawServicePublicationJob, InputAssemblyError> {
    let service_publication_id =
        PublicationId::parse(service_id_override.unwrap_or(config.publication.id.as_str()))
            .map_err(|error| InputAssemblyError::InvalidServiceId {
                service_id: service_id_override
                    .unwrap_or(config.publication.id.as_str())
                    .to_string(),
                message: error.to_string(),
            })?;
    let service_id = service_publication_id.as_str();
    let service_target_component = service_publication_id.runtime_target_component();
    let service_ingress = service_ingress_seed_from_config(config);
    let resolved_packages = resolve_service_packages(
        config,
        &service_ingress,
        &source_tree.root,
        source_package_facts,
        discover_manifests,
    )?;
    let publication = assemble_publication(
        config.publication.clone(),
        source_tree.clone(),
        raw_source_graph,
        resolved_packages.package_graph,
    );
    let package_jobs = resolved_packages.package_jobs;
    let package_manifests = resolved_packages.package_manifests;
    let package_aliases =
        package_alias_bindings(&config.publication.dependencies, &package_manifests);
    let service_dependencies = resolve_service_dependencies(
        &config.runtime.services,
        service_dependency_artifact_roots,
        build_id_for_root,
    )?;

    let seeds = ServiceJobSeeds {
        service_id: service_id.to_string(),
        service_target_component,
        package_manifests,
        access: config.access.clone(),
        timeout: config.runtime.timeout.clone(),
        publication_api_has_entries: !config.publication.api.is_empty(),
        service_version: config.publication.version.clone(),
        service_http_response_max_bytes: service_http_response_max_bytes(config),
    };

    Ok(RawServicePublicationJob {
        publication,
        package_aliases,
        service_id: service_id.to_string(),
        service_dependencies,
        service_ingress,
        package_jobs,
        seeds,
    })
}

fn service_http_response_max_bytes(config: &ServiceConfig) -> Option<u64> {
    config
        .runtime
        .http
        .as_ref()
        .and_then(|http| http.response.as_ref())
        .and_then(|response| response.max_bytes)
}
