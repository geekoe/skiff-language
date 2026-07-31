use std::path::PathBuf;

use skiff_compiler_source::source_graph::{CompilerSourceFile, PublicationSourceGraph};
use skiff_syntax::ast::{Stmt, TestEffectStepOutcome};
use skiff_syntax::parser::parse_source;

use super::{compile_graph_sources, transform_test_source};

#[test]
fn transforms_test_file_into_private_case_functions_in_test_module() {
    let text = r#"
            function helper() -> number { return 1 }
            test defaultRun false
            test "first" { assert helper() == 1 }
            test "second" { assert true }
        "#;
    let source = CompilerSourceFile::from_parsed_ast(
        PathBuf::from("suite.test.skiff"),
        "suite".to_string(),
        false,
        true,
        text.to_string(),
        parse_source(text).expect("test source parses"),
    );

    let transformed = transform_test_source(&source).expect("test source transforms");
    let names = transformed
        .ast
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(transformed.module_path, "suite.__test");
    assert!(!transformed.is_test_file);
    assert!(transformed.ast.tests.is_empty());
    assert_eq!(transformed.ast.test_default_run, None);
    assert_eq!(
        names,
        [
            "helper",
            "skiffTestCase0Setup",
            "skiffTestCase0",
            "skiffTestCase0Gateway",
            "skiffTestCase1Setup",
            "skiffTestCase1",
            "skiffTestCase1Gateway",
        ]
    );
}

#[test]
fn lowers_inline_effect_sequence_into_case_setup() {
    let text = r#"
            test "sequence" effects {
              remote/run {
                expect: { id: "common" },
                sequence: [
                  { expect: { step: 1 }, respond: "ok" },
                  { throw: Failure { message: "no" } },
                ],
              },
            } {
              assert true
            }
        "#;
    let source = CompilerSourceFile::from_parsed_ast(
        PathBuf::from("effects.test.skiff"),
        "effects".to_string(),
        false,
        true,
        text.to_string(),
        parse_source(text).expect("inline effects parse"),
    );

    let transformed = transform_test_source(&source).expect("test source transforms");
    let setup = transformed
        .ast
        .functions
        .iter()
        .find(|function| function.name == "skiffTestCase0Setup")
        .expect("case setup is generated");
    let [first, second] = setup.body.statements.as_slice() else {
        panic!("sequence must generate two registrations");
    };
    let Stmt::CompilerTestEffectRegister {
        target,
        declaration_start,
        expect,
        step_expect,
        outcome: TestEffectStepOutcome::Respond { .. },
        ..
    } = first
    else {
        panic!("first sequence step must be a response registration");
    };
    assert_eq!(target, "remote/run");
    assert!(*declaration_start);
    assert!(expect.is_some());
    assert!(step_expect.is_some());
    let Stmt::CompilerTestEffectRegister {
        declaration_start,
        expect,
        step_expect,
        outcome: TestEffectStepOutcome::Throw { .. },
        ..
    } = second
    else {
        panic!("second sequence step must be a throw registration");
    };
    assert!(!declaration_start);
    assert!(expect.is_none());
    assert!(step_expect.is_none());
}

#[test]
fn source_selection_excludes_tests_from_production_and_transforms_them_for_test_service() {
    let production_text = "function run() -> number { return 1 }";
    let production = CompilerSourceFile::from_parsed_ast(
        PathBuf::from("suite.skiff"),
        "suite".to_string(),
        false,
        false,
        production_text.to_string(),
        parse_source(production_text).expect("production source parses"),
    );
    let test_text = "test \"run\" { assert true }";
    let test = CompilerSourceFile::from_parsed_ast(
        PathBuf::from("suite.test.skiff"),
        "suite".to_string(),
        false,
        true,
        test_text.to_string(),
        parse_source(test_text).expect("test source parses"),
    );
    let graph = PublicationSourceGraph::from_compiler_sources(vec![production, test]);

    let production_sources =
        compile_graph_sources(&graph, false).expect("production source selection");
    assert_eq!(production_sources.len(), 1);
    assert_eq!(production_sources[0].module_path, "suite");

    let test_service_sources =
        compile_graph_sources(&graph, true).expect("test-service source selection");
    assert_eq!(test_service_sources.len(), 2);
    assert_eq!(test_service_sources[0].module_path, "suite");
    assert_eq!(test_service_sources[1].module_path, "suite.__test");
    assert!(test_service_sources[1]
        .ast
        .functions
        .iter()
        .any(|function| function.name == "skiffTestCase0Gateway"));
}

#[test]
fn rejects_user_functions_that_collide_with_compiler_owned_case_functions() {
    for function_name in [
        "skiffTestCase0",
        "skiffTestCase0Setup",
        "skiffTestCase0Gateway",
    ] {
        let text = format!(
            "function {function_name}() -> bool {{ return true }}\n\
                 test \"case\" {{ assert true }}\n"
        );
        let source = CompilerSourceFile::from_parsed_ast(
            PathBuf::from("collision.test.skiff"),
            "collision".to_string(),
            false,
            true,
            text.clone(),
            parse_source(&text).expect("collision source parses"),
        );

        let error = transform_test_source(&source)
            .expect_err("compiler-owned case functions must reject user collisions");
        assert!(
            matches!(
                &error,
                crate::shared::package_compile_error::PackageCompileError::ContractValidation { .. }
            ),
            "{error}"
        );
        assert!(error.to_string().contains(function_name), "{error}");
        assert!(
            error.to_string().contains("collision.test.skiff"),
            "{error}"
        );
    }
}
