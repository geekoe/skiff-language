mod error;
mod index;
mod reader;
mod strict_json;

pub use error::ContractDependencyError;
pub use index::ContractDependencyIndex;
pub use reader::{
    read_contract_dependency, read_contract_dependency_json, ResolvedContractDependency,
};

#[cfg(test)]
mod tests;
