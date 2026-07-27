use std::{collections::BTreeMap, path::PathBuf};

use skiff_artifact_model::{FileIrUnit, PackageSchemaTypeId, TypeRefIr};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput, PackageCompilePolicy,
    PackageDependencyAnalysisFacts, PackageSourceModel, PublicationTypeSymbolIndex,
    SourceDependencyAnalysisInput, SourceSymbolKey,
};

use super::{
    compile_package_source_file_ir_unit, compile_source_file_ir_unit, PackageSourceLoweringInput,
};
use crate::{
    callable_return_types::extend_callable_return_types_for_source, file_ir::ExecutableIr,
};

mod contract_fixture;

use contract_fixture::contract_dependency;

const MODULE: &str = "internal.interface_execution";

#[test]
fn exact_interface_and_impl_contract_types_share_opaque_execution_projection() {
    let (model, package_schema_type_id) = contract_interface_model();
    let empty_external_types = PublicationTypeSymbolIndex::default();
    let empty = lower_model_with_external_types(&model, &empty_external_types);
    let mut unrelated_external_types = PublicationTypeSymbolIndex::default();
    unrelated_external_types.insert_resolved_symbol(
        "Unrelated",
        SourceSymbolKey::new("external.unrelated", "Unrelated"),
    );
    let nonempty = lower_model_with_external_types(&model, &unrelated_external_types);

    assert_eq!(
        empty, nonempty,
        "external symbols cannot affect exact facts"
    );
    let expected = nested_contract_execution_type();
    let operation = &empty.declarations.interfaces["Gateway"].operations[0];
    assert_eq!(operation.name, "echo");
    assert_eq!(operation.params[0].ty, TypeRefIr::builtin("Self"));
    assert_eq!(operation.params[1].name, "input");
    assert_eq!(operation.params[1].ty, expected);
    assert_eq!(operation.return_type, expected);
    assert!(!operation.is_native && !operation.is_provider && !operation.is_static);

    let implementation = executable(&empty, "Handler.echo");
    assert!(matches!(
        &implementation.params[0].ty,
        TypeRefIr::LocalType { .. }
    ));
    assert_eq!(implementation.params[1].name, "input");
    assert_eq!(implementation.params[1].ty, expected);
    assert_eq!(implementation.return_type, expected);

    let wire = serde_json::to_string(&empty).unwrap();
    for forbidden in [
        "payments",
        "types.User",
        "payments.User",
        package_schema_type_id.as_str(),
        "packageSchemaTypeId",
        "contractTypeId",
        "serviceSymbol",
    ] {
        assert!(
            !wire.contains(forbidden),
            "File IR wire must not contain `{forbidden}`: {wire}"
        );
    }
}

#[test]
fn standalone_interface_lowering_without_exact_facts_fails_closed() {
    let error = compile_source_file_ir_unit(
        "interface Gateway { function echo(input: string) -> string }",
        "gateway.skiff",
        "gateway",
        "package",
    )
    .expect_err("interface lowering must require source-owned exact facts")
    .to_string();
    assert!(
        error.contains("cannot lower without exact source requirement facts"),
        "unexpected error: {error}"
    );
}

fn contract_interface_model() -> (PackageSourceModel, PackageSchemaTypeId) {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    initialize_prelude_registry(
        &CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load"),
    )
    .expect("prelude registry initializes");
    let (dependency, package_type_record, package_local_abi) = contract_dependency();
    let package_schema_type_id = package_type_record.package_schema_type_id.clone();
    let dependency_analysis = SourceDependencyAnalysisInput::new(
        [(
            "types".to_string(),
            PackageDependencyAnalysisFacts::new(
                skiff_artifact_model::PackageBuildId::new("build:types"),
                package_local_abi,
                BTreeMap::new(),
            )
            .with_schema_records([package_type_record]),
        )],
        [dependency],
    )
    .unwrap();
    let root = PathBuf::from("/contract-interface");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/interface_execution.skiff"),
        MODULE.to_string(),
        false,
        false,
        r#"
          interface Gateway {
            function echo(
              self: Self,
              input: Array<payments.User?>?
            ) -> Array<types.User?>?
          }

          type Handler implements Gateway {}
          impl Handler {
            function echo(
              self: Handler,
              input: Array<payments.User?>?
            ) -> Array<types.User?>? {
              return input
            }
          }
        "#
        .to_string(),
        "internal/interface_execution.skiff",
    )
    .expect("contract interface fixture should parse");
    let production_sources = vec![source];
    let parsed_sources = parse_publication_sources(&root, &production_sources)
        .expect("contract interface source facts should build");
    let package_aliases = BTreeMap::new();
    let model = build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &package_aliases,
            package_dependencies: &[],
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new("example.com/contract-interface"),
        },
        &dependency_analysis,
    )
    .expect("contract interface source model should build");
    (model, package_schema_type_id)
}

fn lower_model_with_external_types(
    model: &PackageSourceModel,
    external_type_symbols: &PublicationTypeSymbolIndex,
) -> FileIrUnit {
    let parsed = model
        .sources()
        .parsed_sources()
        .first()
        .expect("one contract interface source");
    let package_interface_methods = model.type_resolution().package_interface_method_index();
    let mut callable_return_types = BTreeMap::new();
    extend_callable_return_types_for_source(
        &mut callable_return_types,
        parsed.module_path(),
        parsed.ast(),
    );
    model
        .with_semantic_context(|semantic_context| {
            let source_context = semantic_context
                .source_context(parsed.module_path())
                .map_err(source_error)?;
            compile_package_source_file_ir_unit(PackageSourceLoweringInput {
                source: parsed.source_text(),
                role: "package",
                package_aliases: model.name_resolution().package_aliases_map(),
                package_interface_methods: &package_interface_methods,
                resolved_call_targets: model.resolved_call_targets(),
                external_type_symbols,
                publication_db_metadata: model.indexes().publication_db_metadata_index(),
                semantic_context: &source_context,
                source_alias_targets: model
                    .resolutions()
                    .alias_targets_for_module(parsed.module_path()),
                type_resolution: model.type_resolution(),
                expression_types: Some(model.expression_types()),
                execution_semantics: Some(model.execution_semantics()),
                callable_return_types: &callable_return_types,
                executable_signatures: model.executable_signatures(),
                interface_signatures: Some(model.interface_signatures()),
                service_calls: None,
            })
            .map_err(source_error)
        })
        .expect("contract interface should lower")
}

fn source_error(error: impl std::fmt::Display) -> skiff_compiler_source::SourceCompileError {
    skiff_compiler_source::SourceCompileError::ContractValidation {
        message: format!("test File IR lowering failed: {error}"),
    }
}

fn nested_contract_execution_type() -> TypeRefIr {
    TypeRefIr::Nullable {
        inner: Box::new(TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::PackageSymbol {
                    symbol: skiff_artifact_model::PackageSymbolRef {
                        package: skiff_artifact_model::PackageRefIr::PackageId {
                            package_id: "example.types".to_string(),
                        },
                        symbol_path: "User".to_string(),
                        abi_expectation: None,
                    },
                }),
            }],
        }),
    }
}

fn executable<'a>(unit: &'a FileIrUnit, name: &str) -> &'a ExecutableIr {
    let symbol = format!("{MODULE}.{name}");
    unit.executables
        .iter()
        .find(|executable| executable.symbol == symbol)
        .unwrap_or_else(|| panic!("missing executable `{symbol}`"))
}
