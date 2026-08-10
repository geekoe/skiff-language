use std::{collections::BTreeMap, path::PathBuf};

use skiff_artifact_model::{CallTargetIr, ExprIr};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput, PackageCompilePolicy,
    SourceDependencyAnalysisInput,
};

use crate::{mir::MirFunction, LoweredPackage};

mod calls;
mod canonical;
mod dispatch;
mod generated;
mod owners;

const PACKAGE_ID: &str = "example.com/mir-source-events";

fn build_model(sources: &[(&str, &str, &str)]) -> skiff_compiler_source::PackageSourceModel {
    build_model_for_package(PACKAGE_ID, sources)
}

fn build_model_for_package(
    package_id: &str,
    sources: &[(&str, &str, &str)],
) -> skiff_compiler_source::PackageSourceModel {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    initialize_prelude_registry(
        &CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load"),
    )
    .expect("prelude registry initializes");

    let root = PathBuf::from("/mir-source-event-fixture");
    let production_sources = sources
        .iter()
        .map(|(relative_path, module_path, source_text)| {
            CompilerSourceFile::parse(
                PathBuf::from(relative_path),
                (*module_path).to_string(),
                false,
                false,
                (*source_text).to_string(),
                *relative_path,
            )
            .expect("source-event fixture parses")
        })
        .collect::<Vec<_>>();
    let parsed_sources =
        parse_publication_sources(&root, &production_sources).expect("source-event facts build");
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &BTreeMap::new(),
            package_dependencies: &[],
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new(package_id),
        },
        &SourceDependencyAnalysisInput::new([], []).unwrap(),
    )
    .expect("source-event fixture model builds")
}

fn lower_sources(sources: &[(&str, &str, &str)]) -> LoweredPackage {
    let model = build_model(sources);
    crate::lower(&model).expect("source-event fixture lowers")
}

fn lower_sources_for_package(package_id: &str, sources: &[(&str, &str, &str)]) -> LoweredPackage {
    let model = build_model_for_package(package_id, sources);
    crate::lower(&model).expect("source-event fixture lowers")
}

fn function<'a>(
    lowered: &'a LoweredPackage,
    module_path: &str,
    declaration: &str,
) -> &'a MirFunction {
    let symbol = format!("{module_path}.{declaration}");
    lowered
        .mir_units()
        .iter()
        .find(|unit| unit.module_path == module_path)
        .and_then(|unit| {
            unit.functions
                .iter()
                .find(|function| function.symbol == symbol)
        })
        .expect("MIR function exists")
}

fn direct_call_indices(function: &MirFunction) -> Vec<u32> {
    function
        .expressions
        .iter()
        .filter_map(|expression| match &expression.expression {
            ExprIr::Call { call }
                if matches!(
                    &call.target,
                    CallTargetIr::LocalExecutable { .. }
                        | CallTargetIr::PublicationExecutable { .. }
                        | CallTargetIr::PackageCallable { .. }
                ) =>
            {
                Some(expression.index)
            }
            _ => None,
        })
        .collect()
}
