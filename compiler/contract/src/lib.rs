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
pub use projection::{
    canonicalize_service_owned_operation_contract, project_package_api_visibility,
    project_service_api, ServiceApiFunction, ServiceApiFunctionStatus, ServiceApiProjection,
    ServiceApiVisibility,
};

#[cfg(test)]
mod tests;
