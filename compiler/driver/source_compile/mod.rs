use crate::{
    input::{compile_input::PackageCompileInput, PackageCompilePolicy},
    shared::package_compile_error::PackageCompileError,
};
use skiff_compiler_source::{
    CompileParsedPackageSourcesInput, PackageSourceModel, SourceDependencyAnalysisInput,
};

#[cfg(test)]
thread_local! {
    static TEST_COMPILE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

mod canonical_dependencies;

pub(crate) fn compile(
    input: &PackageCompileInput<'_>,
) -> Result<skiff_compiler_compiled::CompiledPackage, PackageCompileError> {
    #[cfg(test)]
    TEST_COMPILE_COUNT.with(|count| count.set(count.get() + 1));
    let dependency_handoff = canonical_dependencies::CanonicalDependencyHandoff::build(input)?;
    let model = build(input, dependency_handoff.source_analysis())?;
    let lowered = skiff_compiler_lowering::lower_with_contract_operations(
        &model,
        dependency_handoff.contract_operations(),
    )?;
    Ok(skiff_compiler_compiled::CompiledPackage::new(
        model, lowered,
    ))
}

#[cfg(test)]
pub(crate) fn reset_test_compile_count() {
    TEST_COMPILE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn test_compile_count() -> usize {
    TEST_COMPILE_COUNT.with(std::cell::Cell::get)
}

fn build<'a>(
    input: &PackageCompileInput<'a>,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageSourceModel, PackageCompileError> {
    let production_sources = input.package.production_sources();
    let parsed_sources = skiff_compiler_source::parsed_sources::parse_publication_sources(
        &input.package.source_tree.root,
        &production_sources,
    )?;
    Ok(
        skiff_compiler_source::build_package_from_parsed_sources_with_dependency_analysis(
            CompileParsedPackageSourcesInput {
                parsed_sources,
                production_sources,
                diagnostic_root: &input.package.source_tree.root,
                publication_api: Some(&input.package.manifest.api),
                package_aliases: input.package_aliases,
                package_dependencies: input.package_dependencies,
                package_facts: None,
                policy: PackageCompilePolicy::new(input.package_id),
            },
            dependency_analysis,
        )?,
    )
}
