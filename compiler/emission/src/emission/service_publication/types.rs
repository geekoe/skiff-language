use crate::emission::artifact::PublishedServiceArtifacts;
use crate::projection::runtime_manifest_model::SkiffRuntimeManifest;

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltServicePublication {
    pub manifest: SkiffRuntimeManifest,
    pub artifacts: PublishedServiceArtifacts,
}
