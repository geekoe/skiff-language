mod compile;
mod definition;
mod error;
mod projection;

pub use compile::{
    compile_service_contract_definition, definition_contract_operation_id,
    definition_contract_type_id, definition_contract_type_ref,
};
pub use definition::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};
pub use error::{ContractDefinitionError, Result};
pub use projection::{project_service_api, ServiceApiProjection};

#[cfg(test)]
mod tests;
