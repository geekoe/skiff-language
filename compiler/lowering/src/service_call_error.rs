use thiserror::Error;

use skiff_artifact_model::ContractOperationId;

#[derive(Debug, Error)]
pub enum ServiceCallLoweringError {
    #[error("contract operation index contains an empty dependency alias")]
    EmptyContractAlias,
    #[error("contract operation index contains duplicate dependency alias `{alias}`")]
    DuplicateContractAlias { alias: String },
    #[error(
        "contract dependency `{alias}` operation map key {map_key} does not match nested operation id {nested_id}"
    )]
    OperationIdentityMismatch {
        alias: String,
        map_key: ContractOperationId,
        nested_id: ContractOperationId,
    },
    #[error("service call references unknown contract dependency alias `{alias}`")]
    UnknownContractAlias { alias: String },
    #[error(
        "service call references unknown operation {operation_id} on contract dependency `{alias}`"
    )]
    UnknownContractOperation {
        alias: String,
        operation_id: ContractOperationId,
    },
    #[error(
        "service call for contract dependency `{alias}` expects protocol identity {actual}, but the requirement expects {expected}"
    )]
    ProtocolIdentityMismatch {
        alias: String,
        expected: String,
        actual: String,
    },
    #[error("package has more service requirements than fit in a u32 binding slot")]
    TooManyServiceRequirements,
}
