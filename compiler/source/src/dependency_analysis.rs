use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableSemanticFacts, ContractOperationId, PackageCallableId, PackageLocalAbiIdentity,
    ServiceProtocolIdentity,
};

/// Canonical dependency facts made available to source call-target and effect
/// analysis. This input is intentionally independent from legacy publication
/// ABI and provider/deployment artifacts.
#[derive(Debug, Clone, Default)]
pub struct SourceDependencyAnalysisInput {
    packages: BTreeMap<String, PackageDependencyAnalysisFacts>,
    contracts: BTreeMap<String, ContractDependencyAnalysisFacts>,
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

#[derive(Debug, Clone)]
pub struct ContractDependencyAnalysisFacts {
    expected_protocol_identity: ServiceProtocolIdentity,
    operations: BTreeMap<String, ContractOperationId>,
}

pub(crate) enum ResolvedDependencyAnalysisTarget<'a> {
    Package {
        alias: String,
        expected_local_abi: &'a PackageLocalAbiIdentity,
        callable: &'a PackageDependencyCallableAnalysis,
    },
    Contract {
        alias: String,
        expected_protocol_identity: &'a ServiceProtocolIdentity,
        operation_id: &'a ContractOperationId,
    },
    Ambiguous,
    MissingMember,
    Missing,
}

impl SourceDependencyAnalysisInput {
    pub fn new(
        packages: BTreeMap<String, PackageDependencyAnalysisFacts>,
        contracts: BTreeMap<String, ContractDependencyAnalysisFacts>,
    ) -> Self {
        Self {
            packages,
            contracts,
        }
    }

    /// Resolves both dependency kinds through one alias/path rule. An alias
    /// present in both namespaces is ambiguous and therefore never guessed.
    pub(crate) fn resolve_path(&self, path: &str) -> ResolvedDependencyAnalysisTarget<'_> {
        let Some((alias, callable_path)) = path.split_once('.') else {
            return match (
                self.packages.contains_key(path),
                self.contracts.contains_key(path),
            ) {
                (true, true) => ResolvedDependencyAnalysisTarget::Ambiguous,
                (true, false) | (false, true) => ResolvedDependencyAnalysisTarget::MissingMember,
                (false, false) => ResolvedDependencyAnalysisTarget::Missing,
            };
        };
        let package = self.packages.get(alias);
        let contract = self.contracts.get(alias);
        match (package, contract) {
            (Some(_), Some(_)) => ResolvedDependencyAnalysisTarget::Ambiguous,
            (Some(facts), None) => match facts.callables.get(callable_path) {
                Some(callable) => ResolvedDependencyAnalysisTarget::Package {
                    alias: alias.to_string(),
                    expected_local_abi: &facts.expected_local_abi,
                    callable,
                },
                None => ResolvedDependencyAnalysisTarget::MissingMember,
            },
            (None, Some(facts)) => match facts.operations.get(callable_path) {
                Some(operation_id) => ResolvedDependencyAnalysisTarget::Contract {
                    alias: alias.to_string(),
                    expected_protocol_identity: &facts.expected_protocol_identity,
                    operation_id,
                },
                None => ResolvedDependencyAnalysisTarget::MissingMember,
            },
            (None, None) => ResolvedDependencyAnalysisTarget::Missing,
        }
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
        self.contracts.keys().map(String::as_str)
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

impl ContractDependencyAnalysisFacts {
    pub fn new(
        expected_protocol_identity: ServiceProtocolIdentity,
        operations: BTreeMap<String, ContractOperationId>,
    ) -> Self {
        Self {
            expected_protocol_identity,
            operations,
        }
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        CallableEffectSummary, CallableProvenanceSummary, CallableSemanticFacts,
    };

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
    fn one_lookup_rule_distinguishes_package_contract_ambiguous_and_missing() {
        let input = SourceDependencyAnalysisInput::new(
            BTreeMap::from([
                (
                    "pkg".to_string(),
                    PackageDependencyAnalysisFacts::new(
                        PackageLocalAbiIdentity::new("abi:pkg"),
                        BTreeMap::from([("run".to_string(), package_callable("callable:run"))]),
                    ),
                ),
                (
                    "both".to_string(),
                    PackageDependencyAnalysisFacts::new(
                        PackageLocalAbiIdentity::new("abi:both"),
                        BTreeMap::from([("run".to_string(), package_callable("callable:both"))]),
                    ),
                ),
            ]),
            BTreeMap::from([
                (
                    "svc".to_string(),
                    ContractDependencyAnalysisFacts::new(
                        ServiceProtocolIdentity::new("protocol:svc"),
                        BTreeMap::from([(
                            "run".to_string(),
                            ContractOperationId::new("operation:run"),
                        )]),
                    ),
                ),
                (
                    "both".to_string(),
                    ContractDependencyAnalysisFacts::new(
                        ServiceProtocolIdentity::new("protocol:both"),
                        BTreeMap::from([
                            (
                                "run".to_string(),
                                ContractOperationId::new("operation:both"),
                            ),
                            (
                                "other".to_string(),
                                ContractOperationId::new("operation:other"),
                            ),
                        ]),
                    ),
                ),
            ]),
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
            input.resolve_path("both.run"),
            ResolvedDependencyAnalysisTarget::Ambiguous
        ));
        assert!(matches!(
            input.resolve_path("both.other"),
            ResolvedDependencyAnalysisTarget::Ambiguous
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
            ResolvedDependencyAnalysisTarget::MissingMember
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
            BTreeMap::new(),
        );
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
            BTreeMap::new(),
        );
        assert!(input
            .package_callable(
                "pkg",
                &PackageLocalAbiIdentity::new("abi:pkg"),
                &PackageCallableId::new("callable:duplicate"),
            )
            .is_none());
    }
}
