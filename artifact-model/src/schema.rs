pub const FILE_IR_SCHEMA_VERSION: &str = "skiff-file-ir-v11";
pub const FILE_IR_FORMAT_VERSION: &str = "skiff-file-ir-format-v7";
pub const FILE_IR_OPCODE_TABLE_VERSION: &str = "skiff-opcode-table-v2";
pub const PACKAGE_ARTIFACT_SCHEMA_VERSION: &str = "skiff-package-artifact-v9";
pub const SERVICE_CONTRACT_SCHEMA_VERSION: &str = "skiff-service-contract-v5";
pub const SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION: &str = "skiff-service-contract-definition-v4";
pub const SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION: &str = "skiff-service-deployment-input-v5";
pub const SERVICE_DEPLOYMENT_SCHEMA_VERSION: &str = "skiff-service-deployment-v4";
pub const RUNTIME_ASSEMBLY_SCHEMA_VERSION: &str = "skiff-runtime-assembly-v3";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspension_schema_generations_are_atomic_and_unrelated_domains_remain_stable() {
        assert_eq!(FILE_IR_SCHEMA_VERSION, "skiff-file-ir-v11");
        assert_eq!(FILE_IR_FORMAT_VERSION, "skiff-file-ir-format-v7");
        assert_eq!(FILE_IR_OPCODE_TABLE_VERSION, "skiff-opcode-table-v2");
        assert_eq!(PACKAGE_ARTIFACT_SCHEMA_VERSION, "skiff-package-artifact-v9");
        assert_eq!(SERVICE_CONTRACT_SCHEMA_VERSION, "skiff-service-contract-v5");
        assert_eq!(
            SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION,
            "skiff-service-contract-definition-v4"
        );
        assert_eq!(
            SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
            "skiff-service-deployment-input-v5"
        );
        assert_eq!(
            SERVICE_DEPLOYMENT_SCHEMA_VERSION,
            "skiff-service-deployment-v4"
        );
        assert_eq!(RUNTIME_ASSEMBLY_SCHEMA_VERSION, "skiff-runtime-assembly-v3");

        for legacy in [
            "skiff-file-ir-v9",
            "skiff-file-ir-v8",
            "skiff-file-ir-format-v6",
            "skiff-opcode-table-v1",
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
            "skiff-service-deployment-input-v4",
            "skiff-service-deployment-v2",
            "skiff-service-deployment-v3",
            "skiff-runtime-assembly-v2",
        ] {
            assert!(![
                FILE_IR_SCHEMA_VERSION,
                FILE_IR_FORMAT_VERSION,
                FILE_IR_OPCODE_TABLE_VERSION,
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
