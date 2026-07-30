use super::*;

mod deployment;
mod file_ir;
mod framing;
mod gateway;
mod semantic;

#[test]
fn current_identity_generations_are_atomic() {
    assert_eq!(FILE_IR_IDENTITY_PREFIX, "skiff-file-ir-v11:sha256");
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
        "skiff-package-artifact-build-identity-v8"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        "skiff-package-build-v10:sha256"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER,
        "skiff-package-artifact-local-abi-identity-v5"
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
        "skiff-service-protocol-identity-v5"
    );
    assert_eq!(
        SERVICE_PROTOCOL_IDENTITY_PREFIX,
        "skiff-service-protocol-v5:sha256"
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
fn service_protocol_identity_hash_accepts_only_canonical_v5_identity() {
    let hash = "a".repeat(64);
    let identity = format!("{SERVICE_PROTOCOL_IDENTITY_PREFIX}:{hash}");
    assert_eq!(
        service_protocol_identity_hash(&identity).expect("canonical identity"),
        hash
    );

    for invalid in [
        format!("skiff-protocol-v1:sha256:{hash}"),
        format!("skiff-service-protocol-v4:sha256:{hash}"),
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
