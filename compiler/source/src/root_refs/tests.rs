use super::*;
use std::path::{Path, PathBuf};

use crate::shared::ast::{
    AliasDecl, Block, BuiltinPackage, ConstDecl, Expr, FieldDecl, FunctionDecl, ImportDecl,
    InterfaceDecl, PackageId, Param, SourceFile, Stmt, TypeDecl, TypeRef,
};
use crate::shared::error::SourceSpan;
use crate::shared::parser::parse_source;
use crate::source_graph::CompilerSourceFile;

fn span() -> SourceSpan {
    SourceSpan::synthetic()
}

fn empty_block() -> Block {
    Block { statements: vec![] }
}

fn type_ref(name: &str) -> TypeRef {
    TypeRef {
        name: name.to_string(),
    }
}

fn dotted_expr(path: &[&str]) -> Expr {
    let (head, tail) = path
        .split_first()
        .expect("path must have at least one segment");
    tail.iter()
        .fold(Expr::Identifier((*head).to_string()), |object, field| {
            Expr::Field {
                object: Box::new(object),
                field: (*field).to_string(),
            }
        })
}

fn build_index_with_module(module: &str, ast: &SourceFile) -> RootRefIndex {
    let mut index = RootRefIndex::new();
    index.insert_module(module, ast);
    index
}

fn module_with_exported_type(name: &str) -> SourceFile {
    SourceFile {
        provider_capability: None,
        functions: vec![],
        function_signatures: vec![],
        imports: vec![],
        types: vec![TypeDecl {
            exported: true,
            is_native: false,
            name: name.to_string(),
            type_params: vec![],
            discriminator: None,
            alias: None,
            implements: vec![],
            fields: vec![],
            span: span(),
        }],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    }
}

#[test]
fn resolves_root_path_in_type_ref_to_canonical_path() {
    let module_ast = module_with_exported_type("UserDoc");
    let index = build_index_with_module("api.user", &module_ast);
    let mut consumer = SourceFile {
        functions: vec![FunctionDecl {
            exported: false,
            name: "f".to_string(),
            type_params: vec![],
            params: vec![Param {
                name: "doc".to_string(),
                ty: type_ref("root.api.user.UserDoc"),
            }],
            return_type: type_ref("Bool"),
            body: empty_block(),
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: None,
            span: span(),
        }],
        provider_capability: None,
        function_signatures: vec![],
        imports: vec![],
        types: vec![],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };
    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);
    assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
    assert_eq!(consumer.functions[0].params[0].ty.name, "api.user.UserDoc");
    assert_eq!(consumer.imports.len(), 0);
}

#[test]
fn resolves_root_path_to_attached_db_object_type_symbol() {
    let module_ast = parse_source(
        r#"
            type Thread {
              id: string
            }

            db object Thread {
              name "thread"
              primary key(id)
            }
        "#,
    )
    .expect("db object source should parse");
    let index = build_index_with_module("internal.models", &module_ast);
    let mut consumer = parse_source(
        r#"
            type Holder {
              thread: root.internal.models.Thread
            }
        "#,
    )
    .expect("consumer source should parse");

    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);

    assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
    assert_eq!(
        consumer.types[0].fields[0].ty.name,
        "internal.models.Thread"
    );
}

#[test]
fn resolves_root_path_in_db_query_target() {
    let module_ast = parse_source(
        r#"
            type Thread {
              id: string,
              title: string
            }

            db object Thread {
              primary key(id)
            }
        "#,
    )
    .expect("db object source should parse");
    let index = build_index_with_module("internal.models", &module_ast);
    let mut consumer = parse_source(
        r#"
            function run(enabled: bool) -> string {
              const query = db query root.internal.models.Thread {
                where if enabled { title == "x" }
              }
              return "ok"
            }
        "#,
    )
    .expect("consumer source should parse");

    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);

    assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
    let Stmt::Let { value, .. } = &consumer.functions[0].body.statements[0] else {
        panic!("expected db query binding");
    };
    let Expr::DbQuery(query) = value else {
        panic!("expected db query expression");
    };
    assert_eq!(query.target.name, "internal.models.Thread");
}

#[test]
fn resolves_root_path_in_expression_to_canonical_path() {
    let module_ast = module_with_exported_type("Foo");
    let index = build_index_with_module("a.b", &module_ast);
    let mut consumer = SourceFile {
        functions: vec![FunctionDecl {
            exported: false,
            name: "f".to_string(),
            type_params: vec![],
            params: vec![],
            return_type: type_ref("Foo"),
            body: Block {
                statements: vec![Stmt::Return(Some(Expr::Field {
                    object: Box::new(Expr::Field {
                        object: Box::new(Expr::Field {
                            object: Box::new(Expr::Identifier("root".to_string())),
                            field: "a".to_string(),
                        }),
                        field: "b".to_string(),
                    }),
                    field: "Foo".to_string(),
                }))],
            },
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: None,
            span: span(),
        }],
        provider_capability: None,
        function_signatures: vec![],
        imports: vec![],
        types: vec![],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };
    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);
    assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
    assert_eq!(
        consumer.functions[0].body.statements[0],
        Stmt::Return(Some(Expr::Field {
            object: Box::new(Expr::Field {
                object: Box::new(Expr::Identifier("a".to_string())),
                field: "b".to_string(),
            }),
            field: "Foo".to_string(),
        }))
    );
    assert_eq!(consumer.imports.len(), 0);
}

#[test]
fn read_only_resolution_matches_mutable_resolution_without_changing_ast() {
    let module_ast = module_with_exported_type("Foo");
    let index = build_index_with_module("a.b", &module_ast);
    let consumer = SourceFile {
        functions: vec![FunctionDecl {
            exported: false,
            name: "f".to_string(),
            type_params: vec![],
            params: vec![Param {
                name: "doc".to_string(),
                ty: type_ref("root.a.b.Foo"),
            }],
            return_type: type_ref("Array<root.a.b.Foo>"),
            body: Block {
                statements: vec![
                    Stmt::Let {
                        mutable: false,
                        name: "local".to_string(),
                        ty: Some(type_ref("root.a.b.Foo")),
                        value: dotted_expr(&["root", "a", "b", "Foo"]),
                    },
                    Stmt::Expr(dotted_expr(&["package", "a", "b", "Foo"])),
                ],
            },
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: None,
            span: span(),
        }],
        provider_capability: None,
        function_signatures: vec![],
        imports: vec![],
        types: vec![],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };
    let original = consumer.clone();

    let read_only_outcome = collect_root_refs_in_ast(&consumer, &index);

    assert_eq!(consumer, original);

    let mut mutable_consumer = consumer.clone();
    let mutable_outcome = resolve_root_refs_in_ast(&mut mutable_consumer, &index);

    assert_eq!(read_only_outcome.errors, mutable_outcome.errors);
    assert_eq!(
        read_only_outcome.synthetic_imports,
        mutable_outcome.synthetic_imports
    );
    assert_ne!(mutable_consumer, original);
}

#[test]
fn unknown_module_produces_error() {
    let index = RootRefIndex::new();
    let mut consumer = SourceFile {
        provider_capability: None,
        functions: vec![],
        function_signatures: vec![],
        imports: vec![],
        types: vec![TypeDecl {
            exported: false,
            is_native: false,
            name: "Holder".to_string(),
            type_params: vec![],
            discriminator: None,
            alias: Some(type_ref("root.does.not.exist")),
            implements: vec![],
            fields: vec![],
            span: span(),
        }],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };
    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);
    assert_eq!(outcome.errors.len(), 1);
    match &outcome.errors[0].reason {
        RootRefErrorReason::UnknownModule { module_path } => {
            assert_eq!(module_path, "does.not");
        }
        other => panic!("unexpected error reason: {:?}", other),
    }
}

#[test]
fn unknown_symbol_produces_error() {
    let module_ast = module_with_exported_type("Real");
    let index = build_index_with_module("m", &module_ast);
    let mut consumer = SourceFile {
        provider_capability: None,
        functions: vec![],
        function_signatures: vec![],
        imports: vec![],
        types: vec![TypeDecl {
            exported: false,
            is_native: false,
            name: "Holder".to_string(),
            type_params: vec![],
            discriminator: None,
            alias: Some(type_ref("root.m.Missing")),
            implements: vec![],
            fields: vec![],
            span: span(),
        }],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };
    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);
    assert_eq!(outcome.errors.len(), 1);
    match &outcome.errors[0].reason {
        RootRefErrorReason::UnknownSymbol {
            module_path,
            symbol,
        } => {
            assert_eq!(module_path, "m");
            assert_eq!(symbol, "Missing");
        }
        other => panic!("unexpected error reason: {:?}", other),
    }
}

#[test]
fn does_not_duplicate_existing_import() {
    let module_ast = module_with_exported_type("Foo");
    let index = build_index_with_module("a.b", &module_ast);
    let mut consumer = SourceFile {
        provider_capability: None,
        functions: vec![],
        function_signatures: vec![],
        imports: vec![ImportDecl {
            path: vec!["a".to_string(), "b".to_string(), "Foo".to_string()],
            alias: None,
            package: None,
            local_binding: None,
            span: span(),
        }],
        types: vec![TypeDecl {
            exported: false,
            is_native: false,
            name: "Holder".to_string(),
            type_params: vec![],
            discriminator: None,
            alias: Some(type_ref("root.a.b.Foo")),
            implements: vec![],
            fields: vec![],
            span: span(),
        }],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };
    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);
    assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
    assert_eq!(consumer.imports.len(), 1);
}

#[test]
fn module_exports_collects_all_kinds() {
    let mut ast = module_with_exported_type("MyType");
    ast.interfaces.push(InterfaceDecl {
        exported: true,
        name: "MyInterface".to_string(),
        type_params: vec![],
        operations: vec![],
        span: span(),
    });
    ast.functions.push(FunctionDecl {
        exported: true,
        name: "myFn".to_string(),
        type_params: vec![],
        params: vec![],
        return_type: type_ref("Bool"),
        body: empty_block(),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
        span: span(),
    });
    ast.consts.push(ConstDecl {
        exported: true,
        name: "MY_CONST".to_string(),
        ty: None,
        value: Expr::Literal(crate::shared::ast::Literal::Bool(true)),
        span: span(),
    });
    ast.aliases.push(AliasDecl {
        exported: true,
        name: "MyAlias".to_string(),
        target_type: type_ref("MyType"),
        span: span(),
    });
    let index = build_index_with_module("m", &ast);
    let exports = index.module_exports("m").expect("module present");
    assert!(exports.contains("MyType"));
    assert!(exports.contains("MyAlias"));
    assert!(exports.contains("MyInterface"));
    assert!(exports.contains("myFn"));
    assert!(exports.contains("MY_CONST"));
    assert!(!exports.contains("NotExported"));
}

#[test]
fn resolves_root_path_in_alias_target_type() {
    let module_ast = module_with_exported_type("UserDoc");
    let index = build_index_with_module("api.user", &module_ast);
    let mut consumer = SourceFile {
        provider_capability: None,
        functions: vec![],
        function_signatures: vec![],
        imports: vec![],
        types: vec![],
        aliases: vec![AliasDecl {
            exported: false,
            name: "Docs".to_string(),
            target_type: type_ref("Array<root.api.user.UserDoc>"),
            span: span(),
        }],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };

    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);

    assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
    assert_eq!(
        consumer.aliases[0].target_type.name,
        "Array<api.user.UserDoc>"
    );
}

#[test]
fn package_spelling_is_rejected() {
    let module_ast = module_with_exported_type("UserDoc");
    let index = build_index_with_module("api.user", &module_ast);
    let mut consumer = SourceFile {
        functions: vec![FunctionDecl {
            exported: false,
            name: "f".to_string(),
            type_params: vec![],
            params: vec![Param {
                name: "doc".to_string(),
                ty: type_ref("package.api.user.UserDoc"),
            }],
            return_type: type_ref("Bool"),
            body: empty_block(),
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: None,
            span: span(),
        }],
        provider_capability: None,
        function_signatures: vec![],
        imports: vec![],
        types: vec![],
        aliases: vec![],
        interfaces: vec![],
        impls: vec![],
        dbs: vec![],
        consts: vec![],
        tests: vec![],
        test_default_run: None,
        test_default_run_span: None,
        source_spans: Default::default(),
    };
    let outcome = resolve_root_refs_in_ast(&mut consumer, &index);
    assert_eq!(outcome.errors.len(), 1);
    assert_eq!(
        outcome.errors[0].reason,
        RootRefErrorReason::RemovedPackageSyntax
    );
    assert_eq!(outcome.errors[0].path, "package.api.user.UserDoc");
}

// Suppress unused-import warnings under cfg(test) for items only referenced from doc comments.
#[allow(dead_code)]
fn _touch(_: BuiltinPackage, _: PackageId, _: FieldDecl) {}

#[test]
fn text_validation_collects_synthetic_import_without_rewriting_source() {
    let module_ast = module_with_exported_type("UserDoc");
    let index = build_index_with_module("api.user", &module_ast);
    let source = "fn use(x: root.api.user.UserDoc) -> Bool { return true }\n";
    let outcome = validate_root_refs_in_text(source, &index).unwrap();
    assert!(outcome.errors.is_empty(), "errors: {:?}", outcome.errors);
    assert_eq!(outcome.synthetic_imports.len(), 1);
    let (module, symbol) = outcome.synthetic_imports.iter().next().unwrap();
    assert_eq!(module, "api.user");
    assert_eq!(symbol, "UserDoc");
}

#[test]
fn text_validation_skips_chains_in_strings() {
    let module_ast = module_with_exported_type("Foo");
    let index = build_index_with_module("a.b", &module_ast);
    let source = "const greet: String = \"root.a.b.Foo is special\"\n";
    let outcome = validate_root_refs_in_text(source, &index).unwrap();
    assert!(outcome.errors.is_empty());
    assert!(outcome.synthetic_imports.is_empty());
}

#[test]
fn text_validation_skips_root_property_names() {
    let module_ast = module_with_exported_type("Foo");
    let index = build_index_with_module("a.b", &module_ast);
    let source = "const value = config.root.a.b.Foo\n";
    let outcome = validate_root_refs_in_text(source, &index).unwrap();
    assert!(outcome.errors.is_empty());
    assert!(outcome.synthetic_imports.is_empty());
}

fn compiler_source(
    relative_path: &str,
    module_path: &str,
    is_test_file: bool,
    text: &str,
) -> CompilerSourceFile {
    CompilerSourceFile::from_parsed_ast(
        PathBuf::from(relative_path),
        module_path.to_string(),
        false,
        is_test_file,
        text.to_string(),
        parse_source(text).expect("test source should parse"),
    )
}

#[test]
fn service_policy_skips_invalid_test_file_root_refs() {
    let sources = vec![
        compiler_source(
            "internal/main.skiff",
            "internal.main",
            false,
            "type Main {}\n",
        ),
        compiler_source(
            "internal/main.test.skiff",
            "internal.main_test",
            true,
            r#"
                test "invalid root ref" {
                  let _missing: root.internal.missing.Helper = root.internal.missing.Helper {}
                  assert true
                }
            "#,
        ),
    ];

    validate_source_root_refs(
        Path::new("/tmp/service-policy-skip-test"),
        &sources,
        RootRefValidationPolicy::service_sources(),
    )
    .expect("service policy should ignore invalid test-file root refs");

    let parsed_error = validate_source_root_refs(
        Path::new("/tmp/parsed-policy-includes-test"),
        &sources,
        RootRefValidationPolicy::parsed_publication_sources(),
    )
    .expect_err("parsed publication policy should include test-file root refs")
    .to_string();
    assert!(parsed_error.contains("root.internal.missing.Helper"));
}

#[test]
fn service_policy_excludes_test_only_symbols_from_root_index() {
    let sources = vec![
        compiler_source(
            "internal/main.skiff",
            "internal.main",
            false,
            "type Main { helper: root.internal.test_only.Helper }\n",
        ),
        compiler_source(
            "internal/test_only.test.skiff",
            "internal.test_only",
            true,
            "type Helper {}\n",
        ),
    ];

    let service_error = validate_source_root_refs(
        Path::new("/tmp/service-policy-test-symbols"),
        &sources,
        RootRefValidationPolicy::service_sources(),
    )
    .expect_err("service policy should not index test-only symbols")
    .to_string();
    assert!(service_error.contains("root.internal.test_only.Helper"));

    validate_source_root_refs(
        Path::new("/tmp/parsed-policy-test-symbols"),
        &sources,
        RootRefValidationPolicy::parsed_publication_sources(),
    )
    .expect("parsed publication policy should index included test sources");
}

#[test]
fn std_projection_roots_are_policy_controlled() {
    let sources = vec![
        compiler_source(
            "log.skiff",
            "std.log",
            false,
            "type LogEntry { event: root.telemetry.Event }\n",
        ),
        compiler_source("telemetry.skiff", "std.telemetry", false, "type Event {}\n"),
    ];

    let service_error = validate_source_root_refs(
        Path::new("/tmp/service-policy-std-root"),
        &sources,
        RootRefValidationPolicy::service_sources(),
    )
    .expect_err("service policy should not add stripped std projection roots")
    .to_string();
    assert!(service_error.contains("root.telemetry.Event"));

    validate_source_root_refs(
        Path::new("/tmp/parsed-policy-std-root"),
        &sources,
        RootRefValidationPolicy::parsed_publication_sources(),
    )
    .expect("parsed publication policy should add stripped std projection roots");
}
