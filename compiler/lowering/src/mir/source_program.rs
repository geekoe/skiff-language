use std::{collections::BTreeMap, path::Path};

use skiff_compiler_source::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, SourceCompileError,
    SourceDependencyAnalysisInput,
};

use crate::LoweredPackage;

/// Explicit input for a single-file compiler test program.
///
/// This is a public compiler test carrier, not a MIR constructor: the source
/// travels through the production parser, source semantic model and
/// [`crate::lower`] entrypoint. Callers cannot inject or repair MIR facts.
#[derive(Debug, Clone, Copy)]
pub struct SingleSourceProgram<'a> {
    pub platform_root: &'a Path,
    pub package_id: &'a str,
    pub module_path: &'a str,
    pub relative_path: &'a str,
    pub source: &'a str,
}

/// Lowers one real `.skiff` source through the production source and MIR
/// pipeline. Intended for cross-crate compiler conformance tests whose owner
/// cannot depend on the driver crate without introducing a dependency cycle.
#[doc(hidden)]
pub fn lower_single_source_program(
    input: SingleSourceProgram<'_>,
) -> Result<LoweredPackage, SourceCompileError> {
    skiff_compiler_source::callable_effects::initialize_platform_for_compiler_test(
        input.platform_root,
    )
    .map_err(|message| SourceCompileError::ContractValidation { message })?;

    let diagnostic_root = std::env::temp_dir().join("skiff-lowering-source-program");
    let source = CompilerSourceFile::parse(
        input.relative_path.into(),
        input.module_path.to_string(),
        false,
        false,
        input.source.to_string(),
        input.relative_path,
    )?;
    let production_sources = vec![source];
    let parsed_sources = parse_publication_sources(&diagnostic_root, &production_sources)?;
    let model = build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: &diagnostic_root,
            publication_api: None,
            package_aliases: &BTreeMap::new(),
            package_dependencies: &[],
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new(input.package_id),
        },
        &SourceDependencyAnalysisInput::new([], []).map_err(|error| {
            SourceCompileError::ContractValidation {
                message: error.to_string(),
            }
        })?,
    )?;
    crate::lower(&model)
}
