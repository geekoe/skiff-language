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

pub(super) fn package_unit_path(package_path: &str, unit_hash: &str) -> String {
    format!("units/packages/{package_path}/{unit_hash}.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_compiler_core::artifact::{PackageUnit, PACKAGE_UNIT_SCHEMA_VERSION};
    use skiff_compiler_core::json_utils::value_sha256;
    use skiff_compiler_emission::identity::{
        FILE_IR_IDENTITY_PREFIX, PACKAGE_BUILD_IDENTITY_PREFIX,
    };
    use skiff_compiler_projection::typed_artifacts::{
        assign_package_unit_identities, package_abi_identity, package_build_identity,
        publication_abi_identity,
    };

    #[test]
    fn package_unit_identity_modes_share_unit_json_but_not_artifact_hash_path() {
        let mut base = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
        base.schema_version = PACKAGE_UNIT_SCHEMA_VERSION.to_string();
        base.files.push(FileIrRef {
            file_ir_identity: format!("{FILE_IR_IDENTITY_PREFIX}:file"),
            module_path: "api".to_string(),
            artifact_path: Some("units/files/file.json".to_string()),
            source_ast_hash: Some("sha256:source".to_string()),
        });

        let mut service_mode = base.clone();
        assign_package_unit_identities(&mut service_mode);
        let service_value = serde_json::to_value(&service_mode).expect("unit must serialize");
        let service_hash = value_sha256(&service_value);
        let service_path = package_unit_path("example~com~~pkg", &service_hash);

        let mut package_test_mode = base;
        package_test_mode.publication_abi.publication_id = package_test_mode.package_id.clone();
        package_test_mode.publication_abi.version = package_test_mode.version.clone();
        package_test_mode.publication_abi.abi_identity =
            publication_abi_identity(&package_test_mode.publication_abi);
        package_test_mode.abi_identity = package_abi_identity(&package_test_mode);
        package_test_mode.build_identity = package_build_identity(&package_test_mode);
        let package_test_hash = package_test_mode
            .build_identity
            .strip_prefix(&format!("{PACKAGE_BUILD_IDENTITY_PREFIX}:"))
            .expect("package build identity prefix should be stable");
        let package_test_value =
            serde_json::to_value(&package_test_mode).expect("unit must serialize");
        let package_test_path = package_unit_path("example~com~~pkg", package_test_hash);

        assert_eq!(
            service_value, package_test_value,
            "both modes currently produce the same path-bearing unit JSON"
        );
        assert_eq!(
            service_mode.build_identity, package_test_mode.build_identity,
            "both modes currently produce the same package build identity"
        );
        assert_ne!(
            service_hash, package_test_hash,
            "service test writer hashes serialized unit JSON, while package-test writer uses the build identity hash"
        );
        assert_ne!(
            service_path, package_test_path,
            "package unit artifact paths intentionally follow their distinct hash modes"
        );
    }
}
