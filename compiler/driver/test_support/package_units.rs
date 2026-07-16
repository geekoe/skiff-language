use crate::emission::artifact::PublishedFileIrArtifact;
use skiff_compiler_core::artifact::FileIrRef;

pub(super) fn file_ref_for_published(artifact: &PublishedFileIrArtifact) -> FileIrRef {
    FileIrRef {
        file_ir_identity: artifact.identity.clone(),
        module_path: artifact.module_path.clone(),
        artifact_path: Some(artifact.path.clone()),
        source_ast_hash: Some(artifact.unit.source_ast_hash.clone()),
    }
}
