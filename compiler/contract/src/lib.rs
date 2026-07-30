mod compile;
mod definition;
mod error;
mod projection;
mod selection;

pub use compile::{compile_service_contract_definition, definition_contract_operation_id};
pub use definition::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};
pub use error::{ContractDefinitionError, Result};
pub use projection::{
    project_package_api_visibility, project_service_api, ServiceApiFunction,
    ServiceApiFunctionStatus, ServiceApiProjection, ServiceApiVisibility,
};

#[cfg(test)]
mod tests;
