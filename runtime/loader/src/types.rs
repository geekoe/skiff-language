use std::path::PathBuf;

pub use skiff_artifact_identity::{
    PackageUnitArtifactRef as PackageUnitArtifactPointer,
    ServiceAssemblyArtifactRef as ServiceAssemblyPointer,
    ServiceUnitArtifactRef as ServiceUnitArtifactPointer,
};

#[derive(Debug)]
pub struct ArtifactIndexPointer {
    pub service_id: String,
    pub service_version: Option<String>,
    pub build_id: String,
    pub contract_identity: Option<String>,
    pub implementation_identity: Option<String>,
    pub service_unit: ServiceUnitArtifactPointer,
    pub service_assembly: ServiceAssemblyPointer,
    pub package_units: Vec<PackageUnitArtifactPointer>,
}

#[derive(Debug)]
pub struct ArtifactPointerFile {
    pub path: PathBuf,
    pub entry: ArtifactIndexPointer,
}

#[derive(Debug)]
pub struct RootedArtifactPointerFile {
    pub artifact_root: PathBuf,
    pub path: PathBuf,
    pub entry: ArtifactIndexPointer,
}

impl RootedArtifactPointerFile {
    pub fn new(artifact_root: PathBuf, pointer: ArtifactPointerFile) -> Self {
        Self {
            artifact_root,
            path: pointer.path,
            entry: pointer.entry,
        }
    }
}
