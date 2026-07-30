use skiff_compiler_source::source_graph::{CompilerSourceFile, PublicationSourceGraph};
use skiff_syntax::ast::{
    Block, BlockSourceSpans, DependencySourceAddress, ExecutableSourceSpans, Expr, ExprSourceSpans,
    FunctionDecl, Literal, Param, SourceFile, Stmt, StmtSourceSpans, TestDeclaration,
    TestEffectOutcome, TestEffectOutcomeSourceSpans, TestEffectStepOutcome,
    TestEffectStepOutcomeSourceSpans, TypeRef,
};

use crate::input::compile_input::PackageCompileInput;
use crate::shared::package_compile_error::PackageCompileError;

/// Builds the ordinary source set for one package compilation.
///
/// Production compilation excludes test files. A test service receives the
/// same production files plus compiler-owned transformations of every
/// `*.test.skiff` file. The transformed files are ordinary private source
/// modules; no package overlay or alternate artifact shape is involved.
pub(super) fn compile_sources(
    input: &PackageCompileInput<'_>,
) -> Result<Vec<CompilerSourceFile>, PackageCompileError> {
    compile_graph_sources(&input.package.source_graph, input.is_test_service())
}

fn compile_graph_sources(
    source_graph: &PublicationSourceGraph,
    is_test_service: bool,
) -> Result<Vec<CompilerSourceFile>, PackageCompileError> {
    let mut sources = source_graph.production_files();
    if is_test_service {
        for source in source_graph
            .files()
            .iter()
            .filter(|source| source.is_test_file)
        {
            sources.push(transform_test_source(source)?);
        }
    }
    Ok(sources)
}

fn transform_test_source(
    source: &CompilerSourceFile,
) -> Result<CompilerSourceFile, PackageCompileError> {
    reject_compiler_owned_test_function_collisions(source)?;
    let ast = test_service_ast(&source.ast);
    Ok(CompilerSourceFile::from_parsed_ast(
        source.relative_path.clone(),
        format!("{}.__test", source.module_path),
        source.role().is_contract(),
        false,
        source.text.clone(),
        ast,
    ))
}

fn reject_compiler_owned_test_function_collisions(
    source: &CompilerSourceFile,
) -> Result<(), PackageCompileError> {
    for test_index in 0..source.ast.tests.len() {
        let base = format!("skiffTestCase{test_index}");
        for generated_name in [
            base.clone(),
            format!("{base}Setup"),
            format!("{base}Gateway"),
        ] {
            if source
                .ast
                .functions
                .iter()
                .any(|function| function.name == generated_name)
                || source
                    .ast
                    .function_signatures
                    .iter()
                    .any(|function| function.name == generated_name)
            {
                return Err(PackageCompileError::ContractValidation {
                    message: format!(
                        "test source {} declares compiler-owned test function {}; rename the user declaration",
                        source.relative_path.display(),
                        generated_name
                    ),
                });
            }
        }
    }
    Ok(())
}

fn test_service_ast(ast: &SourceFile) -> SourceFile {
    let generated = ast
        .tests
        .iter()
        .enumerate()
        .map(|(test_index, test)| {
            test_service_functions_for_case(
                test,
                ast.source_spans.tests.get(test_index),
                test_index,
            )
        })
        .collect::<Vec<_>>();
    let mut transformed = ast.clone();
    transformed.tests.clear();
    transformed.test_default_run = None;
    transformed.test_default_run_span = None;
    transformed.source_spans.tests.clear();
    for ((setup_spans, setup), (body_spans, body), (gateway_spans, gateway)) in generated {
        if let Some(spans) = setup_spans {
            transformed.source_spans.functions.push(spans);
        }
        transformed.functions.push(setup);
        if let Some(spans) = body_spans {
            transformed.source_spans.functions.push(spans);
        }
        transformed.functions.push(body);
        if let Some(spans) = gateway_spans {
            transformed.source_spans.functions.push(spans);
        }
        transformed.functions.push(gateway);
    }
    transformed
}

type GeneratedTestFunction = (Option<ExecutableSourceSpans>, FunctionDecl);
type GeneratedTestFunctions = (
    GeneratedTestFunction,
    GeneratedTestFunction,
    GeneratedTestFunction,
);

fn test_service_functions_for_case(
    test: &TestDeclaration,
    source_spans: Option<&ExecutableSourceSpans>,
    test_index: usize,
) -> GeneratedTestFunctions {
    let function_name = format!("skiffTestCase{test_index}");
    (
        test_setup(test, source_spans, &function_name),
        test_body(test, source_spans, &function_name),
        test_gateway(test, source_spans, &function_name),
    )
}

fn test_setup(
    test: &TestDeclaration,
    source_spans: Option<&ExecutableSourceSpans>,
    function_name: &str,
) -> GeneratedTestFunction {
    let (statements, statement_spans) = test_setup_statements(test, source_spans);
    let function = FunctionDecl {
        exported: false,
        name: format!("{function_name}Setup"),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRef {
            name: "void".to_string(),
        },
        body: Block { statements },
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
        span: test.span,
    };
    let spans = source_spans.map(|_| ExecutableSourceSpans {
        effects: Vec::new(),
        body: BlockSourceSpans {
            span: test.span,
            statements: statement_spans,
        },
    });
    (spans, function)
}

fn test_setup_statements(
    test: &TestDeclaration,
    source_spans: Option<&ExecutableSourceSpans>,
) -> (Vec<Stmt>, Vec<StmtSourceSpans>) {
    let mut statements = Vec::new();
    let mut statement_spans = Vec::new();
    for (effect_index, effect) in test.effects.iter().enumerate() {
        let effect_spans = source_spans.and_then(|spans| spans.effects.get(effect_index));
        let steps = flattened_test_effect_steps(&effect.outcome);
        let step_spans = effect_spans
            .map(|spans| flattened_test_effect_step_spans(&spans.outcome))
            .unwrap_or_default();
        for (step_index, (step_expect, outcome)) in steps.into_iter().enumerate() {
            statements.push(Stmt::CompilerTestEffectRegister {
                target: effect.target.clone(),
                target_probe: test_effect_target_probe(&effect.target),
                declaration_start: step_index == 0,
                expect: (step_index == 0).then(|| effect.expect.clone()).flatten(),
                step_expect,
                outcome,
            });
            if let Some(effect_spans) = effect_spans {
                let mut expressions = vec![test_effect_target_span(effect.span)];
                if step_index == 0 {
                    expressions.extend(effect_spans.expect.iter().cloned());
                }
                let step_spans = step_spans
                    .get(step_index)
                    .expect("test effect AST and source spans stay aligned");
                expressions.extend(step_spans.0.iter().cloned());
                match &step_spans.1 {
                    TestEffectStepOutcomeSourceSpans::Respond(span)
                    | TestEffectStepOutcomeSourceSpans::Throw(span) => {
                        expressions.push(span.clone());
                    }
                    TestEffectStepOutcomeSourceSpans::Stream(spans) => {
                        expressions.extend(spans.iter().cloned());
                    }
                }
                statement_spans.push(StmtSourceSpans {
                    span: effect.span,
                    expressions,
                    blocks: Vec::new(),
                });
            }
        }
    }
    (statements, statement_spans)
}

fn test_effect_target_probe(target: &str) -> Expr {
    Expr::Call {
        callee: Box::new(match target.split_once('/') {
            Some((dependency_ref, public_path)) => {
                Expr::DependencySourceAddress(DependencySourceAddress {
                    dependency_ref: dependency_ref.to_string(),
                    public_path: public_path.to_string(),
                })
            }
            None => Expr::Identifier(target.to_string()),
        }),
        args: Vec::new(),
    }
}

fn test_effect_target_span(span: skiff_syntax::error::SourceSpan) -> ExprSourceSpans {
    ExprSourceSpans {
        span,
        children: vec![ExprSourceSpans {
            span,
            children: Vec::new(),
            blocks: Vec::new(),
            record_fields: Vec::new(),
        }],
        blocks: Vec::new(),
        record_fields: Vec::new(),
    }
}

fn test_body(
    test: &TestDeclaration,
    source_spans: Option<&ExecutableSourceSpans>,
    function_name: &str,
) -> GeneratedTestFunction {
    let setup_call = Expr::Call {
        callee: Box::new(Expr::Identifier(format!("{function_name}Setup"))),
        args: Vec::new(),
    };
    let mut statements = Vec::with_capacity(test.body.statements.len() + 1);
    statements.push(Stmt::Expr(setup_call));
    statements.extend(test.body.statements.clone());
    let function = FunctionDecl {
        exported: false,
        name: function_name.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRef {
            name: "void".to_string(),
        },
        body: Block { statements },
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
        span: test.span,
    };
    let spans = source_spans.map(|spans| {
        let mut statements = Vec::with_capacity(spans.body.statements.len() + 1);
        statements.push(StmtSourceSpans {
            span: test.span,
            expressions: vec![test_effect_target_span(test.span)],
            blocks: Vec::new(),
        });
        statements.extend(spans.body.statements.iter().cloned());
        ExecutableSourceSpans {
            effects: Vec::new(),
            body: BlockSourceSpans {
                span: spans.body.span,
                statements,
            },
        }
    });
    (spans, function)
}

fn test_gateway(
    test: &TestDeclaration,
    source_spans: Option<&ExecutableSourceSpans>,
    function_name: &str,
) -> GeneratedTestFunction {
    let function = FunctionDecl {
        exported: false,
        name: format!("{function_name}Gateway"),
        type_params: Vec::new(),
        params: vec![Param {
            name: "body".to_string(),
            ty: TypeRef {
                name: "null".to_string(),
            },
        }],
        return_type: TypeRef {
            name: "null".to_string(),
        },
        body: Block {
            statements: vec![
                Stmt::Expr(Expr::Call {
                    callee: Box::new(Expr::Identifier(function_name.to_string())),
                    args: Vec::new(),
                }),
                Stmt::Return(Some(Expr::Literal(Literal::Null))),
            ],
        },
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
        span: test.span,
    };
    let spans = source_spans.map(|spans| ExecutableSourceSpans {
        effects: Vec::new(),
        body: BlockSourceSpans {
            span: spans.body.span,
            statements: vec![
                StmtSourceSpans {
                    span: test.span,
                    expressions: vec![test_effect_target_span(test.span)],
                    blocks: Vec::new(),
                },
                StmtSourceSpans {
                    span: test.span,
                    expressions: vec![ExprSourceSpans {
                        span: test.span,
                        children: Vec::new(),
                        blocks: Vec::new(),
                        record_fields: Vec::new(),
                    }],
                    blocks: Vec::new(),
                },
            ],
        },
    });
    (spans, function)
}

fn flattened_test_effect_steps(
    outcome: &TestEffectOutcome,
) -> Vec<(Option<Expr>, TestEffectStepOutcome)> {
    match outcome {
        TestEffectOutcome::Respond { value } => vec![(
            None,
            TestEffectStepOutcome::Respond {
                value: value.clone(),
            },
        )],
        TestEffectOutcome::Throw { value } => vec![(
            None,
            TestEffectStepOutcome::Throw {
                value: value.clone(),
            },
        )],
        TestEffectOutcome::Stream { events } => vec![(
            None,
            TestEffectStepOutcome::Stream {
                events: events.clone(),
            },
        )],
        TestEffectOutcome::Sequence { steps } => steps
            .iter()
            .map(|step| (step.expect.clone(), step.outcome.clone()))
            .collect(),
    }
}

fn flattened_test_effect_step_spans(
    outcome: &TestEffectOutcomeSourceSpans,
) -> Vec<(Option<ExprSourceSpans>, TestEffectStepOutcomeSourceSpans)> {
    match outcome {
        TestEffectOutcomeSourceSpans::Respond(value) => vec![(
            None,
            TestEffectStepOutcomeSourceSpans::Respond(value.clone()),
        )],
        TestEffectOutcomeSourceSpans::Throw(value) => {
            vec![(None, TestEffectStepOutcomeSourceSpans::Throw(value.clone()))]
        }
        TestEffectOutcomeSourceSpans::Stream(events) => vec![(
            None,
            TestEffectStepOutcomeSourceSpans::Stream(events.clone()),
        )],
        TestEffectOutcomeSourceSpans::Sequence { steps } => steps
            .iter()
            .map(|step| (step.expect.clone(), step.outcome.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
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
}
