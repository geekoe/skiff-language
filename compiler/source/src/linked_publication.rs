use std::{collections::BTreeMap, path::Path};

use crate::{
    parsed_sources::ParsedCompilerSource, source_graph::CompilerSourceFile,
    SourceCompilePackageFacts,
};
use compiler_input_model::{PackageCompilePolicy, PackageDependency, PublicationApiSpec};
use skiff_compiler_input::CompilerPlatformPackageAuthority;
use skiff_artifact_model::PackageArtifact;

pub struct LinkedPackage<'a, 'facts> {
    pub parsed_sources: Vec<ParsedCompilerSource>,
    pub production_sources: Vec<CompilerSourceFile>,
    pub diagnostic_root: &'a Path,
    pub publication_api: Option<&'a PublicationApiSpec>,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_dependencies: &'a [PackageDependency],
    pub package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
    pub package_artifacts: Option<&'facts [PackageArtifact]>,
    pub policy: PackageCompilePolicy<'a>,
    pub platform_package_authority: Option<&'a CompilerPlatformPackageAuthority>,
}

pub struct CompileParsedPackageSourcesInput<'a, 'facts> {
    pub parsed_sources: Vec<ParsedCompilerSource>,
    pub production_sources: Vec<CompilerSourceFile>,
    pub diagnostic_root: &'a Path,
    pub publication_api: Option<&'a PublicationApiSpec>,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_dependencies: &'a [PackageDependency],
    pub package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
    pub package_artifacts: Option<&'facts [PackageArtifact]>,
    pub policy: PackageCompilePolicy<'a>,
    pub platform_package_authority: Option<&'a CompilerPlatformPackageAuthority>,
}

impl<'a, 'facts> LinkedPackage<'a, 'facts> {
    pub fn from_parsed_sources(input: CompileParsedPackageSourcesInput<'a, 'facts>) -> Self {
        Self {
            parsed_sources: input.parsed_sources,
            production_sources: input.production_sources,
            diagnostic_root: input.diagnostic_root,
            publication_api: input.publication_api,
            package_aliases: input.package_aliases,
            package_dependencies: input.package_dependencies,
            package_facts: input.package_facts,
            package_artifacts: input.package_artifacts,
            policy: input.policy,
            platform_package_authority: input.platform_package_authority,
        }
    }
}
