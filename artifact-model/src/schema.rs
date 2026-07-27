pub const SERVICE_ASSEMBLY_SCHEMA_VERSION: &str = "skiff-assembly-v1";
pub const SERVICE_ASSEMBLY_KIND: &str = "service";
pub const PACKAGE_ASSEMBLY_KIND: &str = "package";
pub const PACKAGE_TEST_ASSEMBLY_SCHEMA_VERSION: &str = "skiff-package-test-assembly-v1";
pub const PACKAGE_TEST_ASSEMBLY_KIND: &str = "packageTest";
pub const PACKAGE_TEST_ENTRYPOINT_KIND: &str = "testOnly";
pub const BUNDLE_SCHEMA_VERSION: &str = "skiff-bundle-v1";
pub const ARTIFACT_INDEX_SCHEMA_VERSION: &str = "skiff-artifact-index-v1";
pub const CONTRACT_SCHEMA_ARTIFACT_VERSION: &str = "skiff-contract-schema-v1";
pub const FILE_IR_SCHEMA_VERSION: &str = "skiff-file-ir-v8";
pub const FILE_IR_FORMAT_VERSION: &str = "skiff-file-ir-format-v6";
pub const FILE_IR_OPCODE_TABLE_VERSION: &str = "skiff-opcode-table-v1";
pub const PUBLICATION_ABI_UNIT_SCHEMA_VERSION: &str = "skiff-publication-abi-unit-v1";
pub const PACKAGE_UNIT_SCHEMA_VERSION: &str = "skiff-package-unit-v2";
pub const SERVICE_UNIT_SCHEMA_VERSION: &str = "skiff-service-unit-v1";
pub const PACKAGE_ARTIFACT_SCHEMA_VERSION: &str = "skiff-package-artifact-v9";
pub const SERVICE_CONTRACT_SCHEMA_VERSION: &str = "skiff-service-contract-v5";
pub const SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION: &str = "skiff-service-contract-definition-v4";
pub const SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION: &str = "skiff-service-deployment-input-v4";
pub const SERVICE_DEPLOYMENT_SCHEMA_VERSION: &str = "skiff-service-deployment-v3";
pub const RUNTIME_ASSEMBLY_SCHEMA_VERSION: &str = "skiff-runtime-assembly-v2";
pub const SERVICE_VERSION_POINTER_SCHEMA_VERSION: &str = "skiff-service-version-pointer-v1";
pub const SERVICE_BUILD_SCHEMA_VERSION: &str = "skiff-service-build-v1";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspension_schema_generations_are_atomic_and_unrelated_domains_remain_stable() {
        assert_eq!(FILE_IR_SCHEMA_VERSION, "skiff-file-ir-v8");
        assert_eq!(FILE_IR_FORMAT_VERSION, "skiff-file-ir-format-v6");
        assert_eq!(
            PUBLICATION_ABI_UNIT_SCHEMA_VERSION,
            "skiff-publication-abi-unit-v1"
        );
        assert_eq!(PACKAGE_UNIT_SCHEMA_VERSION, "skiff-package-unit-v2");
        assert_eq!(PACKAGE_ARTIFACT_SCHEMA_VERSION, "skiff-package-artifact-v9");
        assert_eq!(SERVICE_CONTRACT_SCHEMA_VERSION, "skiff-service-contract-v5");
        assert_eq!(
            SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION,
            "skiff-service-contract-definition-v4"
        );
        assert_eq!(
            SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
            "skiff-service-deployment-input-v4"
        );
        assert_eq!(
            SERVICE_DEPLOYMENT_SCHEMA_VERSION,
            "skiff-service-deployment-v3"
        );
        assert_eq!(RUNTIME_ASSEMBLY_SCHEMA_VERSION, "skiff-runtime-assembly-v2");

        for legacy in [
            "skiff-file-ir-v7",
            "skiff-file-ir-format-v5",
            "skiff-file-ir-v6",
            "skiff-file-ir-format-v4",
            "skiff-package-unit-v1",
            "skiff-package-artifact-v8",
            "skiff-package-artifact-v7",
            "skiff-package-artifact-v5",
            "skiff-package-artifact-v6",
            "skiff-package-artifact-v4",
            "skiff-service-contract-v4",
            "skiff-service-contract-v3",
            "skiff-service-contract-definition-v3",
            "skiff-service-contract-definition-v2",
            "skiff-service-deployment-input-v2",
            "skiff-service-deployment-input-v3",
            "skiff-service-deployment-v2",
        ] {
            assert!(![
                FILE_IR_SCHEMA_VERSION,
                FILE_IR_FORMAT_VERSION,
                PACKAGE_UNIT_SCHEMA_VERSION,
                PACKAGE_ARTIFACT_SCHEMA_VERSION,
                SERVICE_CONTRACT_SCHEMA_VERSION,
                SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION,
                SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
                SERVICE_DEPLOYMENT_SCHEMA_VERSION,
                RUNTIME_ASSEMBLY_SCHEMA_VERSION,
            ]
            .contains(&legacy));
        }
    }
}
