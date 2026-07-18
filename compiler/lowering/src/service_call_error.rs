use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceCallLoweringError {
    #[error(
        "typed service calls for contract dependency `{alias}` carry inconsistent ContractRequirement identities"
    )]
    ContractRequirementMismatch { alias: String },
    #[error("package has more service requirements than fit in a u32 binding slot")]
    TooManyServiceRequirements,
    #[error("File IR module `{module_path}` has more distinct service call refs than fit in a u32 owner-local index")]
    TooManyFileServiceCallRefs { module_path: String },
}
