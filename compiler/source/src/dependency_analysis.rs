use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryOperationDescriptor, CallableSemanticFacts, ContractOperationId, ContractRequirement,
    PackageCallableId, PackageCallableSignature, PackageLocalAbiIdentity, PackageSchemaTypeRecord,
    ServiceContract,
};
use skiff_compiler_input::{
    ContractDependencyError, ContractDependencyIndex, ResolvedContractDependency,
};
use thiserror::Error;

use crate::shared::ast_utils::dependency_source_address_parts;

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
    schema_records: BTreeMap<String, skiff_artifact_model::PackageSchemaTypeRecord>,
}

#[derive(Debug, Clone)]
pub struct PackageDependencyCallableAnalysis {
    callable_id: PackageCallableId,
    semantic_facts: CallableSemanticFacts,
    signature: Option<PackageCallableSignature>,
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
        let Some((alias, callable_path)) = dependency_source_address_parts(path) else {
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

    pub(crate) fn exact_contract_operation(
        &self,
        requirement: &ContractRequirement,
        operation_id: &ContractOperationId,
    ) -> Option<&BoundaryOperationDescriptor> {
        let indexed_requirement = self.contracts.requirement(&requirement.alias).ok()?;
        if indexed_requirement != requirement {
            return None;
        }
        self.contracts
            .operation(&requirement.alias, operation_id)
            .ok()
    }

    pub fn public_package_type_by_stable_key(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Result<&PackageSchemaTypeRecord, ContractDependencyError> {
        self.contracts
            .public_package_type_by_stable_key(alias, stable_key)
    }

    pub fn contract_dependencies(&self) -> &ContractDependencyIndex {
        &self.contracts
    }

    pub fn package_type_by_owner_and_stable_key(
        &self,
        package_id: &str,
        stable_key: &str,
    ) -> Option<&PackageSchemaTypeRecord> {
        self.contracts
            .package_type_by_owner_and_stable_key(package_id, stable_key)
            .or_else(|| {
                self.packages
                    .values()
                    .flat_map(|facts| facts.schema_records.values())
                    .find(|record| {
                        record.package_id == package_id
                            && record.stable_schema_key == stable_key
                    })
            })
    }

    pub fn direct_package_type(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Option<&PackageSchemaTypeRecord> {
        self.packages.get(alias)?.schema_records.get(stable_key)
    }

    pub fn exact_package_type(
        &self,
        package_id: &str,
        stable_key: &str,
        type_id: &skiff_artifact_model::PackageSchemaTypeId,
    ) -> Option<&PackageSchemaTypeRecord> {
        let contract = self
            .contracts
            .package_type_by_owner_and_stable_key(package_id, stable_key);
        let direct = self
            .packages
            .values()
            .flat_map(|facts| facts.schema_records.values())
            .find(|record| {
                record.package_id == package_id && record.stable_schema_key == stable_key
            });
        contract
            .into_iter()
            .chain(direct)
            .find(|record| &record.package_schema_type_id == type_id)
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

    pub(crate) fn package_callable_by_source_path(
        &self,
        path: &str,
    ) -> Option<&PackageDependencyCallableAnalysis> {
        let (alias, public_path) =
            dependency_source_address_parts(path).or_else(|| path.split_once('.'))?;
        self.packages.get(alias)?.callables.get(public_path)
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
            schema_records: BTreeMap::new(),
        }
    }

    pub fn with_schema_records(
        mut self,
        records: impl IntoIterator<Item = skiff_artifact_model::PackageSchemaTypeRecord>,
    ) -> Self {
        self.schema_records = records
            .into_iter()
            .map(|record| (record.stable_schema_key.clone(), record))
            .collect();
        self
    }

    pub fn with_schema_bindings(
        mut self,
        records: impl IntoIterator<Item = (String, skiff_artifact_model::PackageSchemaTypeRecord)>,
    ) -> Self {
        self.schema_records = records.into_iter().collect();
        self
    }
}

impl PackageDependencyCallableAnalysis {
    pub fn new(callable_id: PackageCallableId, semantic_facts: CallableSemanticFacts) -> Self {
        Self {
            callable_id,
            semantic_facts,
            signature: None,
        }
    }

    pub fn with_signature(mut self, signature: PackageCallableSignature) -> Self {
        self.signature = Some(signature);
        self
    }

    pub fn callable_id(&self) -> &PackageCallableId {
        &self.callable_id
    }

    pub fn semantic_facts(&self) -> &CallableSemanticFacts {
        &self.semantic_facts
    }

    pub(crate) fn signature(&self) -> Option<&PackageCallableSignature> {
        self.signature.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_identity::contract_operation_id;
    use skiff_artifact_model::{
        CallableEffectSummary, CallableProvenanceSummary, CallableSemanticFacts,
    };

    use crate::contract_dependency_test_fixture::resolved_contract_fixture;

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
        let dependency =
            resolved_contract_fixture("svc", "example.svc", "run", "payload", "payloadClosure");
        let contract = dependency.contract().clone();
        let expected_requirement = dependency.requirement().clone();
        let input = SourceDependencyAnalysisInput::new(
            [(
                "pkg".to_string(),
                PackageDependencyAnalysisFacts::new(
                    PackageLocalAbiIdentity::new("abi:pkg"),
                    BTreeMap::from([
                        ("run".to_string(), package_callable("callable:run")),
                        (
                            "nested.run".to_string(),
                            package_callable("callable:nested-run"),
                        ),
                    ]),
                ),
            )],
            [dependency],
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
                .public_package_type_by_stable_key("svc", "payload")
                .unwrap(),
            input
                .contract_dependencies()
                .public_package_type_by_stable_key("svc", "payload")
                .unwrap()
        );
        assert!(matches!(
            input.resolve_path("pkg/run"),
            ResolvedDependencyAnalysisTarget::Package { .. }
        ));
        assert!(matches!(
            input.resolve_path("svc/run"),
            ResolvedDependencyAnalysisTarget::Contract { .. }
        ));
        assert!(matches!(
            input.resolve_path("pkg/nested.run"),
            ResolvedDependencyAnalysisTarget::Package { .. }
        ));
        assert!(matches!(
            input.resolve_path("pkg.run"),
            ResolvedDependencyAnalysisTarget::Missing
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
            input.resolve_path("missing/run"),
            ResolvedDependencyAnalysisTarget::Missing
        ));
        assert!(matches!(
            input.resolve_path("pkg/missing"),
            ResolvedDependencyAnalysisTarget::MissingMember
        ));
        assert!(matches!(
            input.resolve_path("svc/missing"),
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
        assert!(input
            .public_package_type_by_stable_key("svc", "payloadClosure")
            .is_ok());
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

        let first = resolved_contract_fixture("dup", "example.first", "run", "payload", "result");
        let second = resolved_contract_fixture("dup", "example.second", "run", "payload", "result");
        assert!(matches!(
            SourceDependencyAnalysisInput::new(
                Vec::new(),
                [
                    first,
                    second,
                ],
            ),
            Err(SourceDependencyAnalysisError::DuplicateContractAlias { alias }) if alias == "dup"
        ));

        let dependency =
            resolved_contract_fixture("same", "example.conflict", "run", "payload", "result");
        assert!(matches!(
            SourceDependencyAnalysisInput::new(
                [("same".to_string(), package())],
                [dependency],
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
    fn package_and_service_aliases_select_the_same_package_owned_type() {
        let dependency =
            resolved_contract_fixture("svc", "example.shared", "run", "Payload", "Result");
        let record = dependency
            .schema_records()
            .values()
            .find(|record| record.stable_schema_key == "Payload")
            .unwrap()
            .clone();
        let input = SourceDependencyAnalysisInput::new(
            [(
                "pkg".to_string(),
                PackageDependencyAnalysisFacts::new(
                    PackageLocalAbiIdentity::new("abi"),
                    BTreeMap::new(),
                )
                .with_schema_records([record]),
            )],
            [dependency],
        )
        .unwrap();
        assert_eq!(
            input.direct_package_type("pkg", "Payload"),
            Some(
                input
                    .public_package_type_by_stable_key("svc", "Payload")
                    .unwrap()
            )
        );
    }

    #[test]
    fn exact_contract_lookup_requires_full_requirement_and_operation_identity() {
        let dependency =
            resolved_contract_fixture("svc", "example.exact", "run", "payload", "result");
        let exact_requirement = dependency.requirement().clone();
        let operation_id = contract_operation_id("example.exact", "1.0.0", "run").unwrap();
        let input = SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap();

        assert!(input
            .exact_contract_operation(&exact_requirement, &operation_id)
            .is_some());
        let mut stale_requirement = exact_requirement.clone();
        stale_requirement.contract_version = "0.9.0".to_string();
        assert!(input
            .exact_contract_operation(&stale_requirement, &operation_id)
            .is_none());
        assert!(input
            .exact_contract_operation(
                &exact_requirement,
                &contract_operation_id("example.exact", "1.0.0", "missing").unwrap(),
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
