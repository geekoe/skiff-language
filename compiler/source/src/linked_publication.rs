use std::{collections::BTreeMap, path::Path};

use crate::{
    parsed_sources::ParsedCompilerSource, source_graph::CompilerSourceFile,
    SourceCompilePackageFacts,
};
use compiler_input_model::{
    PackageDependency, PublicationApiSpec, PublicationCompilePolicy, ResolvedServiceDependencies,
    ServiceIngressSeed,
};

pub struct LinkedPublication<'a, 'facts> {
    pub parsed_sources: Vec<ParsedCompilerSource>,
    pub production_sources: Vec<CompilerSourceFile>,
    pub diagnostic_root: &'a Path,
    pub publication_api: Option<&'a PublicationApiSpec>,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_dependencies: &'a [PackageDependency],
    pub package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
    pub service_dependencies: ResolvedServiceDependencies,
    pub service_ingress: Option<ServiceIngressSeed>,
    pub policy: PublicationCompilePolicy<'a>,
}

pub struct CompileParsedPublicationSourcesInput<'a, 'facts> {
    pub parsed_sources: Vec<ParsedCompilerSource>,
    pub production_sources: Vec<CompilerSourceFile>,
    pub diagnostic_root: &'a Path,
    pub publication_api: Option<&'a PublicationApiSpec>,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_dependencies: &'a [PackageDependency],
    pub package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
    pub service_dependencies: ResolvedServiceDependencies,
    pub service_ingress: Option<ServiceIngressSeed>,
    pub policy: PublicationCompilePolicy<'a>,
}

impl<'a, 'facts> LinkedPublication<'a, 'facts> {
    pub fn from_parsed_sources(input: CompileParsedPublicationSourcesInput<'a, 'facts>) -> Self {
        Self {
            parsed_sources: input.parsed_sources,
            production_sources: input.production_sources,
            diagnostic_root: input.diagnostic_root,
            publication_api: input.publication_api,
            package_aliases: input.package_aliases,
            package_dependencies: input.package_dependencies,
            package_facts: input.package_facts,
            service_dependencies: input.service_dependencies,
            service_ingress: service_ingress_for_parsed_policy(input.policy, input.service_ingress),
            policy: input.policy,
        }
    }
}

fn service_ingress_for_parsed_policy(
    policy: PublicationCompilePolicy<'_>,
    service_ingress: Option<ServiceIngressSeed>,
) -> Option<ServiceIngressSeed> {
    match policy {
        PublicationCompilePolicy::Service { .. } => Some(service_ingress.unwrap_or_default()),
        PublicationCompilePolicy::Package { .. } => None,
    }
}
