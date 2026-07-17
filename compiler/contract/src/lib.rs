mod compile;
mod definition;
mod error;

pub use compile::{
    compile_service_contract_definition, definition_contract_operation_id,
    definition_contract_type_id, definition_contract_type_ref,
};
pub use definition::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};
pub use error::{ContractDefinitionError, Result};

#[cfg(test)]
mod tests;
