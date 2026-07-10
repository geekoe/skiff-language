use std::sync::Arc;

use skiff_runtime_loader::ArtifactGraph;

use crate::program::{
    ArtifactFileIrUnit as FileIrUnit, PackageUnit, PublicationResourceTable, ServiceUnit,
};

#[derive(Debug, Clone)]
pub struct LinkerInput {
    pub service: Arc<ServiceUnit>,
    pub service_files: Vec<Arc<FileIrUnit>>,
    pub packages: Vec<Arc<PackageUnit>>,
    pub package_files: Vec<Vec<Arc<FileIrUnit>>>,
    pub service_resources: PublicationResourceTable,
    pub package_resources: Vec<PublicationResourceTable>,
}

impl From<ArtifactGraph> for LinkerInput {
    fn from(graph: ArtifactGraph) -> Self {
        Self {
            service: graph.service_unit,
            service_files: graph.service_files,
            packages: graph.package_units,
            package_files: graph.package_files,
            service_resources: graph.service_resources,
            package_resources: graph.package_resources,
        }
    }
}

impl LinkerInput {
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn from_legacy_parts(
        service: Arc<ServiceUnit>,
        service_files: Vec<Arc<FileIrUnit>>,
        packages: Vec<Arc<PackageUnit>>,
        package_files: Vec<Vec<Arc<FileIrUnit>>>,
    ) -> Self {
        Self {
            service,
            service_files,
            packages,
            package_files,
            service_resources: PublicationResourceTable::default(),
            package_resources: Vec::new(),
        }
    }
}
