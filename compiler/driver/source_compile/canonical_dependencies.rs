use std::collections::BTreeMap;

use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{PackageArtifact, PackageLocalAbiSymbol};
use skiff_compiler_input::ResolvedContractDependency;
use skiff_compiler_source::{
    PackageDependencyAnalysisFacts, PackageDependencyCallableAnalysis,
    SourceDependencyAnalysisInput,
};

use crate::{
    input::{compile_input::PackageCompileInput, PackageDependency},
    shared::package_compile_error::PackageCompileError,
};

pub(super) fn source_dependency_analysis(
    input: &PackageCompileInput<'_>,
) -> Result<SourceDependencyAnalysisInput, PackageCompileError> {
    SourceDependencyAnalysisInput::new(
        package_analysis(input)?,
        validated_contract_dependencies(input)?,
    )
    .map_err(dependency_analysis_error)
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

fn contract_error(error: impl std::fmt::Display) -> PackageCompileError {
    validation_error(format!("contract dependency validation failed: {error}"))
}

fn dependency_analysis_error(error: impl std::fmt::Display) -> PackageCompileError {
    validation_error(format!("dependency alias validation failed: {error}"))
}

fn validation_error(message: String) -> PackageCompileError {
    PackageCompileError::ContractValidation { message }
}
