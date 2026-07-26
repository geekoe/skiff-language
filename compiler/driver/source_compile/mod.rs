use crate::{
    input::{compile_input::PackageCompileInput, PackageCompilePolicy},
    shared::package_compile_error::PackageCompileError,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;
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
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<skiff_compiler_compiled::CompiledPackage, PackageCompileError> {
    #[cfg(test)]
    TEST_COMPILE_COUNT.with(|count| count.set(count.get() + 1));
    let canonical_dependencies =
        canonical_dependencies::source_dependencies(input, resolved_package_schemas)?;
    if let Some(overlay) = &input.package.test_overlay {
        let manifest = &input.package.manifest;
        if overlay.production.package_id != manifest.id.as_str()
            || overlay.production.package_version != manifest.version
        {
            return Err(PackageCompileError::ContractValidation {
                message: format!(
                    "test overlay production coordinate {}@{} does not match source package {}@{}",
                    overlay.production.package_id,
                    overlay.production.package_version,
                    manifest.id,
                    manifest.version
                ),
            });
        }
    }
    let model = build(
        input,
        &canonical_dependencies.analysis,
        &canonical_dependencies.type_resolution_artifacts,
    )?;
    let lowered = skiff_compiler_lowering::lower(&model)?;
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
    type_resolution_artifacts: &[skiff_artifact_model::PackageArtifact],
) -> Result<PackageSourceModel, PackageCompileError> {
    let production_sources = input.package.compile_sources();
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
                package_artifacts: Some(type_resolution_artifacts),
                policy: PackageCompilePolicy::new(input.package_id),
            },
            dependency_analysis,
        )?,
    )
}
