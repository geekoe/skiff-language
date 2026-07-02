use std::path::Path;

use compiler_input_model::{PackageDependency, PublicationApiSpec, PublicationCompilePolicy};

use crate::{
    config_requirements::DependencyPackageConfigFacts,
    parsed_sources::ParsedCompilerSource,
    root_refs,
    source_graph::CompilerSourceFile,
    ConfigRequirementScope, ConfigRequirementSet, PublicationError, SourceCompileLinkedFacts,
    SourceCompileLinkedFactsInput,
};

#[derive(Debug)]
pub struct SourceConfigAndEffectMetadata {
    config: SourceConfigMetadata,
    effects: SourceEffectMetadata,
}

#[derive(Debug)]
pub struct SourceConfigMetadata {
    legacy_config_projection_requirements: ConfigRequirementSet,
    own_config_requirements: ConfigRequirementSet,
    dependency_config_requirements: ConfigRequirementSet,
    effective_config_requirements: ConfigRequirementSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceEffectMetadata {
    Empty,
}

pub struct SourceConfigAndEffectMetadataInput<'a, 'source> {
    pub diagnostic_root: &'a Path,
    pub parsed_sources: &'a [ParsedCompilerSource],
    pub production_sources: &'source [CompilerSourceFile],
    pub package_dependencies: &'a [PackageDependency],
    pub dependency_package_config_facts: Option<&'a [DependencyPackageConfigFacts<'a>]>,
    pub policy: PublicationCompilePolicy<'a>,
    pub publication_api: Option<&'a PublicationApiSpec>,
}

pub struct SourceConfigAndEffectMetadataBatchInput<'a, 'source> {
    pub diagnostic_root: &'a Path,
    pub parsed_sources: &'a [ParsedCompilerSource],
    pub production_sources: &'source [CompilerSourceFile],
    pub package_dependencies: &'a [PackageDependency],
    pub dependency_package_config_facts: Option<&'a [DependencyPackageConfigFacts<'a>]>,
    pub policy: PublicationCompilePolicy<'a>,
    pub publication_api: Option<&'a PublicationApiSpec>,
    pub entrypoint_function_names: &'a [String],
}

pub fn source_config_and_effect_metadata_from_parsed_sources(
    input: SourceConfigAndEffectMetadataInput<'_, '_>,
) -> Result<SourceConfigAndEffectMetadata, PublicationError> {
    validate_source_config_metadata_input(
        input.diagnostic_root,
        input.parsed_sources,
        input.production_sources,
        input.policy,
    )?;
    let linked_facts = SourceCompileLinkedFacts::build(SourceCompileLinkedFactsInput {
        diagnostic_root: input.diagnostic_root,
        parsed_sources: input.parsed_sources,
        production_sources: input.production_sources,
        package_dependencies: input.package_dependencies,
        dependency_package_config_facts: input.dependency_package_config_facts,
        policy: input.policy,
        publication_api: input.publication_api,
    })?;
    let config_usage_seed = crate::config_usage::collect_config_usage_seed_from_parsed_sources(
        input.diagnostic_root,
        input.parsed_sources,
    )?;
    source_config_and_effect_metadata_from_config_usage_seed(
        &config_usage_seed,
        &linked_facts.dependency_config_requirements,
        input.policy,
    )
}

pub fn source_config_and_effect_metadata_batches_from_parsed_sources(
    input: SourceConfigAndEffectMetadataBatchInput<'_, '_>,
) -> Result<Vec<SourceConfigAndEffectMetadata>, PublicationError> {
    validate_source_config_metadata_input(
        input.diagnostic_root,
        input.parsed_sources,
        input.production_sources,
        input.policy,
    )?;
    let linked_facts = SourceCompileLinkedFacts::build(SourceCompileLinkedFactsInput {
        diagnostic_root: input.diagnostic_root,
        parsed_sources: input.parsed_sources,
        production_sources: input.production_sources,
        package_dependencies: input.package_dependencies,
        dependency_package_config_facts: input.dependency_package_config_facts,
        policy: input.policy,
        publication_api: input.publication_api,
    })?;
    crate::config_usage::collect_config_usage_seed_batches_from_parsed_sources(
        input.diagnostic_root,
        input.parsed_sources,
        input.entrypoint_function_names,
    )?
    .iter()
    .map(|config_usage_seed| {
        source_config_and_effect_metadata_from_config_usage_seed(
            config_usage_seed,
            &linked_facts.dependency_config_requirements,
            input.policy,
        )
    })
    .collect()
}

fn validate_source_config_metadata_input(
    diagnostic_root: &Path,
    parsed_sources: &[ParsedCompilerSource],
    production_sources: &[CompilerSourceFile],
    policy: PublicationCompilePolicy<'_>,
) -> Result<(), PublicationError> {
    let root_ref_policy = match policy {
        PublicationCompilePolicy::Package { .. } => {
            root_refs::RootRefValidationPolicy::parsed_publication_sources()
        }
        PublicationCompilePolicy::Service { .. } => {
            root_refs::RootRefValidationPolicy::service_sources()
        }
    };
    root_refs::validate_source_root_refs(
        diagnostic_root,
        production_sources,
        root_ref_policy,
    )?;
    if matches!(policy, PublicationCompilePolicy::Service { .. }) {
        crate::service_storage_rules::validate_service_storage_sources(parsed_sources)?;
    }
    Ok(())
}

fn source_config_and_effect_metadata_from_config_usage_seed(
    config_usage_seed: &crate::config_usage::ConfigUsageSeed,
    dependency_config_requirements: &ConfigRequirementSet,
    policy: PublicationCompilePolicy<'_>,
) -> Result<SourceConfigAndEffectMetadata, PublicationError> {
    let scope = ConfigRequirementScope::from_publication_policy(policy);
    let own_config_requirements =
        ConfigRequirementSet::from_usage_seed(config_usage_seed, scope.clone());
    let effective_config_requirements = ConfigRequirementSet::effective(
        &own_config_requirements,
        dependency_config_requirements,
    )?;
    let legacy_config_projection_requirements =
        effective_config_requirements.matching_scope(&scope);

    Ok(SourceConfigAndEffectMetadata {
        config: SourceConfigMetadata {
            legacy_config_projection_requirements,
            own_config_requirements,
            dependency_config_requirements: dependency_config_requirements.clone(),
            effective_config_requirements,
        },
        effects: SourceEffectMetadata::Empty,
    })
}

impl SourceConfigAndEffectMetadata {
    pub fn config(&self) -> &SourceConfigMetadata {
        &self.config
    }

    pub fn effects(&self) -> SourceEffectMetadata {
        self.effects
    }
}

impl SourceConfigMetadata {
    pub fn legacy_config_projection_requirements(&self) -> &ConfigRequirementSet {
        &self.legacy_config_projection_requirements
    }

    pub fn own_config_requirements(&self) -> &ConfigRequirementSet {
        &self.own_config_requirements
    }

    pub fn dependency_config_requirements(&self) -> &ConfigRequirementSet {
        &self.dependency_config_requirements
    }

    pub fn effective_config_requirements(&self) -> &ConfigRequirementSet {
        &self.effective_config_requirements
    }
}
