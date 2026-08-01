pub const FILE_IR_IDENTITY_PREFIX: &str = "skiff-file-ir-v11:sha256";
pub const ACTOR_ABI_IDENTITY_SCHEMA_MARKER: &str = "skiff-actor-abi-identity-v1";
pub const ACTOR_ABI_IDENTITY_PREFIX: &str = "skiff-actor-abi-v1:sha256";
pub const ACTOR_METHOD_IDENTITY_SCHEMA_MARKER: &str = "skiff-actor-method-identity-v1";
pub const ACTOR_METHOD_IDENTITY_PREFIX: &str = "skiff-actor-method-v1:sha256";
pub const ACTOR_IMPLEMENTATION_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-actor-implementation-identity-v1";
pub const ACTOR_IMPLEMENTATION_IDENTITY_PREFIX: &str = "skiff-actor-implementation-v1:sha256";
pub const PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-package-artifact-build-identity-v9";
pub const PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-package-artifact-local-abi-identity-v6";
pub const PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX: &str = "skiff-package-build-v10:sha256";
pub const PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX: &str = "skiff-package-local-abi-v7:sha256";
pub const PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-package-schema-type-identity-v2";
pub const PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX: &str = "skiff-package-schema-type-v2:sha256";
pub const PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-package-schema-index-identity-v1";
pub const PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX: &str = "skiff-package-schema-index-v1:sha256";
pub const CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER: &str = "skiff-contract-operation-identity-v1";
pub const CONTRACT_OPERATION_IDENTITY_PREFIX: &str = "skiff-contract-operation-v1:sha256";
pub const GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER: &str = "skiff-gateway-entry-identity-v2";
pub const GATEWAY_ENTRY_IDENTITY_PREFIX: &str = skiff_artifact_model::GATEWAY_ENTRY_IDENTITY_PREFIX;
pub const SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER: &str = "skiff-service-protocol-identity-v5";
pub const SERVICE_PROTOCOL_IDENTITY_PREFIX: &str = "skiff-service-protocol-v5:sha256";
pub const DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER: &str =
    "skiff-deployment-artifact-identity-v4";
pub const DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX: &str = "skiff-deployment-artifact-v4:sha256";
pub const ASSEMBLY_IDENTITY_SCHEMA_MARKER: &str = "skiff-runtime-assembly-identity-v3";
pub use skiff_artifact_model::RUNTIME_ASSEMBLY_IDENTITY_PREFIX as ASSEMBLY_IDENTITY_PREFIX;
