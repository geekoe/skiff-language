use crate::{
    error::ProjectionError,
    {contract::ContractProjection, TimeoutProjectionConfig},
};

pub fn validate_timeout_targets(
    timeout: &TimeoutProjectionConfig,
    contract: &ContractProjection,
) -> Result<(), ProjectionError> {
    let operation_names = contract.operation_names();
    let mut violations = Vec::new();
    for operation in timeout.methods.keys() {
        if !operation_names.contains(operation) {
            violations.push(format!(
                "timeout.methods references unknown service operation {operation}"
            ));
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    violations.sort();
    Err(ProjectionError::ContractValidation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}
