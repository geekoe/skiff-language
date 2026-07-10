use std::sync::Arc;

use skiff_artifact_model::{FileIrUnit, PackageUnit, ServiceUnit};
use skiff_runtime_linked_program::PublicationResourceTable;

#[derive(Debug, Clone)]
pub struct ArtifactGraph {
    pub service_unit: Arc<ServiceUnit>,
    pub service_files: Vec<Arc<FileIrUnit>>,
    pub package_units: Vec<Arc<PackageUnit>>,
    pub package_files: Vec<Vec<Arc<FileIrUnit>>>,
    pub service_resources: PublicationResourceTable,
    pub package_resources: Vec<PublicationResourceTable>,
    pub identities: ArtifactGraphIdentities,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactGraphIdentities {
    pub service_file_ir_identities: Vec<String>,
    pub package_build_identities: Vec<String>,
    pub package_file_ir_identities: Vec<Vec<String>>,
}

impl ArtifactGraphIdentities {
    pub fn from_loaded_units(
        service_files: &[Arc<FileIrUnit>],
        package_units: &[Arc<PackageUnit>],
        package_files: &[Vec<Arc<FileIrUnit>>],
    ) -> Self {
        Self {
            service_file_ir_identities: service_files
                .iter()
                .map(|file| file.file_ir_identity.clone())
                .collect(),
            package_build_identities: package_units
                .iter()
                .map(|package| package.build_identity.clone())
                .collect(),
            package_file_ir_identities: package_files
                .iter()
                .map(|files| {
                    files
                        .iter()
                        .map(|file| file.file_ir_identity.clone())
                        .collect()
                })
                .collect(),
        }
    }
}
