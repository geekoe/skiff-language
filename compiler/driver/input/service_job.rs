use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    input::{
        package_job::{package_publication_job_from_raw, PackagePublicationJob},
        service_dependencies::dynamic_build_id,
        service_packages::{
            service_source_package_facts_from_compiler_sources, PackageManifestDiscoveryResult,
        },
        source_graph::{publication_from_raw_with_source_graph, PublicationSourceGraph},
        PackageDependency, PackageManifest, PackageManifestKey, Publication,
        ResolvedServiceDependencies, ServiceAccessConfig, ServiceConfig, ServiceIngressSeed,
        SourceTree, TimeoutConfig,
    },
    shared::publication_error::PublicationError,
};

pub(crate) struct ServicePublicationJob {
    pub(crate) publication: Publication,
    pub(crate) package_aliases: BTreeMap<String, Vec<String>>,
    pub(crate) service_id: String,
    pub(crate) service_dependencies: ResolvedServiceDependencies,
    pub(crate) service_ingress: ServiceIngressSeed,
    pub(crate) package_jobs: Vec<PackagePublicationJob>,
    pub(crate) seeds: ServiceJobSeeds,
}

pub(crate) struct ServiceJobSeeds {
    pub(crate) service_id: String,
    pub(crate) service_target_component: String,
    pub(crate) package_manifests: BTreeMap<PackageManifestKey, PackageManifest>,
    pub(crate) access: ServiceAccessConfig,
    pub(crate) timeout: TimeoutConfig,
    pub(crate) publication_api_has_entries: bool,
    pub(crate) service_version: String,
    pub(crate) service_http_response_max_bytes: Option<u64>,
}

pub(crate) fn build_service_job(
    config: &ServiceConfig,
    source_tree: &SourceTree,
    service_id_override: Option<&str>,
    service_dependency_artifact_roots: &[PathBuf],
    discover_manifests: impl FnOnce(&Path, &[PackageDependency]) -> PackageManifestDiscoveryResult,
) -> Result<ServicePublicationJob, PublicationError> {
    let raw_source_graph = skiff_compiler_input::read_publication_sources(source_tree)?;
    let user_source_graph =
        PublicationSourceGraph::parse_raw_publication_sources(&raw_source_graph)?;
    let user_production_sources = user_source_graph.production_files();
    let source_package_facts =
        service_source_package_facts_from_compiler_sources(&user_production_sources);
    let raw_job = skiff_compiler_input::build_service_job(
        config,
        source_tree,
        service_id_override,
        raw_source_graph,
        &source_package_facts,
        service_dependency_artifact_roots,
        dynamic_build_id,
        discover_manifests,
    )?;
    let package_jobs = raw_job
        .package_jobs
        .into_iter()
        .map(package_publication_job_from_raw)
        .collect::<Result<Vec<_>, _>>()?;
    let publication =
        publication_from_raw_with_source_graph(raw_job.publication, user_source_graph);

    Ok(ServicePublicationJob {
        publication,
        package_aliases: raw_job.package_aliases,
        service_id: raw_job.service_id,
        service_dependencies: raw_job.service_dependencies,
        service_ingress: raw_job.service_ingress,
        package_jobs,
        seeds: ServiceJobSeeds::from(raw_job.seeds),
    })
}

impl From<skiff_compiler_input::ServiceJobSeeds> for ServiceJobSeeds {
    fn from(seeds: skiff_compiler_input::ServiceJobSeeds) -> Self {
        Self {
            service_id: seeds.service_id,
            service_target_component: seeds.service_target_component,
            package_manifests: seeds.package_manifests,
            access: seeds.access,
            timeout: seeds.timeout,
            publication_api_has_entries: seeds.publication_api_has_entries,
            service_version: seeds.service_version,
            service_http_response_max_bytes: seeds.service_http_response_max_bytes,
        }
    }
}
