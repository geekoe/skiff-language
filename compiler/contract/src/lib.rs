mod compile;
mod definition;
mod error;
mod projection;
mod public_instances;
mod selection;

pub use compile::{compile_service_contract_definition, definition_contract_operation_id};
pub use definition::{ServiceContractDefinition, ServiceContractDefinitionDiagnosticText};
pub use error::{ContractDefinitionError, Result};
pub use projection::{
    project_package_api_visibility, project_service_api,
    project_service_api_with_public_instance_operations, ServiceApiFunction,
    ServiceApiFunctionStatus, ServiceApiProjection, ServiceApiVisibility,
};
pub use public_instances::{
    ServicePublicInstanceInterfaceOperations, ServicePublicInstanceOperationFacts,
    ServicePublicInstanceOperationSlot,
};

#[cfg(test)]
mod tests;
