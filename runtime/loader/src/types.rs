use std::path::PathBuf;

#[derive(Debug)]
pub struct ArtifactIndexPointer {
    pub service_id: String,
    pub service_version: Option<String>,
    pub build_id: String,
    pub contract_identity: Option<String>,
    pub implementation_identity: Option<String>,
    pub service_unit_path: Option<PathBuf>,
    pub service_assembly: ServiceAssemblyPointer,
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

#[derive(Debug)]
pub struct ServiceAssemblyPointer {
    pub path: PathBuf,
    pub assembly_identity: Option<String>,
}
