use crate::{
    ast::{Expr, Literal, Stmt},
    parser::parse_source,
};

fn assert_array_items(expr: &Expr, expected: &[Expr]) {
    let Expr::ArrayLiteral { items } = expr else {
        panic!("expected array literal, got {expr:?}");
    };
    assert_eq!(items, expected);
}

#[test]
fn parses_non_empty_array_literal() {
    let ast = parse_source(
        r#"
function run() -> Array<number> {
  return [1, 2, 3]
}
"#,
    )
    .expect("array literal should parse");

    let [Stmt::Return(Some(array))] = ast.functions[0].body.statements.as_slice() else {
        panic!("expected return array literal");
    };
    assert_array_items(
        array,
        &[
            Expr::Literal(Literal::Number(1.0)),
            Expr::Literal(Literal::Number(2.0)),
            Expr::Literal(Literal::Number(3.0)),
        ],
    );
}

#[test]
fn parses_empty_array_literal() {
    let ast = parse_source(
        r#"
function run() -> Array<number> {
  return []
}
"#,
    )
    .expect("empty array literal should parse");

    let [Stmt::Return(Some(array))] = ast.functions[0].body.statements.as_slice() else {
        panic!("expected return array literal");
    };
    assert_array_items(array, &[]);
}

#[test]
fn parses_nested_array_literal() {
    let ast = parse_source(
        r#"
function run() -> Array<Array<number>> {
  return [[1], [2, [3]]]
}
"#,
    )
    .expect("nested array literal should parse");

    let [Stmt::Return(Some(Expr::ArrayLiteral { items }))] =
        ast.functions[0].body.statements.as_slice()
    else {
        panic!("expected return array literal");
    };
    let [first, second] = items.as_slice() else {
        panic!("expected two array elements, got {items:?}");
    };
    assert_array_items(first, &[Expr::Literal(Literal::Number(1.0))]);
    let Expr::ArrayLiteral {
        items: second_items,
    } = second
    else {
        panic!("expected nested array literal, got {second:?}");
    };
    let [two, three] = second_items.as_slice() else {
        panic!("expected two nested array elements, got {second_items:?}");
    };
    assert_eq!(two, &Expr::Literal(Literal::Number(2.0)));
    assert_array_items(three, &[Expr::Literal(Literal::Number(3.0))]);
}

#[test]
fn accepts_trailing_comma_before_closing_bracket() {
    let ast = parse_source(
        r#"
function run() -> Array<number> {
  return [
    1,
    2,
  ]
}
"#,
    )
    .expect("trailing comma should parse");

    let [Stmt::Return(Some(array))] = ast.functions[0].body.statements.as_slice() else {
        panic!("expected return array literal");
    };
    assert_array_items(
        array,
        &[
            Expr::Literal(Literal::Number(1.0)),
            Expr::Literal(Literal::Number(2.0)),
        ],
    );
}

#[test]
fn rejects_array_literal_missing_closing_bracket() {
    let error = parse_source(
        r#"
function run() -> void {
  return [1, 2
}
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("expected symbol ]"),
        "unexpected parse error: {error}"
    );
}

#[test]
fn rejects_array_literal_missing_expression() {
    let error = parse_source(
        r#"
function run() -> void {
  return [1,,2]
}
"#,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("expected expression"),
        "unexpected parse error: {error}"
    );
}

#[test]
fn array_literal_has_one_required_serde_shape() {
    let expression = Expr::ArrayLiteral {
        items: vec![Expr::Identifier("a".to_string())],
    };
    let wire = serde_json::to_value(&expression).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({
            "ArrayLiteral": {
                "items": [
                    { "Identifier": "a" }
                ]
            }
        })
    );
    assert_eq!(serde_json::from_value::<Expr>(wire).unwrap(), expression);
}

#[test]
fn array_literal_source_spans_cover_items_and_children() {
    let source = r#"function run() -> void {
  let values = [1, [2]]
}
"#;
    let ast = parse_source(source).unwrap();

    let expression = &ast.source_spans.functions[0].body.statements[0].expressions[0];
    assert_eq!(
        &source[expression.span.start.offset..expression.span.end.offset],
        "[1, [2]]"
    );
    let [first, second] = expression.children.as_slice() else {
        panic!(
            "expected two array element spans, got {:?}",
            expression.children
        );
    };
    assert_eq!(&source[first.span.start.offset..first.span.end.offset], "1");
    assert_eq!(
        &source[second.span.start.offset..second.span.end.offset],
        "[2]"
    );
    let [inner] = second.children.as_slice() else {
        panic!("expected one nested array element span");
    };
    assert_eq!(&source[inner.span.start.offset..inner.span.end.offset], "2");
}
