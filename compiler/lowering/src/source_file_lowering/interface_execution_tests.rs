use std::{collections::BTreeMap, path::PathBuf};

use skiff_artifact_model::{ContractTypeId, FileIrUnit, TypeRefIr};
use skiff_compiler_source::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PackageSourceModel,
    PublicationTypeSymbolIndex, SourceDependencyAnalysisInput, SourceSymbolKey,
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
    let (model, contract_type_id) = contract_interface_model();
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
    assert_eq!(
        operation.params[0].ty,
        TypeRefIr::TypeParam {
            name: "Self".to_string()
        }
    );
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
        contract_type_id.as_str(),
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

fn contract_interface_model() -> (PackageSourceModel, ContractTypeId) {
    let (dependency, contract_type_id) = contract_dependency();
    let dependency_analysis = SourceDependencyAnalysisInput::new(Vec::new(), [dependency]).unwrap();
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
            ) -> Array<payments.User?>?
          }

          type Handler implements Gateway {}
          impl Handler {
            function echo(
              self: Handler,
              input: Array<payments.User?>?
            ) -> Array<payments.User?>? {
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
            platform_package_authority: None,
        },
        &dependency_analysis,
    )
    .expect("contract interface source model should build");
    (model, contract_type_id)
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
        inner: Box::new(TypeRefIr::Native {
            name: "Array".to_string(),
            args: vec![TypeRefIr::Nullable {
                inner: Box::new(TypeRefIr::native("unknown")),
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
