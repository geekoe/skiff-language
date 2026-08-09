use crate::{
    ast::{CallArg, DispatchTiming, Expr, Stmt},
    parser::parse_source,
};

fn start_function_body(source: &str) -> Vec<Stmt> {
    let ast = parse_source(source).expect("source should parse");
    let start = ast
        .functions
        .iter()
        .find(|function| function.name == "start")
        .expect("start function");
    start.body.statements.clone()
}

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

    let body = start_function_body(source);
    let [stmt] = body.as_slice() else {
        panic!("expected one statement");
    };
    let Stmt::Expr(Expr::Dispatch { call, timing: None }) = stmt else {
        panic!("expected dispatch expression statement, got {stmt:?}");
    };
    let Expr::Call { callee, args } = call.as_ref() else {
        panic!("expected dispatch call, got {call:?}");
    };
    assert_eq!(callee.as_ref(), &Expr::Identifier("runDrain".to_string()));
    assert_eq!(args, &vec![CallArg::Value(Expr::Identifier("threadId".to_string()))]);
}

#[test]
fn parses_dispatch_in_assignment_and_argument_positions() {
    let source = r#"
        function runDrain(threadId: ThreadId) -> void {
          return
        }

        function start(threadId: ThreadId) -> void {
          let first = dispatch runDrain(threadId)
          consume(dispatch runDrain(threadId))
        }
    "#;

    let body = start_function_body(source);
    let [first, second] = body.as_slice() else {
        panic!("expected two statements");
    };
    let Stmt::Let { value, .. } = first else {
        panic!("expected let statement, got {first:?}");
    };
    assert!(
        matches!(value, Expr::Dispatch { timing: None, .. }),
        "unexpected value {value:?}"
    );
    let Stmt::Expr(Expr::Call { args, .. }) = second else {
        panic!("expected consume call statement, got {second:?}");
    };
    assert!(
        matches!(
            args.as_slice(),
            [CallArg::Value(Expr::Dispatch { timing: None, .. })]
        ),
        "unexpected call args {args:?}"
    );
}

#[test]
fn parses_dispatch_after_duration_literal_and_at_expression() {
    let source = r#"
        function runDrain(threadId: ThreadId) -> void {
          return
        }

        function start(threadId: ThreadId, instant: Instant) -> void {
          dispatch runDrain(threadId) after(200ms)
          dispatch runDrain(threadId) at(instant)
          let zero = dispatch runDrain(threadId) after(0ms)
        }
    "#;

    let body = start_function_body(source);
    let [after, at, zero] = body.as_slice() else {
        panic!("expected three statements");
    };
    let Stmt::Expr(Expr::Dispatch {
        timing: Some(DispatchTiming::After(value)),
        ..
    }) = after
    else {
        panic!("expected dispatch after(...), got {after:?}");
    };
    // `after(200ms)` desugars to `Duration.milliseconds(200)`.
    let Expr::Call { callee, args } = value.as_ref() else {
        panic!("expected desugared Duration.milliseconds call, got {value:?}");
    };
    let Expr::Field { object, field } = callee.as_ref() else {
        panic!("expected Duration.milliseconds field, got {callee:?}");
    };
    assert_eq!(object.as_ref(), &Expr::Identifier("Duration".to_string()));
    assert_eq!(field, "milliseconds");
    assert_eq!(
        args,
        &vec![CallArg::Value(Expr::Literal(crate::ast::Literal::Number(200.0)))]
    );

    let Stmt::Expr(Expr::Dispatch {
        timing: Some(DispatchTiming::At(value)),
        ..
    }) = at
    else {
        panic!("expected dispatch at(...), got {at:?}");
    };
    assert_eq!(value.as_ref(), &Expr::Identifier("instant".to_string()));

    let Stmt::Let {
        value:
            Expr::Dispatch {
                timing: Some(DispatchTiming::After(value)),
                ..
            },
        ..
    } = zero
    else {
        panic!("expected zero-duration dispatch, got {zero:?}");
    };
    let Expr::Call { args, .. } = value.as_ref() else {
        panic!("expected zero-duration desugar, got {value:?}");
    };
    assert_eq!(args, &vec![CallArg::Value(Expr::Literal(crate::ast::Literal::Number(0.0)))]);
}

#[test]
fn rejects_dispatch_without_call_expression() {
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
            .contains("dispatch expects a call expression"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_negative_duration_literal_in_after_clause() {
    let error = parse_source(
        r#"
        function start() -> void {
          dispatch work() after(-1ms)
        }
    "#,
    )
    .expect_err("negative duration literal should be rejected");

    assert!(
        error.to_string().contains("expected expression"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_duplicate_timing_clause() {
    let error = parse_source(
        r#"
        function start(instant: Instant) -> void {
          dispatch work() after(200ms) at(instant)
        }
    "#,
    )
    .expect_err("duplicate timing clause should be rejected");

    assert!(
        error
            .to_string()
            .contains("dispatch accepts at most one timing clause"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_missing_timing_operand() {
    let error = parse_source(
        r#"
        function start() -> void {
          dispatch work() after()
        }
    "#,
    )
    .expect_err("missing timing operand should be rejected");

    assert!(
        error.to_string().contains("expected expression"),
        "unexpected error: {error}"
    );
}
