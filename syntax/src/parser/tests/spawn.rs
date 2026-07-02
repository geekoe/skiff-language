use crate::{
    ast::{Expr, Stmt},
    parser::parse_source,
};

#[test]
fn parses_spawn_call_statement() {
    let source = r#"
        function runDrain(threadId: ThreadId) -> void {
          return
        }

        function start(threadId: ThreadId) -> void {
          spawn runDrain(threadId)
        }
    "#;

    let ast = parse_source(source).expect("spawn statement should parse");
    let start = ast
        .functions
        .iter()
        .find(|function| function.name == "start")
        .expect("start function");
    let [stmt] = start.body.statements.as_slice() else {
        panic!("expected one statement");
    };

    let Stmt::Spawn {
        call: Expr::Call { callee, args },
    } = stmt
    else {
        panic!("expected spawn call statement, got {stmt:?}");
    };

    assert_eq!(callee.as_ref(), &Expr::Identifier("runDrain".to_string()));
    assert_eq!(args, &vec![Expr::Identifier("threadId".to_string())]);
}

#[test]
fn rejects_spawn_in_expression_position() {
    let error = parse_source(
        r#"
        function start() -> number {
          const value = spawn runDrain()
          return value
        }
    "#,
    )
    .expect_err("spawn should not parse as an expression");

    assert!(
        error
            .to_string()
            .contains("spawn is a statement and cannot be used as an expression"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_spawn_statement_without_call_expression() {
    let error = parse_source(
        r#"
        function start(runDrain: number) -> void {
          spawn runDrain
        }
    "#,
    )
    .expect_err("spawn should require a call expression");

    assert!(
        error
            .to_string()
            .contains("spawn statement expects a call expression"),
        "unexpected error: {error}"
    );
}
