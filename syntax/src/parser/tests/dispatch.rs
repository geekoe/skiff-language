use crate::{
    ast::{Expr, Stmt},
    parser::parse_source,
};

#[test]
fn parses_dispatch_call_statement() {
    let source = r#"
        function runDrain(threadId: ThreadId) -> void {
          return
        }

        function start(threadId: ThreadId) -> void {
          dispatch runDrain(threadId)
        }
    "#;

    let ast = parse_source(source).expect("dispatch statement should parse");
    let start = ast
        .functions
        .iter()
        .find(|function| function.name == "start")
        .expect("start function");
    let [stmt] = start.body.statements.as_slice() else {
        panic!("expected one statement");
    };

    let Stmt::Dispatch {
        call: Expr::Call { callee, args },
    } = stmt
    else {
        panic!("expected dispatch call statement, got {stmt:?}");
    };

    assert_eq!(callee.as_ref(), &Expr::Identifier("runDrain".to_string()));
    assert_eq!(args, &vec![Expr::Identifier("threadId".to_string())]);
}

#[test]
fn rejects_dispatch_in_expression_position() {
    let error = parse_source(
        r#"
        function start() -> number {
          const value = dispatch runDrain()
          return value
        }
    "#,
    )
    .expect_err("dispatch should not parse as an expression");

    assert!(
        error
            .to_string()
            .contains("dispatch is a statement and cannot be used as an expression"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_dispatch_statement_without_call_expression() {
    let error = parse_source(
        r#"
        function start(runDrain: number) -> void {
          dispatch runDrain
        }
    "#,
    )
    .expect_err("dispatch should require a call expression");

    assert!(
        error
            .to_string()
            .contains("dispatch statement expects a call expression"),
        "unexpected error: {error}"
    );
}
