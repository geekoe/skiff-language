use super::*;

mod deployment;
mod file_ir;
mod framing;
mod gateway;
mod semantic;

#[test]
fn current_identity_generations_are_atomic() {
    assert_eq!(
        skiff_artifact_model::FILE_IR_SCHEMA_VERSION,
        "skiff-file-ir-v13"
    );
    assert_eq!(
        skiff_artifact_model::FILE_IR_FORMAT_VERSION,
        "skiff-file-ir-format-v7"
    );
    assert_eq!(
        skiff_artifact_model::FILE_IR_OPCODE_TABLE_VERSION,
        "skiff-opcode-table-v2"
    );
    assert_eq!(FILE_IR_IDENTITY_PREFIX, "skiff-file-ir-v13:sha256");
    assert_eq!(
        BYTECODE_IDENTITY_SCHEMA_MARKER,
        "skiff-bytecode-artifact-v4"
    );
    assert_eq!(BYTECODE_IDENTITY_PREFIX, "skiff-bytecode-image-v4:sha256");
    assert_eq!(
        skiff_artifact_model::BYTECODE_SCHEMA_VERSION,
        "skiff-bytecode-v6"
    );
    assert_eq!(
        skiff_artifact_model::BYTECODE_ISA_VERSION,
        "skiff-bytecode-isa-v4"
    );
    assert_eq!(
        skiff_artifact_model::bytecode::opcodes::OPCODE_CONTRACT_FORMAT,
        2
    );
    assert_eq!(
        skiff_artifact_model::BYTECODE_STATEMENT_MANIFEST_SCHEMA_MARKER,
        "skiff-bytecode-statement-manifest-v1"
    );
    assert_eq!(
        skiff_artifact_model::BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX,
        "skiff-bytecode-statement-manifest-v1:sha256"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
        "skiff-package-artifact-build-identity-v12"
    );
    assert_eq!(
        skiff_artifact_model::PACKAGE_ARTIFACT_SCHEMA_VERSION,
        "skiff-package-artifact-v14"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        "skiff-package-build-v13:sha256"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER,
        "skiff-package-artifact-local-abi-identity-v6"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
        "skiff-package-local-abi-v7:sha256"
    );
    assert_eq!(
        PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER,
        "skiff-package-schema-type-identity-v2"
    );
    assert_eq!(
        PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX,
        "skiff-package-schema-type-v2:sha256"
    );
    assert_eq!(
        PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER,
        "skiff-package-schema-index-identity-v1"
    );
    assert_eq!(
        PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX,
        "skiff-package-schema-index-v1:sha256"
    );
    assert_eq!(
        CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER,
        "skiff-contract-operation-identity-v1"
    );
    assert_eq!(
        CONTRACT_OPERATION_IDENTITY_PREFIX,
        "skiff-contract-operation-v1:sha256"
    );
    assert_eq!(
        SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
        "skiff-service-protocol-identity-v6"
    );
    assert_eq!(
        skiff_artifact_model::SERVICE_CONTRACT_SCHEMA_VERSION,
        "skiff-service-contract-v6"
    );
    assert_eq!(
        SERVICE_PROTOCOL_IDENTITY_PREFIX,
        "skiff-service-protocol-v6:sha256"
    );
    assert_eq!(
        DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER,
        "skiff-deployment-artifact-identity-v4"
    );
    assert_eq!(
        DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
        "skiff-deployment-artifact-v4:sha256"
    );
    assert_eq!(
        ASSEMBLY_IDENTITY_SCHEMA_MARKER,
        "skiff-runtime-assembly-identity-v3"
    );
    assert_eq!(ASSEMBLY_IDENTITY_PREFIX, "skiff-runtime-assembly-v3:sha256");
}

#[test]
fn service_protocol_identity_hash_accepts_only_canonical_v6_identity() {
    let hash = "a".repeat(64);
    let identity = format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{hash}");
    assert_eq!(
        service_protocol_identity_hash(&identity).expect("canonical identity"),
        hash
    );

    for invalid in [
        format!("skiff-protocol-v1:sha256:{hash}"),
        format!("skiff-service-protocol-v5:sha256:{hash}"),
        SERVICE_PROTOCOL_IDENTITY_PREFIX.to_string(),
        format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}", "a".repeat(63)),
        format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}", "A".repeat(64)),
        format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{}", "g".repeat(64)),
    ] {
        assert!(
            service_protocol_identity_hash(&invalid).is_err(),
            "{invalid} must be rejected"
        );
    }
}
