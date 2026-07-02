use std::{collections::BTreeMap, path::Path};

use crate::{
    error::InputAssemblyError,
    package_config::{
        resolve_package_imports, PackageConfigError, PackageDependency, PackageManifest,
        PackageManifestKey,
    },
    package_job::{build_package_jobs, RawPackagePublicationJob},
    service_config::ServiceConfig,
    service_ingress::ServiceIngressSeed,
    ResolvedPackageGraph,
};

pub type PackageManifestDiscoveryResult =
    Result<BTreeMap<PackageManifestKey, PackageManifest>, PackageConfigError>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceSourcePackageFacts {
    pub imports: Vec<Vec<String>>,
    pub references_std_package_types: bool,
}

pub struct ResolvedServicePackages {
    pub package_graph: ResolvedPackageGraph,
    pub package_jobs: Vec<RawPackagePublicationJob>,
    pub package_manifests: BTreeMap<PackageManifestKey, PackageManifest>,
}

pub fn resolve_service_packages(
    config: &ServiceConfig,
    service_ingress: &ServiceIngressSeed,
    source_root: &Path,
    source_facts: &ServiceSourcePackageFacts,
    discover_manifests: impl FnOnce(&Path, &[PackageDependency]) -> PackageManifestDiscoveryResult,
) -> Result<ResolvedServicePackages, InputAssemblyError> {
    let package_manifests = discover_manifests(source_root, &config.publication.dependencies)?;
    let mut package_imports = source_facts.imports.clone();
    if service_ingress.has_runtime_ingress() {
        package_imports.push(vec!["std".to_string()]);
    }
    if source_facts.references_std_package_types {
        package_imports.push(vec!["std".to_string()]);
    }
    let packages = resolve_package_imports(
        &package_imports,
        &config.publication.dependencies,
        &package_manifests,
    )?;
    let package_jobs = build_package_jobs(packages)?;

    Ok(ResolvedServicePackages {
        package_graph: ResolvedPackageGraph::declared_only(config.publication.dependencies.clone()),
        package_jobs,
        package_manifests,
    })
}
