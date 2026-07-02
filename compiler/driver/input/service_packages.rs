use crate::input::source_graph::CompilerSourceFile;

pub(crate) type PackageManifestDiscoveryResult =
    skiff_compiler_input::PackageManifestDiscoveryResult;

pub(crate) fn service_source_package_facts_from_compiler_sources(
    user_production_sources: &[CompilerSourceFile],
) -> skiff_compiler_input::ServiceSourcePackageFacts {
    let facts = skiff_compiler_source::service_package_facts::service_source_package_facts_from_compiler_sources(
        user_production_sources,
    );
    skiff_compiler_input::ServiceSourcePackageFacts {
        imports: facts.imports,
        references_std_package_types: facts.references_std_package_types,
    }
}
