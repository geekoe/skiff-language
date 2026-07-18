use std::collections::BTreeMap;

use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{PackageArtifact, PackageLocalAbiSymbol};
use skiff_compiler_input::{ContractDependencyIndex, ResolvedContractDependency};
use skiff_compiler_lowering::{
    ContractDependencyOperationIndex, ContractDependencyOperationIndexEntry,
};
use skiff_compiler_source::{
    PackageDependencyAnalysisFacts, PackageDependencyCallableAnalysis,
    SourceDependencyAnalysisInput,
};

use crate::{
    input::{compile_input::PackageCompileInput, PackageDependency},
    shared::package_compile_error::PackageCompileError,
};

pub(super) struct CanonicalDependencyHandoff {
    source_analysis: SourceDependencyAnalysisInput,
    contract_operations: ContractDependencyOperationIndex,
}

impl CanonicalDependencyHandoff {
    pub(super) fn build(input: &PackageCompileInput<'_>) -> Result<Self, PackageCompileError> {
        let source_analysis = SourceDependencyAnalysisInput::new(
            package_analysis(input)?,
            validated_contract_dependencies(input)?,
        )
        .map_err(dependency_analysis_error)?;
        let contract_operations =
            contract_operation_index(source_analysis.contract_dependencies())?;
        Ok(Self {
            source_analysis,
            contract_operations,
        })
    }

    pub(super) fn source_analysis(&self) -> &SourceDependencyAnalysisInput {
        &self.source_analysis
    }

    pub(super) fn contract_operations(&self) -> &ContractDependencyOperationIndex {
        &self.contract_operations
    }
}

fn validated_contract_dependencies(
    input: &PackageCompileInput<'_>,
) -> Result<Vec<ResolvedContractDependency>, PackageCompileError> {
    input
        .contract_dependencies
        .iter()
        .map(|dependency| {
            ResolvedContractDependency::validated(
                dependency.requirement.clone(),
                dependency.contract.clone(),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(contract_error)
}

fn package_analysis(
    input: &PackageCompileInput<'_>,
) -> Result<Vec<(String, PackageDependencyAnalysisFacts)>, PackageCompileError> {
    let artifacts = input
        .dependency_packages
        .iter()
        .map(|artifact| {
            (
                (
                    artifact.package_id.as_str(),
                    artifact.package_version.as_str(),
                ),
                artifact,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut facts = Vec::new();
    for dependency in input.package_dependencies {
        let Some(artifact) = artifacts.get(&(dependency.id.as_str(), dependency.version.as_str()))
        else {
            return Err(validation_error(format!(
                "package {} dependency {}@{} has no canonical PackageArtifact",
                input.package_id, dependency.id, dependency.version
            )));
        };
        validate_package_artifact_identities(artifact).map_err(|error| {
            validation_error(format!(
                "package dependency {}@{} identity validation failed: {error}",
                dependency.id, dependency.version
            ))
        })?;
        let alias = dependency.effective_alias().to_string();
        let callables = package_callable_analysis(dependency, artifact)?;
        facts.push((
            alias,
            PackageDependencyAnalysisFacts::new(
                artifact.package_local_abi.local_abi_identity.clone(),
                callables,
            ),
        ));
    }
    Ok(facts)
}

fn package_callable_analysis(
    dependency: &PackageDependency,
    artifact: &PackageArtifact,
) -> Result<BTreeMap<String, PackageDependencyCallableAnalysis>, PackageCompileError> {
    artifact
        .package_local_abi
        .public_symbols
        .iter()
        .filter_map(|(public_path, symbol)| {
            let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
                return None;
            };
            let result = artifact
                .callable_semantic_facts
                .get(callable_id)
                .cloned()
                .ok_or_else(|| {
                    validation_error(format!(
                        "package dependency {} callable {} has no semantic facts",
                        dependency.id, callable_id
                    ))
                })
                .map(|semantic_facts| {
                    (
                        dependency_member_path(dependency, public_path),
                        PackageDependencyCallableAnalysis::new(callable_id.clone(), semantic_facts),
                    )
                });
            Some(result)
        })
        .collect()
}

fn dependency_member_path(dependency: &PackageDependency, public_path: &str) -> String {
    if dependency.id == skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID {
        public_path
            .strip_prefix("std.")
            .unwrap_or(public_path)
            .to_string()
    } else {
        public_path.to_string()
    }
}

fn contract_operation_index(
    contracts: &ContractDependencyIndex,
) -> Result<ContractDependencyOperationIndex, PackageCompileError> {
    ContractDependencyOperationIndex::build(contracts.dependencies().map(|dependency| {
        ContractDependencyOperationIndexEntry::new(
            dependency.requirement().clone(),
            dependency.contract().operations.clone(),
        )
    }))
    .map_err(|error| validation_error(format!("contract operation index failed: {error}")))
}

fn contract_error(error: impl std::fmt::Display) -> PackageCompileError {
    validation_error(format!("contract dependency validation failed: {error}"))
}

fn dependency_analysis_error(error: impl std::fmt::Display) -> PackageCompileError {
    validation_error(format!("dependency alias validation failed: {error}"))
}

fn validation_error(message: String) -> PackageCompileError {
    PackageCompileError::ContractValidation { message }
}
