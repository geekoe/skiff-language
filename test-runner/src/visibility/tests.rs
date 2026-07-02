use super::*;
use std::path::PathBuf;

use skiff_compiler::test_support::{TestPackageApiEntry, TestPackageManifest};
use skiff_syntax::parser::parse_source;

#[test]
fn type_name_candidates_extracts_union_nullable_and_string_literal_names() {
    assert_eq!(
        type_name_candidates(r#"User | Error? | "ok" | string"#),
        vec!["User", "Error"]
    );
}

#[test]
fn type_name_candidates_extracts_generic_names_in_order_without_builtins() {
    assert_eq!(
        type_name_candidates("Result<Array<User?>, Map<string, DomainError>>"),
        vec!["User", "DomainError"]
    );
}

#[test]
fn type_name_candidates_omit_date_prelude_type() {
    assert_eq!(
        type_name_candidates("Date | DomainEvent"),
        vec!["DomainEvent"]
    );
}

#[test]
fn type_name_candidates_extracts_record_field_names() {
    assert_eq!(
        type_name_candidates(r#"{ user: User, status: "ready", meta: Map<string, Metadata> }"#),
        vec!["User", "Metadata"]
    );
}

#[test]
fn type_name_candidates_extracts_function_param_and_return_names() {
    assert_eq!(
        type_name_candidates("fn(input: User, ctx: Context) -> Result<Response, Error>"),
        vec!["User", "Context", "Response", "Error"]
    );
}

#[test]
fn type_name_candidates_preserve_qualified_type_name() {
    assert_eq!(
        type_name_candidates("internal.model.User"),
        vec!["internal.model.User"]
    );
}

#[test]
fn db_target_allows_qualified_production_db_object_without_import() {
    let mut production_exports = BTreeMap::new();
    let mut symbols = ProductionModuleSymbols::default();
    insert_production_symbol(
        &mut symbols.symbols,
        "User",
        ProductionSymbolKind::DbObject,
        true,
    );
    symbols.db_objects.insert("User".to_string());
    production_exports.insert("internal.model".to_string(), symbols);

    let test_locals = TestLocalSymbols::default();

    assert_eq!(
        check_db_target_private_visibility(
            &TypeRef {
                name: "internal.model.User".to_string()
            },
            &production_exports,
            &test_locals,
        ),
        None
    );
}

#[test]
fn ordinary_type_still_requires_import_for_qualified_path() {
    let mut production_exports = BTreeMap::new();
    let mut symbols = ProductionModuleSymbols::default();
    insert_production_symbol(
        &mut symbols.symbols,
        "User",
        ProductionSymbolKind::Type,
        true,
    );
    production_exports.insert("internal.model".to_string(), symbols);

    let test_locals = TestLocalSymbols::default();

    assert_eq!(
        check_type_name_private_visibility(
            "internal.model.User",
            &production_exports,
            &test_locals,
        ),
        Some("production type internal.model.User must be imported before use".to_string())
    );
}

#[test]
fn imported_flat_dependency_alias_allows_public_export_but_not_bare_symbol() {
    let mut production_exports = BTreeMap::new();
    let mut symbols = ProductionModuleSymbols::default();
    insert_production_symbol(
        &mut symbols.symbols,
        "LlmClient",
        ProductionSymbolKind::Interface,
        true,
    );
    insert_production_symbol(
        &mut symbols.symbols,
        "LlmMessage",
        ProductionSymbolKind::Type,
        true,
    );
    production_exports.insert("example.com/llm-api".to_string(), symbols);

    let test_locals = TestLocalSymbols {
        imports: BTreeSet::from(["llmApi".to_string()]),
        package_ids: BTreeSet::from(["example.com/llm-api".to_string()]),
        package_aliases: BTreeMap::from([(
            "llmApi".to_string(),
            vec!["LlmClient".to_string(), "LlmMessage".to_string()],
        )]),
        ..Default::default()
    };

    assert_eq!(
        private_visibility_error_for_symbol_path(
            "llmApi.LlmClient",
            &production_exports,
            &test_locals,
            SymbolUseKind::Type,
        ),
        None
    );
    assert_eq!(
        private_visibility_error_for_symbol_path(
            "llmApi.LlmMessage",
            &production_exports,
            &test_locals,
            SymbolUseKind::Type,
        ),
        None
    );
    assert_eq!(
        private_visibility_error_for_symbol_path(
            "LlmClient",
            &production_exports,
            &test_locals,
            SymbolUseKind::Type,
        ),
        Some(
            "production interface example.com/llm-api.LlmClient must be accessed through an imported module"
                .to_string()
        )
    );
    assert_eq!(
        private_visibility_error_for_symbol_path(
            "LlmMessage",
            &production_exports,
            &test_locals,
            SymbolUseKind::Type,
        ),
        Some(
            "production type example.com/llm-api.LlmMessage must be accessed through an imported module"
                .to_string()
        )
    );
}

#[test]
fn imported_nested_dependency_alias_allows_public_export() {
    let mut production_exports = BTreeMap::new();
    let mut symbols = ProductionModuleSymbols::default();
    insert_production_symbol(
        &mut symbols.symbols,
        "runtimeState",
        ProductionSymbolKind::Function,
        true,
    );
    production_exports.insert("thread".to_string(), symbols);

    let test_locals = TestLocalSymbols {
        imports: BTreeSet::from(["agent".to_string()]),
        package_ids: BTreeSet::from(["example.com/agent".to_string()]),
        package_aliases: BTreeMap::from([(
            "agent".to_string(),
            vec!["thread.runtimeState".to_string()],
        )]),
        ..Default::default()
    };

    assert_eq!(
        private_visibility_error_for_symbol_path(
            "agent.thread.runtimeState",
            &production_exports,
            &test_locals,
            SymbolUseKind::Value,
        ),
        None
    );
    assert_eq!(
        private_visibility_error_for_symbol_path(
            "thread.runtimeState",
            &production_exports,
            &test_locals,
            SymbolUseKind::Value,
        ),
        Some("production function thread.runtimeState must be imported before use".to_string())
    );
}

#[test]
fn package_surface_export_marks_current_source_symbol_exported() {
    let manifest = package_manifest(
        "example.com/main",
        "api.publicAnswer",
        "api",
        "publicAnswer",
    );
    let sources = vec![package_source(
        "api",
        "api.skiff",
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )];

    let exports = production_function_exports(&manifest, &sources, false);
    let symbol = exports
        .get("api")
        .and_then(|symbols| symbols.symbols.get("publicAnswer"))
        .expect("surface export should mark api.publicAnswer");

    assert!(symbol.exported);
}

#[test]
fn dependency_surface_export_adds_alias_relative_visibility_path() {
    let manifest = package_manifest("example.com/deplib", "depapi.answer", "depapi", "answer");
    let sources = vec![package_source(
        "depapi",
        "depapi.skiff",
        r#"
            function answer() -> string {
                return "dep"
            }
        "#,
    )];

    let exports = production_function_exports(&manifest, &sources, true);

    for module_path in ["depapi", "example.com/deplib.depapi"] {
        let symbol = exports
            .get(module_path)
            .and_then(|symbols| symbols.symbols.get("answer"))
            .expect("dependency surface export should be visible through alias and package paths");
        assert!(symbol.exported, "{module_path}.answer should be exported");
    }
}

#[test]
fn std_dependency_surface_export_marks_std_alias_visibility_path() {
    let manifest = package_manifest(SKIFF_STD_PUBLICATION_ID, "http.request", "http", "request");
    let sources = vec![package_source(
        "std.http",
        "http.skiff",
        r#"
            native function request() -> number
        "#,
    )];

    let exports = production_function_exports(&manifest, &sources, true);
    let symbol = exports
        .get("std.http")
        .and_then(|symbols| symbols.symbols.get("request"))
        .expect("std dependency export should be visible through std alias path");

    assert!(symbol.exported);
}

fn package_manifest(id: &str, path: &str, module: &str, symbol: &str) -> TestPackageManifest {
    TestPackageManifest {
        id: id.to_string(),
        version: "1.0.0".to_string(),
        api: vec![TestPackageApiEntry::source(path, module, symbol)],
        dependencies: Vec::new(),
        path: PathBuf::from("package.yml"),
        synthetic: false,
    }
}

fn package_source(module_path: &str, relative_path: &str, text: &str) -> PackageTestSource {
    PackageTestSource {
        relative_path: PathBuf::from(relative_path),
        module_path: module_path.to_string(),
        is_test_file: false,
        text: text.to_string(),
        ast: parse_source(text).expect("test source should parse"),
        synthetic_imports: BTreeSet::new(),
        friend_module_path: None,
    }
}
