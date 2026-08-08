//! Phase 0 baseline: lock binary expression associativity and precedence so the
//! Phase 1+ expression refactor cannot change semantics silently.
//!
//! Reference: `doc/reference/syntax.md` section 7. The parser uses precedence
//! climbing (`parse_binary(min_prec)`
//! with `prec + 1` tightening the right operand), so every operator is left
//! associative and higher-precedence operators bind tighter.

use crate::ast::{BinaryOp, Expr, Stmt};
use crate::parser::parse_source;

fn parse_return_expression(source: &str) -> Expr {
    let wrapped = format!("function run() -> void {{ return {source} }}");
    let ast = parse_source(&wrapped).expect("binary expression should parse");
    let [Stmt::Return(Some(expr))] = ast.functions[0].body.statements.as_slice() else {
        panic!(
            "expected a single return expression statement, got {:?}",
            ast.functions[0].body.statements
        );
    };
    expr.clone()
}

fn assert_identifier(expr: &Expr, expected: &str) {
    assert_eq!(expr, &Expr::Identifier(expected.to_string()));
}

fn assert_binary(expr: &Expr, expected_op: BinaryOp, left: impl Fn(&Expr), right: impl Fn(&Expr)) {
    match expr {
        Expr::Binary {
            op,
            left: left_expr,
            right: right_expr,
        } => {
            assert_eq!(*op, expected_op, "unexpected binary operator");
            left(left_expr);
            right(right_expr);
        }
        other => panic!("expected binary {expected_op:?}, got {other:?}"),
    }
}

#[test]
fn subtraction_is_left_associative() {
    assert_binary(
        &parse_return_expression("a - b - c"),
        BinaryOp::Sub,
        |left| {
            assert_binary(
                left,
                BinaryOp::Sub,
                |inner| assert_identifier(inner, "a"),
                |inner| assert_identifier(inner, "b"),
            );
        },
        |right| assert_identifier(right, "c"),
    );
}

#[test]
fn division_is_left_associative() {
    assert_binary(
        &parse_return_expression("a / b / c"),
        BinaryOp::Div,
        |left| {
            assert_binary(
                left,
                BinaryOp::Div,
                |inner| assert_identifier(inner, "a"),
                |inner| assert_identifier(inner, "b"),
            );
        },
        |right| assert_identifier(right, "c"),
    );
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    assert_binary(
        &parse_return_expression("a + b * c"),
        BinaryOp::Add,
        |left| assert_identifier(left, "a"),
        |right| {
            assert_binary(
                right,
                BinaryOp::Mul,
                |inner| assert_identifier(inner, "b"),
                |inner| assert_identifier(inner, "c"),
            );
        },
    );
}

#[test]
fn and_binds_tighter_than_or() {
    assert_binary(
        &parse_return_expression("a && b || c"),
        BinaryOp::Or,
        |left| {
            assert_binary(
                left,
                BinaryOp::And,
                |inner| assert_identifier(inner, "a"),
                |inner| assert_identifier(inner, "b"),
            );
        },
        |right| assert_identifier(right, "c"),
    );
}

#[test]
fn relational_binds_tighter_than_equality_and_chain_is_left_associative() {
    // `a < b == c` must parse as `(a < b) == c`; if precedence changed to
    // equality-first the shape would become `a < (b == c)`.
    assert_binary(
        &parse_return_expression("a < b == c"),
        BinaryOp::Eq,
        |left| {
            assert_binary(
                left,
                BinaryOp::Lt,
                |inner| assert_identifier(inner, "a"),
                |inner| assert_identifier(inner, "b"),
            );
        },
        |right| assert_identifier(right, "c"),
    );
}
