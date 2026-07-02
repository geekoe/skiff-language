use crate::{
    input::{
        compile_input::PublicationInput, PackageDependency, Publication, PublicationCompilePolicy,
        ResolvedServiceDependencies, ServiceIngressSeed,
    },
    shared::publication_error::PublicationError,
};
use std::collections::BTreeMap;

pub(crate) use skiff_compiler_source::*;

pub(crate) fn build(input: PublicationInput<'_>) -> Result<SourceCompileModel, PublicationError> {
    build_with_package_facts(input, None)
}

pub(crate) fn build_with_package_facts<'a, 'facts>(
    input: PublicationInput<'a>,
    package_facts: Option<&'facts [SourceCompilePackageFacts<'a>]>,
) -> Result<SourceCompileModel, PublicationError> {
    let parts = SourceCompileInputParts::from_publication_input(input);
    let production_sources = parts.publication.production_sources();
    let parsed_sources = skiff_compiler_source::parsed_sources::parse_publication_sources(
        &parts.publication.source_tree.root,
        &production_sources,
    )?;
    Ok(skiff_compiler_source::build_from_parsed_sources(
        CompileParsedPublicationSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: &parts.publication.source_tree.root,
            publication_api: Some(&parts.publication.manifest.api),
            package_aliases: parts.package_aliases,
            package_dependencies: parts.package_dependencies,
            package_facts,
            service_dependencies: parts.service_dependencies,
            service_ingress: parts.service_ingress,
            policy: parts.policy,
        },
    )?)
}

struct SourceCompileInputParts<'a> {
    publication: &'a Publication,
    package_aliases: &'a BTreeMap<String, Vec<String>>,
    package_dependencies: &'a [PackageDependency],
    service_dependencies: ResolvedServiceDependencies,
    service_ingress: Option<ServiceIngressSeed>,
    policy: PublicationCompilePolicy<'a>,
}

impl<'a> SourceCompileInputParts<'a> {
    fn from_publication_input(input: PublicationInput<'a>) -> Self {
        match input {
            PublicationInput::Package(package) => Self {
                publication: package.core.publication,
                package_aliases: package.core.package_aliases,
                package_dependencies: package.core.package_dependencies,
                service_dependencies: Default::default(),
                service_ingress: None,
                policy: PublicationCompilePolicy::Package {
                    package_id: package.package_id,
                },
            },
            PublicationInput::Service(service) => Self {
                publication: service.core.publication,
                package_aliases: service.core.package_aliases,
                package_dependencies: service.core.package_dependencies,
                service_dependencies: service.service_dependencies,
                service_ingress: Some(service.service_ingress),
                policy: PublicationCompilePolicy::Service {
                    service_id: service.service_id,
                },
            },
        }
    }
}
