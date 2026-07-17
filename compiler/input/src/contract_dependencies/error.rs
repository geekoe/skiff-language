use thiserror::Error;

use skiff_artifact_model::{ContractOperationId, ContractTypeId};

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
    #[error("contract dependency `{alias}` has no contract type {contract_type_id}")]
    UnknownType {
        alias: String,
        contract_type_id: ContractTypeId,
    },
    #[error("contract dependency `{alias}` has no contract type stable key `{stable_key}`")]
    UnknownTypeStableKey { alias: String, stable_key: String },
}
