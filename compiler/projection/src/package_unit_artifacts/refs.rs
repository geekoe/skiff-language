use skiff_artifact_model::{FileIrRef, PublicationResourceRef};
use skiff_compiler_projection_input::PublicationResourceProjectionInput;

use super::{PackageFileIrProjection, ProjectedPublicationResource};

pub fn file_ir_refs_for_projected(artifacts: &[PackageFileIrProjection]) -> Vec<FileIrRef> {
    artifacts
        .iter()
        .map(|artifact| FileIrRef {
            file_ir_identity: artifact.identity.clone(),
            module_path: artifact.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(artifact.source_ast_hash.clone()),
        })
        .collect()
}

pub fn projected_publication_resources(
    resources: &[PublicationResourceProjectionInput],
) -> Vec<ProjectedPublicationResource> {
    resources
        .iter()
        .map(|resource| ProjectedPublicationResource {
            path: resource.path().to_string(),
            absolute_path: resource.absolute_path().to_path_buf(),
            byte_len: resource.byte_len(),
            sha256: resource.sha256().to_string(),
            content_type: resource.content_type().map(str::to_string),
        })
        .collect()
}

pub fn resource_refs_for_projected(
    resources: &[ProjectedPublicationResource],
) -> Vec<PublicationResourceRef> {
    resources
        .iter()
        .map(|resource| PublicationResourceRef {
            path: resource.path.clone(),
            sha256: resource.sha256.clone(),
            byte_len: resource.byte_len,
            content_type: resource.content_type.clone(),
            artifact_path: None,
        })
        .collect()
}
