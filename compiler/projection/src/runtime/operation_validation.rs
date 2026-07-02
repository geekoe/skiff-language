use crate::{contract::ContractProjection, error::ProjectionError};

pub fn validate_runtime_operation_modes(
    _contract: &ContractProjection,
) -> Result<(), ProjectionError> {
    Ok(())
}
