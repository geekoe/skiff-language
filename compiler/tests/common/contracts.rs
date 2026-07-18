#![allow(dead_code)]

use skiff_compiler::{
    compile_contract, ContractDefinitionError, ServiceContract, ServiceContractDefinition,
};

/// Compiles an explicit, code-free contract definition through the public
/// compiler pipeline. This fixture never reads provider source or service
/// configuration and therefore cannot infer or fabricate a contract.
pub fn compile_service_contract(
    definition: ServiceContractDefinition,
) -> Result<ServiceContract, ContractDefinitionError> {
    compile_contract(definition)
}
