use std::{fs, path::Path};

use skiff_artifact_identity::validate_service_contract_identities;
use skiff_artifact_model::{ContractRequirement, ServiceContract};
use skiff_compiler_input_model::{is_reserved_source_import_alias, is_valid_source_import_alias};

use super::{strict_json::StrictJsonValue, ContractDependencyError};

/// A ServiceContract that has crossed the compiler input trust boundary.
/// Construction is private to the canonical validation routines below.
#[derive(Debug, Clone)]
pub struct ResolvedContractDependency {
    requirement: ContractRequirement,
    contract: ServiceContract,
}

impl ResolvedContractDependency {
    pub fn validated(
        requirement: ContractRequirement,
        contract: ServiceContract,
    ) -> Result<Self, ContractDependencyError> {
        validate_alias(&requirement.alias)?;
        validate_service_contract_identities(&contract).map_err(|source| {
            ContractDependencyError::InvalidContract {
                alias: requirement.alias.clone(),
                source,
            }
        })?;
        if contract.service_id != requirement.service_id
            || contract.contract_version != requirement.contract_version
        {
            return Err(ContractDependencyError::CoordinateMismatch {
                alias: requirement.alias,
                expected_service_id: requirement.service_id,
                expected_version: requirement.contract_version,
                actual_service_id: contract.service_id,
                actual_version: contract.contract_version,
            });
        }
        if contract.service_protocol_identity != requirement.expected_protocol_identity {
            return Err(ContractDependencyError::ProtocolIdentityMismatch {
                alias: requirement.alias,
                expected: requirement.expected_protocol_identity.to_string(),
                actual: contract.service_protocol_identity.to_string(),
            });
        }
        Ok(Self {
            requirement,
            contract,
        })
    }

    pub fn requirement(&self) -> &ContractRequirement {
        &self.requirement
    }

    pub fn contract(&self) -> &ServiceContract {
        &self.contract
    }
}

/// Reads and validates a published ServiceContract. The reader recomputes and
/// validates canonical identities; it never overwrites an untrusted declared
/// identity with an assigned value.
pub fn read_contract_dependency(
    path: &Path,
    requirement: ContractRequirement,
) -> Result<ResolvedContractDependency, ContractDependencyError> {
    let bytes = fs::read(path).map_err(|source| ContractDependencyError::Read {
        path: path.display().to_string(),
        source,
    })?;
    read_contract_dependency_json(path.display().to_string(), &bytes, requirement)
}

pub fn read_contract_dependency_json(
    label: impl Into<String>,
    bytes: &[u8],
    requirement: ContractRequirement,
) -> Result<ResolvedContractDependency, ContractDependencyError> {
    let label = label.into();
    let value = serde_json::from_slice::<StrictJsonValue>(bytes)
        .map_err(|source| ContractDependencyError::Parse {
            label: label.clone(),
            source,
        })?
        .into_inner();
    let contract = serde_json::from_value::<ServiceContract>(value).map_err(|source| {
        ContractDependencyError::Parse {
            label: label.clone(),
            source,
        }
    })?;
    ResolvedContractDependency::validated(requirement, contract)
}

fn validate_alias(alias: &str) -> Result<(), ContractDependencyError> {
    if !is_valid_source_import_alias(alias) || is_reserved_source_import_alias(alias) {
        return Err(ContractDependencyError::InvalidAlias {
            alias: alias.to_string(),
        });
    }
    Ok(())
}
