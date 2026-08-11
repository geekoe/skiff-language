//! Phase 0 baseline: lock the `SourceSpanTable` side channel for
//! functions/impl_methods/consts/db_index_wheres and the local-variable
//! collection of `test_default_run_span`.

use crate::error::SourceSpan;
use crate::parser::{parse_source, parse_source_with_bodies_tolerant};

fn slice<'a>(source: &'a str, span: &SourceSpan) -> &'a str {
    &source[span.start.offset..span.end.offset]
}

#[test]
fn source_spans_functions_impl_methods_and_consts_are_collected() {
    let source = r#"function add(a: number, b: number) -> number {
  final sum = a + b
  return sum
}

type Service {}

impl Service {
  function run(self: Service, value: string) -> string {
    return value
  }
}

const offset: number = 1 + 2
"#;
    let ast = parse_source(source).unwrap();

    assert_eq!(ast.source_spans.functions.len(), 1);
    let function_spans = &ast.source_spans.functions[0];
    assert!(function_spans.effects.is_empty());
    assert_eq!(
        slice(source, &function_spans.body.span),
        "{\n  final sum = a + b\n  return sum\n}"
    );
    assert_eq!(function_spans.body.statements.len(), 2);
    let [let_stmt, return_stmt] = function_spans.body.statements.as_slice() else {
        panic!("expected two function statement spans");
    };
    assert_eq!(slice(source, &let_stmt.span), "final sum = a + b");
    assert_eq!(let_stmt.expressions.len(), 1);
    assert_eq!(slice(source, &let_stmt.expressions[0].span), "a + b");
    assert_eq!(let_stmt.expressions[0].children.len(), 2);
    assert_eq!(
        slice(source, &let_stmt.expressions[0].children[0].span),
        "a"
    );
    assert_eq!(
        slice(source, &let_stmt.expressions[0].children[1].span),
        "b"
    );
    assert_eq!(slice(source, &return_stmt.span), "return sum");
    assert_eq!(return_stmt.expressions.len(), 1);
    assert_eq!(slice(source, &return_stmt.expressions[0].span), "sum");

    assert_eq!(ast.source_spans.impl_methods.len(), 1);
    let method_spans = &ast.source_spans.impl_methods[0];
    assert!(method_spans.effects.is_empty());
    assert_eq!(
        slice(source, &method_spans.body.span),
        "{\n    return value\n  }"
    );
    assert_eq!(method_spans.body.statements.len(), 1);
    let [method_return] = method_spans.body.statements.as_slice() else {
        panic!("expected one impl method statement span");
    };
    assert_eq!(slice(source, &method_return.span), "return value");
    assert_eq!(method_return.expressions.len(), 1);
    assert_eq!(slice(source, &method_return.expressions[0].span), "value");

    assert_eq!(ast.source_spans.consts.len(), 1);
    assert_eq!(slice(source, &ast.source_spans.consts[0].span), "1 + 2");
    assert_eq!(ast.source_spans.consts[0].children.len(), 2);
    assert_eq!(
        slice(source, &ast.source_spans.consts[0].children[0].span),
        "1"
    );
    assert_eq!(
        slice(source, &ast.source_spans.consts[0].children[1].span),
        "2"
    );

    assert!(ast.source_spans.tests.is_empty());
    assert!(ast.source_spans.db_index_wheres.is_empty());
}

#[test]
fn source_spans_db_index_wheres_are_collected() {
    let source = r#"type Message {
  id: string,
  threadId: string
}

db object Message {
  primary key(id)
  index byOwner(ownerId, createdAt desc) where ownerId == owner
  unique index byExternalId(externalId) where externalId != null
}
"#;
    let ast = parse_source(source).unwrap();

    assert_eq!(ast.source_spans.db_index_wheres.len(), 2);
    let first = &ast.source_spans.db_index_wheres[0];
    assert_eq!(first.db_name, "Message");
    assert_eq!(first.index_name, "byOwner");
    assert_eq!(slice(source, &first.expression.span), "ownerId == owner");
    assert_eq!(first.expression.children.len(), 2);
    assert_eq!(slice(source, &first.expression.children[0].span), "ownerId");
    assert_eq!(slice(source, &first.expression.children[1].span), "owner");

    let second = &ast.source_spans.db_index_wheres[1];
    assert_eq!(second.db_name, "Message");
    assert_eq!(second.index_name, "byExternalId");
    assert_eq!(slice(source, &second.expression.span), "externalId != null");
    assert_eq!(second.expression.children.len(), 2);
}

#[test]
fn index_source_spans_visit_object_before_selector() {
    let source = r#"function read(
  items: Array<number>,
  selectors: Map<string, integer>,
  key: string
) -> number {
  return items[selectors[key]].value
}
"#;
    let ast = parse_source(source).unwrap();
    let expression = &ast.source_spans.functions[0].body.statements[0].expressions[0];

    assert_eq!(
        slice(source, &expression.span),
        "items[selectors[key]].value"
    );
    let [indexed_object] = expression.children.as_slice() else {
        panic!("expected field object span");
    };
    assert_eq!(slice(source, &indexed_object.span), "items[selectors[key]]");
    let [items, selector] = indexed_object.children.as_slice() else {
        panic!("expected index object and selector spans");
    };
    assert_eq!(slice(source, &items.span), "items");
    assert_eq!(slice(source, &selector.span), "selectors[key]");
    let [selectors, key] = selector.children.as_slice() else {
        panic!("expected nested index object and selector spans");
    };
    assert_eq!(slice(source, &selectors.span), "selectors");
    assert_eq!(slice(source, &key.span), "key");
}

#[test]
fn test_default_run_span_is_collected_locally_and_skipped_by_serde() {
    let source = "test defaultRun false";
    let ast = parse_source(source).unwrap();
    assert_eq!(ast.test_default_run, Some(false));
    let span = ast.test_default_run_span.expect("test_default_run_span");
    assert_eq!(slice(source, &span), "test defaultRun false");

    let with_semicolon = "test defaultRun true;";
    let ast = parse_source(with_semicolon).unwrap();
    assert_eq!(ast.test_default_run, Some(true));
    assert_eq!(
        slice(with_semicolon, &ast.test_default_run_span.expect("span")),
        "test defaultRun true;"
    );

    // Both test_default_run_span and source_spans are #[serde(skip)]
    // (syntax/src/ast.rs), so a wire round trip drops them while keeping the
    // parsed defaultRun value. Single-channel refactoring must not regress the
    // locally collected span.
    let wire = serde_json::to_value(&ast).unwrap();
    let decoded = serde_json::from_value::<crate::ast::SourceFile>(wire).unwrap();
    assert_eq!(decoded.test_default_run, Some(true));
    assert!(decoded.test_default_run_span.is_none());
    assert!(decoded.source_spans.is_empty());
}

#[test]
fn tolerant_mode_collects_spans_for_successful_impl_method_bodies_only() {
    let source = r#"impl Example {
  function ok(self: Example) -> void {
    return
  }
}
"#;
    let ast = parse_source_with_bodies_tolerant(source).unwrap();

    assert_eq!(ast.source_spans.impl_methods.len(), 1);
    let spans = &ast.source_spans.impl_methods[0];
    assert_eq!(spans.body.statements.len(), 1);
    assert_eq!(slice(source, &spans.body.statements[0].span), "return");
    assert!(spans.body.statements[0].expressions.is_empty());
}
