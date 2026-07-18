#![allow(dead_code)]

use skiff_compiler::{
    compile_contract, ContractDefinitionError, ContractRequirement,
    PackageContractCompileDependency, ServiceContract, ServiceContractDefinition,
};

/// Compiles an explicit, code-free contract definition through the public
/// compiler pipeline. This fixture never reads provider source or service
/// configuration and therefore cannot infer or fabricate a contract.
pub fn compile_service_contract(
    definition: ServiceContractDefinition,
) -> Result<ServiceContract, ContractDefinitionError> {
    compile_contract(definition)
}

/// Binds an already validated contract to the source alias used by one package.
/// All protocol coordinates remain owned by the contract artifact.
pub fn package_contract_dependency(
    alias: impl Into<String>,
    contract: ServiceContract,
) -> PackageContractCompileDependency {
    let requirement = ContractRequirement {
        alias: alias.into(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    PackageContractCompileDependency {
        requirement,
        contract,
    }
}
