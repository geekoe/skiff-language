use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryOperationDescriptor, CallableSemanticFacts, ContractOperationId, ContractRequirement,
    PackageBuildId, PackageCallableId, PackageCallableSignature, PackageLocalAbiIdentity,
    PackageSchemaTypeRecord, PackageTypeRef, ServiceContract,
};
use skiff_compiler_input::{
    ContractDependencyError, ContractDependencyIndex, ResolvedContractDependency,
};
use thiserror::Error;

use crate::shared::ast_utils::dependency_source_address_parts;
use crate::PublicationDbMetadataIndex;

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
    #[error(
        "package dependency view `{alias}` has invalid canonical alias `{canonical_alias}`: {reason}"
    )]
    InvalidPackageCanonicalAlias {
        alias: String,
        canonical_alias: String,
        reason: String,
    },
}

/// Canonical dependency facts made available to source call-target and effect
/// analysis. This input is intentionally independent from legacy publication
/// ABI and provider/deployment artifacts.
#[derive(Debug, Clone, Default)]
pub struct SourceDependencyAnalysisInput {
    packages: BTreeMap<String, PackageDependencyAnalysisFacts>,
    contracts: ContractDependencyIndex,
    foreign_db_metadata: PublicationDbMetadataIndex,
}

#[derive(Debug, Clone)]
pub struct PackageDependencyAnalysisFacts {
    canonical_alias: Option<String>,
    package_build_id: PackageBuildId,
    expected_local_abi: PackageLocalAbiIdentity,
    compiler_owned: bool,
    callables: BTreeMap<String, PackageDependencyCallableAnalysis>,
    constants: BTreeMap<String, PackageDependencyConstantAnalysis>,
    schema_records: BTreeMap<String, skiff_artifact_model::PackageSchemaTypeRecord>,
}

#[derive(Debug, Clone)]
pub struct PackageDependencyCallableAnalysis {
    callable_id: PackageCallableId,
    semantic_facts: CallableSemanticFacts,
    signature: Option<PackageCallableSignature>,
    /// Inout parameter positions by index (name for diagnostics). The Package
    /// Local ABI signature wire does not yet carry parameter modes; this
    /// channel lets exact package-direct inout calls be verified and stays
    /// empty for wire-derived callees (fail closed).
    inout_parameters: BTreeMap<usize, String>,
}

#[derive(Debug, Clone)]
pub struct PackageDependencyConstantAnalysis {
    const_id: String,
    ty: PackageTypeRef,
}

pub(crate) enum ResolvedDependencyAnalysisTarget<'a> {
    Package {
        alias: String,
        package_build_id: &'a PackageBuildId,
        expected_local_abi: &'a PackageLocalAbiIdentity,
        compiler_owned: bool,
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
        for (alias, facts) in &package_index {
            let Some(canonical_alias) = facts.canonical_alias.as_deref() else {
                continue;
            };
            let Some(canonical) = package_index.get(canonical_alias) else {
                return Err(
                    SourceDependencyAnalysisError::InvalidPackageCanonicalAlias {
                        alias: alias.clone(),
                        canonical_alias: canonical_alias.to_string(),
                        reason: "the primary dependency view is missing".to_string(),
                    },
                );
            };
            if canonical
                .canonical_alias
                .as_deref()
                .is_some_and(|owner| owner != canonical_alias)
            {
                return Err(
                    SourceDependencyAnalysisError::InvalidPackageCanonicalAlias {
                        alias: alias.clone(),
                        canonical_alias: canonical_alias.to_string(),
                        reason: "the target is not a primary dependency view".to_string(),
                    },
                );
            }
            if facts.package_build_id != canonical.package_build_id {
                return Err(
                    SourceDependencyAnalysisError::InvalidPackageCanonicalAlias {
                        alias: alias.clone(),
                        canonical_alias: canonical_alias.to_string(),
                        reason: "the view and primary alias select different package builds"
                            .to_string(),
                    },
                );
            }
            if facts.expected_local_abi != canonical.expected_local_abi {
                return Err(
                    SourceDependencyAnalysisError::InvalidPackageCanonicalAlias {
                        alias: alias.clone(),
                        canonical_alias: canonical_alias.to_string(),
                        reason: "the view and primary alias select different Local ABI identities"
                            .to_string(),
                    },
                );
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
            foreign_db_metadata: PublicationDbMetadataIndex::default(),
        })
    }

    /// Attaches DB facts already validated against exact direct dependency
    /// artifacts and canonical provider File IR records.
    pub fn with_foreign_db_metadata(mut self, metadata: PublicationDbMetadataIndex) -> Self {
        self.foreign_db_metadata = metadata;
        self
    }

    pub(crate) fn foreign_db_metadata(&self) -> &PublicationDbMetadataIndex {
        &self.foreign_db_metadata
    }

    /// Resolves both dependency kinds through the namespace frozen by `new`.
    pub(crate) fn resolve_path(&self, path: &str) -> ResolvedDependencyAnalysisTarget<'_> {
        let address = dependency_source_address_parts(path).or_else(|| {
            let (alias, callable_path) = path.split_once('.')?;
            self.packages
                .get(alias)
                .is_some_and(|facts| facts.compiler_owned)
                .then_some((alias, callable_path))
        });
        let Some((alias, callable_path)) = address else {
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
                    alias: facts
                        .canonical_alias
                        .clone()
                        .unwrap_or_else(|| alias.to_string()),
                    package_build_id: &facts.package_build_id,
                    expected_local_abi: &facts.expected_local_abi,
                    compiler_owned: facts.compiler_owned,
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
                        record.package_id == package_id && record.stable_schema_key == stable_key
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
        let mut matches = self
            .packages
            .iter()
            .filter(|(view_alias, facts)| {
                facts
                    .canonical_alias
                    .as_deref()
                    .unwrap_or(view_alias.as_str())
                    == alias
                    && &facts.expected_local_abi == expected_local_abi
            })
            .flat_map(|(_, facts)| facts.callables.values())
            .filter(|callable| &callable.callable_id == callable_id);
        let callable = matches.next()?;
        matches.next().is_none().then_some(callable)
    }

    pub(crate) fn package_callable_by_source_path(
        &self,
        path: &str,
    ) -> Option<(&str, &PackageDependencyCallableAnalysis)> {
        let (alias, public_path) =
            dependency_source_address_parts(path).or_else(|| path.split_once('.'))?;
        let (alias, facts) = self.packages.get_key_value(alias)?;
        Some((
            facts.canonical_alias.as_deref().unwrap_or(alias.as_str()),
            facts.callables.get(public_path)?,
        ))
    }

    pub fn package_constant_by_source_path(
        &self,
        path: &str,
    ) -> Option<(
        &str,
        &PackageLocalAbiIdentity,
        &PackageDependencyConstantAnalysis,
    )> {
        let (alias, source_path) = dependency_source_address_parts(path)?;
        let (alias, facts) = self.packages.get_key_value(alias)?;
        Some((
            facts.canonical_alias.as_deref().unwrap_or(alias.as_str()),
            &facts.expected_local_abi,
            facts.constants.get(source_path)?,
        ))
    }

    pub(crate) fn package_aliases(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(String::as_str)
    }

    pub(crate) fn compiler_owned_package_owners(
        &self,
    ) -> impl Iterator<Item = (&str, &PackageBuildId, &PackageLocalAbiIdentity)> {
        self.packages.iter().filter_map(|(alias, facts)| {
            facts.compiler_owned.then_some((
                alias.as_str(),
                &facts.package_build_id,
                &facts.expected_local_abi,
            ))
        })
    }

    pub(crate) fn contract_aliases(&self) -> impl Iterator<Item = &str> {
        self.contracts
            .dependencies()
            .map(|dependency| dependency.requirement().alias.as_str())
    }
}

impl PackageDependencyAnalysisFacts {
    pub fn new(
        package_build_id: PackageBuildId,
        expected_local_abi: PackageLocalAbiIdentity,
        callables: BTreeMap<String, PackageDependencyCallableAnalysis>,
    ) -> Self {
        Self {
            canonical_alias: None,
            package_build_id,
            expected_local_abi,
            compiler_owned: false,
            callables,
            constants: BTreeMap::new(),
            schema_records: BTreeMap::new(),
        }
    }

    /// Makes a source-only view lower through the manifest dependency's
    /// primary alias. Multiple views still describe one requirement.
    pub fn with_canonical_alias(mut self, alias: impl Into<String>) -> Self {
        self.canonical_alias = Some(alias.into());
        self
    }

    /// Marks facts selected from a compiler-owned package graph entry rather
    /// than from a manifest dependency. The package still lowers through the
    /// ordinary requirement/link path; this bit only authorizes its reserved
    /// source namespace without fabricating a manifest declaration.
    pub fn compiler_owned(mut self) -> Self {
        self.compiler_owned = true;
        self
    }

    pub fn with_constants(
        mut self,
        constants: impl IntoIterator<Item = (String, PackageDependencyConstantAnalysis)>,
    ) -> Self {
        self.constants = constants.into_iter().collect();
        self
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
            inout_parameters: BTreeMap::new(),
        }
    }

    pub fn with_signature(mut self, signature: PackageCallableSignature) -> Self {
        self.signature = Some(signature);
        self
    }

    /// Declares inout parameter positions (by parameter index, name for
    /// diagnostics) of this exact package-direct callee.
    pub fn with_inout_parameters(
        mut self,
        inout_parameters: impl IntoIterator<Item = (usize, String)>,
    ) -> Self {
        self.inout_parameters = inout_parameters.into_iter().collect();
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

    pub(crate) fn inout_parameters(&self) -> &BTreeMap<usize, String> {
        &self.inout_parameters
    }
}

impl PackageDependencyConstantAnalysis {
    pub fn new(const_id: impl Into<String>, ty: PackageTypeRef) -> Self {
        Self {
            const_id: const_id.into(),
            ty,
        }
    }

    pub fn const_id(&self) -> &str {
        &self.const_id
    }

    pub fn ty(&self) -> &PackageTypeRef {
        &self.ty
    }
}

#[cfg(test)]
mod tests;
