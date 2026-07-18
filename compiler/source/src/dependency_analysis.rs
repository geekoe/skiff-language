use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryOperationDescriptor, CallableSemanticFacts, ContractRequirement, ContractTypeId,
    PackageCallableId, PackageLocalAbiIdentity, ServiceContract,
};
use skiff_compiler_input::{
    ContractDependencyError, ContractDependencyIndex, ResolvedContractDependency,
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourceDependencyAnalysisError {
    #[error("package dependency alias `{alias}` is declared more than once")]
    DuplicatePackageAlias { alias: String },
    #[error("contract dependency alias `{alias}` is declared more than once")]
    DuplicateContractAlias { alias: String },
    #[error("dependency alias `{alias}` is declared by both a package and a contract")]
    AliasKindConflict { alias: String },
    #[error("invalid validated contract dependency facts: {message}")]
    InvalidContractFacts { message: String },
}

/// Canonical dependency facts made available to source call-target and effect
/// analysis. This input is intentionally independent from legacy publication
/// ABI and provider/deployment artifacts.
#[derive(Debug, Clone, Default)]
pub struct SourceDependencyAnalysisInput {
    packages: BTreeMap<String, PackageDependencyAnalysisFacts>,
    contracts: ContractDependencyIndex,
}

#[derive(Debug, Clone)]
pub struct PackageDependencyAnalysisFacts {
    expected_local_abi: PackageLocalAbiIdentity,
    callables: BTreeMap<String, PackageDependencyCallableAnalysis>,
}

#[derive(Debug, Clone)]
pub struct PackageDependencyCallableAnalysis {
    callable_id: PackageCallableId,
    semantic_facts: CallableSemanticFacts,
}

pub(crate) enum ResolvedDependencyAnalysisTarget<'a> {
    Package {
        alias: String,
        expected_local_abi: &'a PackageLocalAbiIdentity,
        callable: &'a PackageDependencyCallableAnalysis,
    },
    Contract {
        requirement: &'a ContractRequirement,
        operation: &'a BoundaryOperationDescriptor,
    },
    UnknownContractMember {
        alias: String,
        stable_key: Option<String>,
    },
    MissingMember,
    Missing,
}

impl SourceDependencyAnalysisInput {
    /// Freezes the package/contract alias namespace before source analysis.
    /// Both inputs remain iterators so duplicates cannot be hidden by an
    /// eager map construction at a caller.
    pub fn new(
        packages: impl IntoIterator<Item = (String, PackageDependencyAnalysisFacts)>,
        contracts: impl IntoIterator<Item = ResolvedContractDependency>,
    ) -> Result<Self, SourceDependencyAnalysisError> {
        let mut package_index = BTreeMap::new();
        for (alias, facts) in packages {
            if package_index.insert(alias.clone(), facts).is_some() {
                return Err(SourceDependencyAnalysisError::DuplicatePackageAlias { alias });
            }
        }
        let contracts = ContractDependencyIndex::build(contracts).map_err(|error| match error {
            ContractDependencyError::DuplicateAlias { alias } => {
                SourceDependencyAnalysisError::DuplicateContractAlias { alias }
            }
            error => SourceDependencyAnalysisError::InvalidContractFacts {
                message: error.to_string(),
            },
        })?;
        if let Some(alias) = contracts
            .dependencies()
            .map(|dependency| dependency.requirement().alias.as_str())
            .find(|alias| package_index.contains_key(*alias))
        {
            return Err(SourceDependencyAnalysisError::AliasKindConflict {
                alias: alias.to_string(),
            });
        }
        Ok(Self {
            packages: package_index,
            contracts,
        })
    }

    /// Resolves both dependency kinds through the namespace frozen by `new`.
    pub(crate) fn resolve_path(&self, path: &str) -> ResolvedDependencyAnalysisTarget<'_> {
        let Some((alias, callable_path)) = path.split_once('.') else {
            return if self.packages.contains_key(path) {
                ResolvedDependencyAnalysisTarget::MissingMember
            } else if self.contracts.requirement(path).is_ok() {
                ResolvedDependencyAnalysisTarget::UnknownContractMember {
                    alias: path.to_string(),
                    stable_key: None,
                }
            } else {
                ResolvedDependencyAnalysisTarget::Missing
            };
        };
        if let Some(facts) = self.packages.get(alias) {
            return match facts.callables.get(callable_path) {
                Some(callable) => ResolvedDependencyAnalysisTarget::Package {
                    alias: alias.to_string(),
                    expected_local_abi: &facts.expected_local_abi,
                    callable,
                },
                None => ResolvedDependencyAnalysisTarget::MissingMember,
            };
        }
        let Ok(requirement) = self.contracts.requirement(alias) else {
            return ResolvedDependencyAnalysisTarget::Missing;
        };
        match self.contracts.operation_by_stable_key(alias, callable_path) {
            Ok(operation) => ResolvedDependencyAnalysisTarget::Contract {
                requirement,
                operation,
            },
            Err(_) => ResolvedDependencyAnalysisTarget::UnknownContractMember {
                alias: alias.to_string(),
                stable_key: Some(callable_path.to_string()),
            },
        }
    }

    pub fn contract_requirement(
        &self,
        alias: &str,
    ) -> Result<&ContractRequirement, ContractDependencyError> {
        self.contracts.requirement(alias)
    }

    pub fn contract(&self, alias: &str) -> Result<&ServiceContract, ContractDependencyError> {
        self.contracts.contract(alias)
    }

    pub fn contract_operation_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&BoundaryOperationDescriptor, ContractDependencyError> {
        self.contracts.operation_by_stable_key(alias, stable_key)
    }

    pub fn public_contract_type_id_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&ContractTypeId, ContractDependencyError> {
        self.contracts
            .public_contract_type_id_by_stable_key(alias, stable_key)
    }

    pub fn contract_dependencies(&self) -> &ContractDependencyIndex {
        &self.contracts
    }

    pub(crate) fn package_callable(
        &self,
        alias: &str,
        expected_local_abi: &PackageLocalAbiIdentity,
        callable_id: &PackageCallableId,
    ) -> Option<&PackageDependencyCallableAnalysis> {
        let facts = self.packages.get(alias)?;
        if &facts.expected_local_abi != expected_local_abi {
            return None;
        }
        let mut matches = facts
            .callables
            .values()
            .filter(|callable| &callable.callable_id == callable_id);
        let callable = matches.next()?;
        matches.next().is_none().then_some(callable)
    }

    pub(crate) fn package_aliases(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(String::as_str)
    }

    pub(crate) fn contract_aliases(&self) -> impl Iterator<Item = &str> {
        self.contracts
            .dependencies()
            .map(|dependency| dependency.requirement().alias.as_str())
    }
}

impl PackageDependencyAnalysisFacts {
    pub fn new(
        expected_local_abi: PackageLocalAbiIdentity,
        callables: BTreeMap<String, PackageDependencyCallableAnalysis>,
    ) -> Self {
        Self {
            expected_local_abi,
            callables,
        }
    }
}

impl PackageDependencyCallableAnalysis {
    pub fn new(callable_id: PackageCallableId, semantic_facts: CallableSemanticFacts) -> Self {
        Self {
            callable_id,
            semantic_facts,
        }
    }

    pub fn callable_id(&self) -> &PackageCallableId {
        &self.callable_id
    }

    pub fn semantic_facts(&self) -> &CallableSemanticFacts {
        &self.semantic_facts
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_identity::{contract_operation_id, contract_type_id};
    use skiff_artifact_model::{
        CallableEffectSummary, CallableProvenanceSummary, CallableSemanticFacts,
    };

    use crate::contract_dependency_test_fixture::{contract_fixture, requirement};

    use super::*;

    fn package_callable(id: &str) -> PackageDependencyCallableAnalysis {
        PackageDependencyCallableAnalysis::new(
            PackageCallableId::new(id),
            CallableSemanticFacts {
                effects: CallableEffectSummary::analysis_pending(),
                provenance: CallableProvenanceSummary::Unknown {
                    reason: skiff_artifact_model::CallableProvenanceUnknownReason::AnalysisPending,
                },
                resolved_call_targets: BTreeMap::new(),
            },
        )
    }

    #[test]
    fn canonical_contract_facts_preserve_requirement_descriptor_and_public_nominal_type() {
        let contract = contract_fixture("example.svc", "1.0.0", "run", "payload", "payloadClosure");
        let expected_requirement = requirement("svc", &contract);
        let input =
            SourceDependencyAnalysisInput::new(
                [("pkg".to_string(), package_facts("abi:pkg", "callable:run"))],
                [ResolvedContractDependency::validated(
                    expected_requirement.clone(),
                    contract.clone(),
                )
                .unwrap()],
            )
            .unwrap();

        assert_eq!(
            input.contract_requirement("svc").unwrap(),
            &expected_requirement
        );
        assert_eq!(input.contract("svc").unwrap(), &contract);
        let operation = input
            .contract_operation_by_stable_key("svc", "run")
            .unwrap();
        assert_eq!(
            operation.operation_id,
            contract_operation_id("example.svc", "1.0.0", "run").unwrap()
        );
        assert_eq!(
            input
                .public_contract_type_id_by_stable_key("svc", "payload")
                .unwrap(),
            &contract_type_id("example.svc", "1.0.0", "payload").unwrap()
        );
        assert!(matches!(
            input.resolve_path("pkg.run"),
            ResolvedDependencyAnalysisTarget::Package { .. }
        ));
        assert!(matches!(
            input.resolve_path("svc.run"),
            ResolvedDependencyAnalysisTarget::Contract { .. }
        ));
        assert!(matches!(
            input.contract_operation_by_stable_key("missing", "run"),
            Err(ContractDependencyError::UnknownAlias { .. })
        ));
        assert!(matches!(
            input.contract_operation_by_stable_key("svc", "missing"),
            Err(ContractDependencyError::UnknownOperationStableKey { .. })
        ));
        assert!(matches!(
            input.resolve_path("missing.run"),
            ResolvedDependencyAnalysisTarget::Missing
        ));
        assert!(matches!(
            input.resolve_path("pkg.missing"),
            ResolvedDependencyAnalysisTarget::MissingMember
        ));
        assert!(matches!(
            input.resolve_path("svc.missing"),
            ResolvedDependencyAnalysisTarget::UnknownContractMember {
                alias,
                stable_key: Some(stable_key),
            } if alias == "svc" && stable_key == "missing"
        ));
        assert!(matches!(
            input.resolve_path("svc"),
            ResolvedDependencyAnalysisTarget::UnknownContractMember {
                alias,
                stable_key: None,
            } if alias == "svc"
        ));
        assert!(matches!(
            input.public_contract_type_id_by_stable_key("svc", "payloadClosure"),
            Err(ContractDependencyError::ContractTypeNotPublicNameable { .. })
        ));
    }

    #[test]
    fn constructor_rejects_duplicates_and_cross_kind_alias_conflicts() {
        let package = || package_facts("abi:pkg", "callable:run");
        assert!(matches!(
            SourceDependencyAnalysisInput::new(
                [("dup".to_string(), package()), ("dup".to_string(), package())],
                Vec::new(),
            ),
            Err(SourceDependencyAnalysisError::DuplicatePackageAlias { alias }) if alias == "dup"
        ));

        let first = contract_fixture("example.first", "1.0.0", "run", "payload", "payloadClosure");
        let second = contract_fixture(
            "example.second",
            "1.0.0",
            "run",
            "payload",
            "payloadClosure",
        );
        assert!(matches!(
            SourceDependencyAnalysisInput::new(
                Vec::new(),
                [
                    ResolvedContractDependency::validated(requirement("dup", &first), first)
                        .unwrap(),
                    ResolvedContractDependency::validated(requirement("dup", &second), second)
                        .unwrap(),
                ],
            ),
            Err(SourceDependencyAnalysisError::DuplicateContractAlias { alias }) if alias == "dup"
        ));

        let contract = contract_fixture(
            "example.conflict",
            "1.0.0",
            "run",
            "payload",
            "payloadClosure",
        );
        assert!(matches!(
            SourceDependencyAnalysisInput::new(
                [("same".to_string(), package())],
                [ResolvedContractDependency::validated(
                    requirement("same", &contract),
                    contract,
                )
                .unwrap()],
            ),
            Err(SourceDependencyAnalysisError::AliasKindConflict { alias }) if alias == "same"
        ));
    }

    #[test]
    fn canonical_package_lookup_rejects_identity_mismatch() {
        let input = SourceDependencyAnalysisInput::new(
            BTreeMap::from([(
                "pkg".to_string(),
                PackageDependencyAnalysisFacts::new(
                    PackageLocalAbiIdentity::new("abi:pkg"),
                    BTreeMap::from([("run".to_string(), package_callable("callable:run"))]),
                ),
            )]),
            Vec::new(),
        )
        .unwrap();
        assert!(input
            .package_callable(
                "pkg",
                &PackageLocalAbiIdentity::new("abi:pkg"),
                &PackageCallableId::new("callable:run"),
            )
            .is_some());
        assert!(input
            .package_callable(
                "pkg",
                &PackageLocalAbiIdentity::new("abi:stale"),
                &PackageCallableId::new("callable:run"),
            )
            .is_none());
    }

    #[test]
    fn canonical_package_lookup_rejects_duplicate_callable_identity() {
        let input = SourceDependencyAnalysisInput::new(
            BTreeMap::from([(
                "pkg".to_string(),
                PackageDependencyAnalysisFacts::new(
                    PackageLocalAbiIdentity::new("abi:pkg"),
                    BTreeMap::from([
                        ("first".to_string(), package_callable("callable:duplicate")),
                        ("second".to_string(), package_callable("callable:duplicate")),
                    ]),
                ),
            )]),
            Vec::new(),
        )
        .unwrap();
        assert!(input
            .package_callable(
                "pkg",
                &PackageLocalAbiIdentity::new("abi:pkg"),
                &PackageCallableId::new("callable:duplicate"),
            )
            .is_none());
    }

    fn package_facts(abi: &str, callable: &str) -> PackageDependencyAnalysisFacts {
        PackageDependencyAnalysisFacts::new(
            PackageLocalAbiIdentity::new(abi),
            BTreeMap::from([("run".to_string(), package_callable(callable))]),
        )
    }
}
