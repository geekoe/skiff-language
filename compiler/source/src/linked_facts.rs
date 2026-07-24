use std::path::Path;

use crate::{
    package_rules::validate_package_sources_with_dependency_analysis,
    parsed_sources::ParsedCompilerSource,
    shared::{id::STD_SOURCE_ALIAS, publication_error::PublicationError},
    source_graph::CompilerSourceFile,
};
use compiler_input_model::{
    is_standard_package_id, PackageCompilePolicy, PackageDependency, PublicationApiSpec,
};

use super::{
    config_requirements::DependencyPackageConfigFacts, ConfigRequirementSet, PublicationApiSeed,
};

pub struct SourceCompileLinkedFacts {
    pub publication_api_seed: PublicationApiSeed,
    pub dependency_config_requirements: ConfigRequirementSet,
}

pub struct SourceCompileLinkedFactsInput<'a, 'source> {
    pub diagnostic_root: &'a Path,
    pub parsed_sources: &'a [ParsedCompilerSource],
    pub production_sources: &'source [CompilerSourceFile],
    pub package_dependencies: &'a [PackageDependency],
    pub dependency_package_config_facts: Option<&'a [DependencyPackageConfigFacts<'a>]>,
    pub policy: PackageCompilePolicy<'a>,
    pub publication_api: Option<&'a PublicationApiSpec>,
    pub dependency_analysis: &'a crate::SourceDependencyAnalysisInput,
}

impl SourceCompileLinkedFacts {
    pub fn build(input: SourceCompileLinkedFactsInput<'_, '_>) -> Result<Self, PublicationError> {
        let package_id = input.policy.package_id();
        validate_package_sources_with_dependency_analysis(
            package_id,
            input.package_dependencies,
            input.diagnostic_root,
            input.parsed_sources,
            input.dependency_analysis,
        )?;
        let publication_api_seed = match input.publication_api {
            Some(spec) => PublicationApiSeed::from_publication_sources_with_resolved_modules(
                spec,
                input.production_sources.iter(),
                |entry| {
                    if is_standard_package_id(package_id) {
                        return skiff_compiler_core::export_config::package_public_path(
                            STD_SOURCE_ALIAS,
                            entry.source_module_hint(),
                        );
                    }
                    entry.source_module_hint().to_string()
                },
            )?,
            None => PublicationApiSeed::no_publication_api(),
        };
        let dependency_config_requirements = input
            .dependency_package_config_facts
            .map(|package_facts| {
                ConfigRequirementSet::from_service_package_graph(
                    input.package_dependencies,
                    package_facts,
                )
            })
            .transpose()?
            .unwrap_or_else(ConfigRequirementSet::empty);
        Ok(Self {
            publication_api_seed,
            dependency_config_requirements,
        })
    }
}
