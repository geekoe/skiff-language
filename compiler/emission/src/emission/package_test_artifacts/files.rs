use std::collections::BTreeMap;

use skiff_compiler_core::artifact::PackageTestFileIrRef;

use super::{PackageTestArtifactBuildError, PackageTestFileIrArtifact, PublishedFileIrArtifact};
use crate::emission::file_ir_artifacts::published_file_ir_artifact_from_unit;
use crate::emission::identity::file_ir_identity;

pub(super) fn published_files(
    files: Vec<PublishedFileIrArtifact>,
) -> Result<Vec<PublishedFileIrArtifact>, PackageTestArtifactBuildError> {
    files
        .into_iter()
        .map(|file| {
            let identity = file_ir_identity(&file.unit)?;
            if file.unit.file_ir_identity != identity || file.identity != identity {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "production file {} identity metadata does not match its File IR payload",
                        file.source_path
                    ),
                });
            }
            if file.path.trim().is_empty() {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "production file {} artifact path is empty",
                        file.source_path
                    ),
                });
            }
            Ok(file)
        })
        .collect()
}

pub(super) fn published_test_files(
    files: Vec<PackageTestFileIrArtifact>,
) -> Result<Vec<PublishedFileIrArtifact>, PackageTestArtifactBuildError> {
    files
        .into_iter()
        .map(|file| {
            let mut unit = file.file_ir;
            let identity = file_ir_identity(&unit)?;
            unit.file_ir_identity = identity;
            Ok(published_file_ir_artifact_from_unit(
                &unit,
                file.source_path,
                file.module_path,
                "package-test".to_string(),
            ))
        })
        .collect()
}

pub(super) fn package_test_file_ref(file: &PublishedFileIrArtifact) -> PackageTestFileIrRef {
    PackageTestFileIrRef {
        file_ir_identity: file.identity.clone(),
        file_ir_path: file.path.clone(),
        source_path: file.source_path.clone(),
        module_path: file.module_path.clone(),
    }
}

pub(super) fn test_files_by_source_path(
    files: &[PackageTestFileIrRef],
) -> Result<BTreeMap<String, PackageTestFileIrRef>, PackageTestArtifactBuildError> {
    let mut by_source_path = BTreeMap::new();
    for file in files {
        if by_source_path
            .insert(file.source_path.clone(), file.clone())
            .is_some()
        {
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "duplicate package test file source path {}",
                    file.source_path
                ),
            });
        }
    }
    Ok(by_source_path)
}
