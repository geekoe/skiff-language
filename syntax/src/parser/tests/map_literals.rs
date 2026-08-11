use crate::{
    ast::{Expr, Literal, MapLiteralEntry, Stmt},
    parser::parse_source,
};

fn assert_map_entries(expr: &Expr, expected: &[(String, Expr)]) {
    let Expr::MapLiteral { entries } = expr else {
        panic!("expected map literal, got {expr:?}");
    };
    let actual = entries
        .iter()
        .map(|entry| (entry.key.clone(), entry.value.clone()))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn parses_non_empty_map_literal() {
    let ast = parse_source(
        r#"
function run() -> Map<string, number> {
  return { alpha: 1, beta: 2 }
}
"#,
    )
    .expect("map literal should parse");

    let [Stmt::Return(Some(map))] = ast.functions[0].body.statements.as_slice() else {
        panic!("expected return map literal");
    };
    assert_map_entries(
        map,
        &[
            ("alpha".to_string(), Expr::Literal(Literal::Number(1.0))),
            ("beta".to_string(), Expr::Literal(Literal::Number(2.0))),
        ],
    );
}

#[test]
fn parses_empty_map_literal() {
    let ast = parse_source(
        r#"
function run() -> Map<string, number> {
  return {}
}
"#,
    )
    .expect("empty map literal should parse");

    let [Stmt::Return(Some(map))] = ast.functions[0].body.statements.as_slice() else {
        panic!("expected return map literal");
    };
    assert_map_entries(map, &[]);
}

#[test]
fn parses_string_key_map_literal() {
    let ast = parse_source(
        r#"
function run() -> Map<string, number> {
  return { "answer": 42 }
}
"#,
    )
    .expect("string-key map literal should parse");

    let [Stmt::Return(Some(Expr::MapLiteral { entries }))] =
        ast.functions[0].body.statements.as_slice()
    else {
        panic!("expected return map literal");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key, "answer");
    assert!(matches!(
        &entries[0].value,
        Expr::Literal(Literal::Number(value)) if (*value - 42.0).abs() < f64::EPSILON
    ));
}

#[test]
fn accepts_trailing_comma_before_closing_brace() {
    let ast = parse_source(
        r#"
function run() -> Map<string, number> {
  return {
    alpha: 1,
    beta: 2,
  }
}
"#,
    )
    .expect("trailing comma should parse");

    let [Stmt::Return(Some(map))] = ast.functions[0].body.statements.as_slice() else {
        panic!("expected return map literal");
    };
    assert_map_entries(
        map,
        &[
            ("alpha".to_string(), Expr::Literal(Literal::Number(1.0))),
            ("beta".to_string(), Expr::Literal(Literal::Number(2.0))),
        ],
    );
}

#[test]
fn rejects_map_literal_missing_value() {
    let error = parse_source(
        r#"
function run() -> Map<string, number> {
  return { alpha: }
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
fn map_literal_has_one_required_serde_shape() {
    let expression = Expr::MapLiteral {
        entries: vec![MapLiteralEntry {
            key: "a".to_string(),
            key_span: None,
            value: Expr::Identifier("value".to_string()),
        }],
    };
    let wire = serde_json::to_value(&expression).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({
            "MapLiteral": {
                "entries": [
                    {
                        "key": "a",
                        "value": { "Identifier": "value" }
                    }
                ]
            }
        })
    );
    assert_eq!(serde_json::from_value::<Expr>(wire).unwrap(), expression);
}

#[test]
fn map_literal_source_spans_cover_entries_and_children() {
    let source = r#"function run() -> void {
  final values = { alpha: 1, beta: [2] }
}
"#;
    let ast = parse_source(source).unwrap();

    let expression = &ast.source_spans.functions[0].body.statements[0].expressions[0];
    assert_eq!(
        &source[expression.span.start.offset..expression.span.end.offset],
        "{ alpha: 1, beta: [2] }"
    );
    let [first, second] = expression.children.as_slice() else {
        panic!(
            "expected two map value spans, got {:?}",
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
    let [alpha, beta] = expression.record_fields.as_slice() else {
        panic!("expected two map key spans");
    };
    assert_eq!(alpha.name, "alpha");
    assert_eq!(
        &source[alpha.name_span.start.offset..alpha.name_span.end.offset],
        "alpha"
    );
    assert_eq!(beta.name, "beta");
    assert_eq!(
        &source[beta.name_span.start.offset..beta.name_span.end.offset],
        "beta"
    );
}
