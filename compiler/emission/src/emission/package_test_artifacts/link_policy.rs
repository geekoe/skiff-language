use std::collections::BTreeMap;

use serde_json::{json, Value};
use skiff_compiler_core::artifact::{
    PackageDependencyPublicLinkScope, PackageProductionLinkScope, PackageTestEntrypoint,
    PackageTestFileIrRef, PackageTestFileLinkScope, PackageTestLinkPolicy,
    PackageTestPackageUnitRef, PackageUnit,
};
use skiff_compiler_core::json_utils::value_sha256;

use super::PublishedFileIrArtifact;

pub(super) fn link_policy(
    production_ref: &PackageTestPackageUnitRef,
    package_unit: &PackageUnit,
    test_file_artifacts: &[PublishedFileIrArtifact],
    test_file_refs: &[PackageTestFileIrRef],
    entrypoints: &[PackageTestEntrypoint],
    dependency_public_scopes: &[PackageDependencyPublicLinkScope],
) -> PackageTestLinkPolicy {
    let mut entrypoint_ids_by_file = BTreeMap::<String, Vec<String>>::new();
    for entrypoint in entrypoints {
        entrypoint_ids_by_file
            .entry(entrypoint.owner_test_file.file_ir_identity.clone())
            .or_default()
            .push(entrypoint.entrypoint_local_id.clone());
    }
    for ids in entrypoint_ids_by_file.values_mut() {
        ids.sort();
        ids.dedup();
    }

    PackageTestLinkPolicy {
        current_package_production: PackageProductionLinkScope {
            package_id: production_ref.package_id.clone(),
            version: production_ref.version.clone(),
            build_identity: production_ref.build_identity.clone(),
            files_digest: value_sha256(
                &serde_json::to_value(&package_unit.files).expect("files must serialize"),
            ),
            implementation_links_digest: value_sha256(
                &serde_json::to_value(&package_unit.implementation_links)
                    .expect("implementation links must serialize"),
            ),
            allow_private: true,
        },
        test_file_scopes: test_file_refs
            .iter()
            .zip(test_file_artifacts)
            .map(|(file_ref, file)| {
                let entrypoint_local_ids = entrypoint_ids_by_file
                    .get(&file_ref.file_ir_identity)
                    .cloned()
                    .unwrap_or_default();
                PackageTestFileLinkScope {
                    owner_test_file_identity: file_ref.file_ir_identity.clone(),
                    source_path: file_ref.source_path.clone(),
                    module_path: file_ref.module_path.clone(),
                    allowed_local_link_digest: package_test_allowed_local_link_digest(
                        file_ref,
                        &file.unit,
                        &entrypoint_local_ids,
                    ),
                    entrypoint_local_ids,
                }
            })
            .collect(),
        dependency_public_scopes: dependency_public_scopes.to_vec(),
    }
}

fn package_test_allowed_local_link_digest(
    file_ref: &PackageTestFileIrRef,
    file: &skiff_compiler_core::artifact::FileIrUnit,
    entrypoint_local_ids: &[String],
) -> String {
    let mut entrypoint_local_ids = entrypoint_local_ids.to_vec();
    entrypoint_local_ids.sort();
    entrypoint_local_ids.dedup();
    value_sha256(&json!({
        "fileIrIdentity": file_ref.file_ir_identity,
        "sourcePath": file_ref.source_path,
        "modulePath": file_ref.module_path,
        "entrypointLocalIds": entrypoint_local_ids,
        "localTargets": {
            "declarations": &file.declarations,
            "linkTargets": &file.link_targets,
            "typeCount": file.type_table.len(),
            "constCount": file.constants.len(),
            "executableCount": file.executables.len(),
        },
    }))
}

pub(super) fn source_map(test_files: &[PackageTestFileIrRef]) -> Value {
    json!({
        "sources": test_files
            .iter()
            .map(|file| {
                json!({
                    "sourcePath": file.source_path,
                    "modulePath": file.module_path,
                    "fileIrIdentity": file.file_ir_identity,
                })
            })
            .collect::<Vec<_>>()
    })
}
