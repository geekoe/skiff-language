use thiserror::Error;

use skiff_artifact_model::{ContractOperationId, PackageSchemaTypeId};

#[derive(Debug, Error)]
pub enum ContractDependencyError {
    #[error("failed to read ServiceContract dependency {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse ServiceContract dependency {label}: {source}")]
    Parse {
        label: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("contract dependency alias `{alias}` must match [a-z][A-Za-z0-9_]* and must not be reserved")]
    InvalidAlias { alias: String },
    #[error("contract dependency alias `{alias}` is declared more than once")]
    DuplicateAlias { alias: String },
    #[error("ServiceContract dependency `{alias}` failed canonical validation: {source}")]
    InvalidContract {
        alias: String,
        #[source]
        source: skiff_artifact_identity::ArtifactIdentityError,
    },
    #[error("ServiceContract dependency `{alias}` package schema records failed canonical validation: {source}")]
    InvalidSchemaRecords {
        alias: String,
        #[source]
        source: skiff_artifact_identity::ArtifactIdentityError,
    },
    #[error(
        "ServiceContract dependency `{alias}` requires Package schema owner `{package_id}` without a validated public schema bundle"
    )]
    MissingPackageSchema { alias: String, package_id: String },
    #[error(
        "ServiceContract dependency `{alias}` repeats package schema type identity {package_schema_type_id} across owners"
    )]
    DuplicateSchemaTypeId {
        alias: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error(
        "ServiceContract dependency `{alias}` package schema record set has {actual} records, expected exactly {expected}"
    )]
    SchemaRecordSetMismatch {
        alias: String,
        expected: usize,
        actual: usize,
    },
    #[error(
        "ServiceContract dependency `{alias}` package schema requirements are not the exact operation-reachable closure"
    )]
    SchemaReachabilityMismatch { alias: String },
    #[error(
        "ServiceContract dependency `{alias}` is missing package schema record {package_id}:{package_schema_type_id}"
    )]
    MissingSchemaRecord {
        alias: String,
        package_id: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error(
        "ServiceContract dependency `{alias}` package schema record {package_schema_type_id} owner is {actual_package_id}, expected {expected_package_id}"
    )]
    SchemaRecordOwnerMismatch {
        alias: String,
        package_schema_type_id: PackageSchemaTypeId,
        expected_package_id: String,
        actual_package_id: String,
    },
    #[error(
        "ServiceContract dependency `{alias}` package schema reference {package_schema_type_id} does not match its record owner or stable key"
    )]
    SchemaReferenceMismatch {
        alias: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error(
        "ServiceContract dependency `{alias}` coordinates {actual_service_id}@{actual_version} do not match requirement {expected_service_id}@{expected_version}"
    )]
    CoordinateMismatch {
        alias: String,
        expected_service_id: String,
        expected_version: String,
        actual_service_id: String,
        actual_version: String,
    },
    #[error(
        "ServiceContract dependency `{alias}` protocol identity {actual} does not match expected identity {expected}"
    )]
    ProtocolIdentityMismatch {
        alias: String,
        expected: String,
        actual: String,
    },
    #[error("unknown contract dependency alias `{alias}`")]
    UnknownAlias { alias: String },
    #[error("contract dependency `{alias}` has no operation {operation_id}")]
    UnknownOperation {
        alias: String,
        operation_id: ContractOperationId,
    },
    #[error("contract dependency `{alias}` has no operation stable key `{stable_key}`")]
    UnknownOperationStableKey { alias: String, stable_key: String },
    #[error("contract dependency `{alias}` has no package schema type {package_schema_type_id}")]
    UnknownType {
        alias: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    #[error("contract dependency `{alias}` has no contract type stable key `{stable_key}`")]
    UnknownTypeStableKey { alias: String, stable_key: String },
    #[error(
        "contract dependency `{alias}` type stable key `{stable_key}` is not a public Package type {package_schema_type_id}"
    )]
    ContractTypeNotPublicNameable {
        alias: String,
        stable_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
}
