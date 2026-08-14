use std::{collections::BTreeMap, path::PathBuf};

use skiff_artifact_model::{ExecutableIr, ExprIr, FileIrUnit, LiteralIr, StmtIr, TypeRefIr};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::{
    build_package_from_parsed_sources, parsed_sources::parse_publication_sources,
    prelude_registry::initialize_prelude_registry, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PackageSourceModel, SourceCompileError,
};

use super::{compile_package_source_file_ir_unit, PackageSourceLoweringInput};
use crate::callable_return_types::extend_callable_return_types_for_source;

const MODULE: &str = "internal.object_construct_lowering";

fn source_model(source_text: &str) -> PackageSourceModel {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve");
    let platform_sources = CompilerPlatformSources::new(&platform_root)
        .expect("workspace platform sources should load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry should initialize");

    let root = PathBuf::from("/object-construct-lowering");
    let source = CompilerSourceFile::parse(
        PathBuf::from("internal/object_construct_lowering.skiff"),
        MODULE.to_string(),
        false,
        false,
        source_text.to_string(),
        "internal/object_construct_lowering.skiff",
    )
    .expect("object materialization source should parse");
    let production_sources = vec![source];
    let parsed_sources = parse_publication_sources(&root, &production_sources)
        .expect("object materialization source facts should build");
    build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources,
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &BTreeMap::new(),
        package_dependencies: &[],
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new("example.com/object-construct-lowering"),
    })
    .expect("object materialization source model should build")
}

fn lowered_unit(source_text: &str) -> FileIrUnit {
    let model = source_model(source_text);
    crate::lower(&model)
        .expect("target-typed object literals should lower")
        .file_ir_units()
        .first()
        .expect("one File IR unit should be emitted")
        .clone()
}

fn lowering_error_with_expression_model(
    source_text: &str,
    expression_model_source: &str,
) -> String {
    let model = source_model(source_text);
    let expression_model = source_model(expression_model_source);
    let parsed = model
        .sources()
        .parsed_sources()
        .first()
        .expect("one object source");
    let mut callable_return_types = BTreeMap::new();
    extend_callable_return_types_for_source(
        &mut callable_return_types,
        parsed.module_path(),
        parsed.ast(),
    );
    let package_interface_methods = model.type_resolution().package_interface_method_index();

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
                external_type_symbols: model.indexes().publication_type_symbols(),
                publication_db_metadata: model.indexes().publication_db_metadata_index(),
                semantic_context: &source_context,
                source_alias_targets: model
                    .resolutions()
                    .alias_targets_for_module(parsed.module_path()),
                type_resolution: model.type_resolution(),
                expression_types: Some(expression_model.expression_types()),
                source_events: None,
                execution_semantics: Some(model.execution_semantics()),
                callable_return_types: &callable_return_types,
                executable_signatures: model.executable_signatures(),
                interface_signatures: Some(model.interface_signatures()),
                service_calls: None,
            })
            .map_err(source_error)
        })
        .expect_err("inconsistent object materialization facts should fail closed")
        .to_string()
}

fn executable<'a>(unit: &'a FileIrUnit, name: &str) -> &'a ExecutableIr {
    let symbol = format!("{MODULE}.{name}");
    unit.executables
        .iter()
        .find(|executable| executable.symbol == symbol)
        .unwrap_or_else(|| panic!("missing executable `{symbol}`"))
}

fn return_expression(executable: &ExecutableIr) -> &ExprIr {
    let entry = executable
        .body
        .blocks
        .iter()
        .find(|block| block.label == "entry")
        .expect("executable should have an entry block");
    let statement_ref = entry
        .statements
        .last()
        .expect("entry block should end in return");
    let StmtIr::Return { value: Some(value) } =
        &executable.body.statements[statement_ref.statement as usize]
    else {
        panic!("entry block should end in a value return")
    };
    &executable.body.expressions[value.expression as usize]
}

fn referenced_expression<'a>(
    executable: &'a ExecutableIr,
    expression: &skiff_artifact_model::ExprRefIr,
) -> &'a ExprIr {
    &executable.body.expressions[expression.expression as usize]
}

fn construct_fields(expression: &ExprIr) -> &BTreeMap<String, skiff_artifact_model::ExprRefIr> {
    let ExprIr::Construct { fields, .. } = expression else {
        panic!("expected Construct, found {expression:?}")
    };
    fields
}

fn assert_null(executable: &ExecutableIr, expression: &skiff_artifact_model::ExprRefIr) {
    assert!(matches!(
        referenced_expression(executable, expression),
        ExprIr::Literal {
            value: LiteralIr::Null
        }
    ));
}

fn assert_connect_result_type(type_ref: &TypeRefIr) {
    let TypeRefIr::PackageSymbol { symbol } = type_ref else {
        panic!("expected exact std WebSocketConnectResult, found {type_ref:?}");
    };
    assert_eq!(
        symbol.package,
        skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string()
        }
    );
    assert_eq!(symbol.symbol_path, "std.websocket.WebSocketConnectResult");
}

fn assert_string(
    executable: &ExecutableIr,
    expression: &skiff_artifact_model::ExprRefIr,
    expected: &str,
) {
    assert!(matches!(
        referenced_expression(executable, expression),
        ExprIr::Literal {
            value: LiteralIr::String { value }
        } if value == expected
    ));
}

#[test]
fn object_materialization_lowers_accept_reject_nullable_and_nested_nominals_to_construct() {
    let unit = lowered_unit(
        r#"
          import std

          function acceptFull() -> std.websocket.WebSocketConnectResult {
            return { tag: "accept", businessIdentity: "biz", connectionPolicy: { maxConnections: 8, overflow: "close-oldest" } }
          }

          function acceptDefaults() -> std.websocket.WebSocketConnectResult {
            return { tag: "accept" }
          }

          function reject() -> std.websocket.WebSocketConnectResult {
            return { tag: "reject", code: 403, reason: "denied" }
          }
        "#,
    );

    let accept_full = executable(&unit, "acceptFull");
    let ExprIr::Construct {
        type_ref,
        fields: accept_fields,
    } = return_expression(accept_full)
    else {
        panic!("accept object literal should lower to Construct")
    };
    assert_connect_result_type(type_ref);
    assert_eq!(
        accept_fields.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "admissionRank",
            "businessIdentity",
            "connectionPolicy",
            "tag"
        ]
    );
    assert_null(accept_full, &accept_fields["admissionRank"]);
    assert_string(accept_full, &accept_fields["tag"], "accept");
    let policy = referenced_expression(accept_full, &accept_fields["connectionPolicy"]);
    let policy_fields = construct_fields(policy);
    assert_eq!(
        policy_fields.keys().map(String::as_str).collect::<Vec<_>>(),
        ["closeCode", "closeReason", "maxConnections", "overflow"]
    );
    assert_null(accept_full, &policy_fields["closeCode"]);
    assert_null(accept_full, &policy_fields["closeReason"]);

    let accept_defaults = executable(&unit, "acceptDefaults");
    let defaults = construct_fields(return_expression(accept_defaults));
    assert_null(accept_defaults, &defaults["admissionRank"]);
    assert_null(accept_defaults, &defaults["businessIdentity"]);
    assert_null(accept_defaults, &defaults["connectionPolicy"]);
    assert_string(accept_defaults, &defaults["tag"], "accept");

    let reject = executable(&unit, "reject");
    let ExprIr::Construct {
        type_ref,
        fields: reject_fields,
    } = return_expression(reject)
    else {
        panic!("reject object literal should lower to Construct")
    };
    assert_connect_result_type(type_ref);
    assert_eq!(
        reject_fields.keys().map(String::as_str).collect::<Vec<_>>(),
        ["code", "reason", "tag"]
    );
    assert_string(reject, &reject_fields["tag"], "reject");
}

#[test]
fn exact_response_start_emit_lowers_contextual_empty_headers_as_typed_array() {
    let unit = lowered_unit(
        r#"
          import std

          function events() -> Stream<std.http.HttpResponseStreamEvent> {
            emit({ tag: "start", status: 207, headers: [] })
            emit({ tag: "end" })
            return
          }
        "#,
    );
    let executable = executable(&unit, "events");
    let (array_index, items) = executable
        .body
        .expressions
        .iter()
        .enumerate()
        .find_map(|(index, expression)| match expression {
            ExprIr::ArrayLiteral { items } => Some((index, items)),
            _ => None,
        })
        .expect("direct empty headers should lower as an array literal");
    assert!(items.is_empty());
    let TypeRefIr::Builtin { name, args } = &executable.expression_types[array_index] else {
        panic!(
            "empty headers should retain its source-owned Array type, found {:?}",
            executable.expression_types[array_index]
        )
    };
    assert_eq!(name, "Array");
    let [TypeRefIr::PackageSymbol { symbol: header }] = args.as_slice() else {
        panic!("headers should carry one exact nominal item, found {args:?}")
    };
    assert_eq!(
        header.package,
        skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string()
        }
    );
    assert_eq!(header.symbol_path, "std.http.HttpHeader");

    let (start_index, start_type, start_fields) = executable
        .body
        .expressions
        .iter()
        .enumerate()
        .find_map(|(index, expression)| match expression {
            ExprIr::Construct { type_ref, fields } if fields.contains_key("headers") => {
                Some((index, type_ref, fields))
            }
            _ => None,
        })
        .expect("start branch should lower through its source-owned union materialization");
    let TypeRefIr::PackageSymbol { symbol: event } = start_type else {
        panic!("start branch must retain the exact response event target: {start_type:?}")
    };
    assert_eq!(event.symbol_path, "std.http.HttpResponseStreamEvent");
    assert_eq!(start_fields["headers"].expression as usize, array_index);
    assert!(executable.body.statements.iter().any(|statement| {
        matches!(statement, StmtIr::Emit { value, .. } if value.expression as usize == start_index)
    }));
}

#[test]
fn object_materialization_keeps_only_map_and_json_facts_as_map_literals() {
    let unit = lowered_unit(
        r#"
          type Context { id: string }

          function contexts() -> Map<string, Context> {
            return { beta: { id: "b" }, alpha: { id: "a" } }
          }

          function jsonValue() -> Json {
            return { nested: { enabled: true }, label: "ok" }
          }

          function jsonObject() -> JsonObject {
            return { count: 2 }
          }
        "#,
    );

    let contexts = executable(&unit, "contexts");
    let ExprIr::MapLiteral { entries } = return_expression(contexts) else {
        panic!("Map target should lower to MapLiteral")
    };
    assert_eq!(
        entries.keys().map(String::as_str).collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    for value in entries.values() {
        assert!(matches!(
            referenced_expression(contexts, value),
            ExprIr::Construct { .. }
        ));
    }

    let json = executable(&unit, "jsonValue");
    let ExprIr::MapLiteral { entries } = return_expression(json) else {
        panic!("Json target should lower to MapLiteral")
    };
    assert_eq!(
        entries.keys().map(String::as_str).collect::<Vec<_>>(),
        ["label", "nested"]
    );
    assert!(matches!(
        referenced_expression(json, &entries["nested"]),
        ExprIr::MapLiteral { .. }
    ));

    assert!(matches!(
        return_expression(executable(&unit, "jsonObject")),
        ExprIr::MapLiteral { .. }
    ));
}

#[test]
fn object_materialization_missing_target_fact_fails_closed_at_source_lowering_interface() {
    let object_source = r#"
      type Context { id: string }
      function make() -> Context { return { id: "ctx" } }
    "#;
    let explicit_source = r#"
      type Context { id: string }
      function make() -> Context { return Context { id: "ctx" } }
    "#;
    let error = lowering_error_with_expression_model(object_source, explicit_source);
    assert!(
        error.contains("requires source-owned materialization fact"),
        "unexpected lowering error: {error}"
    );
}

#[test]
fn object_materialization_rejects_compatible_key_and_field_fact_with_forged_target() {
    let actual = r#"
      type Context { id: string }
      function make() -> Context { return { id: "ctx" } }
    "#;
    let forged = r#"
      type Context { id: string }
      function make() -> Map<string, string> { return { id: "ctx" } }
    "#;

    let error = lowering_error_with_expression_model(actual, forged);
    assert!(
        error.contains("resolved target") && error.contains("current expected target"),
        "forged Map materialization should fail target consistency: {error}"
    );
}

#[test]
fn object_materialization_rejects_stale_nested_and_synthetic_field_types() {
    let cases = [
        (
            r#"
              type Child { id: string }
              type Other { id: string }
              type Context { child: Child }
              function make() -> Context { return { child: { id: "ctx" } } }
            "#,
            r#"
              type Child { id: string }
              type Other { id: string }
              type Context { child: Other }
              function make() -> Context { return { child: { id: "ctx" } } }
            "#,
            "field `child` fact type",
        ),
        (
            r#"
              type Context { note: string? }
              function make() -> Context { return {} }
            "#,
            r#"
              type Context { note: integer? }
              function make() -> Context { return {} }
            "#,
            "field `note` fact type",
        ),
    ];

    for (actual, stale, expected) in cases {
        let error = lowering_error_with_expression_model(actual, stale);
        assert!(
            error.contains(expected),
            "stale materialization should report {expected:?}: {error}"
        );
    }
}

fn source_error(error: impl std::fmt::Display) -> SourceCompileError {
    SourceCompileError::ContractValidation {
        message: error.to_string(),
    }
}
