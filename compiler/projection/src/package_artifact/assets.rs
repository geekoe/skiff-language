use skiff_artifact_model::{FileIrRef, FileIrUnit, PublicationResourceRef};
use skiff_compiler_projection_input::PublicationResourceProjectionInput;

use super::model::ProjectedPackageResource;

pub(super) fn file_ir_refs_from_units(units: &[FileIrUnit]) -> Vec<FileIrRef> {
    let mut refs = units
        .iter()
        .map(|unit| FileIrRef {
            file_ir_identity: unit.file_ir_identity.clone(),
            module_path: unit.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(unit.source_ast_hash.clone()),
        })
        .collect::<Vec<_>>();
    refs.sort_by(|left, right| {
        (&left.file_ir_identity, &left.module_path)
            .cmp(&(&right.file_ir_identity, &right.module_path))
    });
    refs
}

pub(super) fn project_package_resources(
    resources: &[PublicationResourceProjectionInput],
) -> Vec<ProjectedPackageResource> {
    resources
        .iter()
        .map(|resource| ProjectedPackageResource {
            path: resource.path().to_string(),
            absolute_path: resource.absolute_path().to_path_buf(),
            byte_len: resource.byte_len(),
            sha256: resource.sha256().to_string(),
            content_type: resource.content_type().map(str::to_string),
        })
        .collect()
}

pub(super) fn resource_refs_from_projected(
    resources: &[ProjectedPackageResource],
) -> Vec<PublicationResourceRef> {
    let mut refs = resources
        .iter()
        .map(|resource| PublicationResourceRef {
            path: resource.path.clone(),
            sha256: resource.sha256.clone(),
            byte_len: resource.byte_len,
            content_type: resource.content_type.clone(),
            artifact_path: None,
        })
        .collect::<Vec<_>>();
    refs.sort_by(|left, right| left.path.cmp(&right.path));
    refs
}

#[cfg(test)]
mod tests;
