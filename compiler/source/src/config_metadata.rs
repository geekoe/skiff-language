use std::path::Path;

use compiler_input_model::{PackageCompilePolicy, PackageDependency, PublicationApiSpec};

use crate::{
    parsed_sources::ParsedCompilerSource, root_refs, source_graph::CompilerSourceFile,
    ConfigRequirementSet, PublicationError,
};

#[derive(Debug)]
pub struct SourceConfigMetadata {
    own_config_requirements: ConfigRequirementSet,
}

pub struct SourceConfigMetadataInput<'a, 'source> {
    pub diagnostic_root: &'a Path,
    pub parsed_sources: &'a [ParsedCompilerSource],
    pub production_sources: &'source [CompilerSourceFile],
    pub package_dependencies: &'a [PackageDependency],
    pub policy: PackageCompilePolicy<'a>,
    pub publication_api: Option<&'a PublicationApiSpec>,
}

pub struct SourceConfigMetadataBatchInput<'a, 'source> {
    pub diagnostic_root: &'a Path,
    pub parsed_sources: &'a [ParsedCompilerSource],
    pub production_sources: &'source [CompilerSourceFile],
    pub package_dependencies: &'a [PackageDependency],
    pub policy: PackageCompilePolicy<'a>,
    pub publication_api: Option<&'a PublicationApiSpec>,
    pub entrypoint_function_names: &'a [String],
}

pub fn source_config_metadata_from_parsed_sources(
    input: SourceConfigMetadataInput<'_, '_>,
) -> Result<SourceConfigMetadata, PublicationError> {
    validate_source_config_metadata_input(
        input.diagnostic_root,
        input.production_sources,
        input.policy,
    )?;
    let config_usage_seed = crate::config_usage::collect_config_usage_seed_from_parsed_sources(
        input.diagnostic_root,
        input.parsed_sources,
    )?;
    Ok(source_config_metadata_from_config_usage_seed(
        &config_usage_seed,
        input.policy,
    ))
}

pub fn source_config_metadata_batches_from_parsed_sources(
    input: SourceConfigMetadataBatchInput<'_, '_>,
) -> Result<Vec<SourceConfigMetadata>, PublicationError> {
    validate_source_config_metadata_input(
        input.diagnostic_root,
        input.production_sources,
        input.policy,
    )?;
    Ok(
        crate::config_usage::collect_config_usage_seed_batches_from_parsed_sources(
            input.diagnostic_root,
            input.parsed_sources,
            input.entrypoint_function_names,
        )?
        .iter()
        .map(|config_usage_seed| {
            source_config_metadata_from_config_usage_seed(config_usage_seed, input.policy)
        })
        .collect(),
    )
}

fn validate_source_config_metadata_input(
    diagnostic_root: &Path,
    production_sources: &[CompilerSourceFile],
    policy: PackageCompilePolicy<'_>,
) -> Result<(), PublicationError> {
    let root_ref_policy = root_refs::RootRefValidationPolicy::parsed_publication_sources();
    root_refs::validate_source_root_refs(diagnostic_root, production_sources, root_ref_policy)?;
    let _ = policy;
    Ok(())
}

fn source_config_metadata_from_config_usage_seed(
    config_usage_seed: &crate::config_usage::ConfigUsageSeed,
    policy: PackageCompilePolicy<'_>,
) -> SourceConfigMetadata {
    let _ = policy;
    let own_config_requirements = ConfigRequirementSet::from_usage_seed(config_usage_seed);
    SourceConfigMetadata {
        own_config_requirements,
    }
}

impl SourceConfigMetadata {
    pub fn own_config_requirements(&self) -> &ConfigRequirementSet {
        &self.own_config_requirements
    }
}
