use std::path::{Path, PathBuf};

use skiff_artifact_model::{InstructionSourceSite, StatementAttributionClass};

use crate::{
    parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile, ExpressionKey,
    ExpressionOwnerKey,
};

use super::{SourceEventFacts, SourceEventKey, SourceStatementKey};

fn parsed_sources(sources: &[(&str, &str)]) -> Vec<crate::parsed_sources::ParsedCompilerSource> {
    let files = sources
        .iter()
        .map(|(module_path, text)| {
            let relative_path = format!("{}.skiff", module_path.replace('.', "/"));
            CompilerSourceFile::parse(
                PathBuf::from(&relative_path),
                (*module_path).to_string(),
                false,
                false,
                (*text).to_string(),
                &relative_path,
            )
            .expect("source-event fixture should parse")
        })
        .collect::<Vec<_>>();
    parse_publication_sources(Path::new("/source-events"), &files)
        .expect("source-event fixture should build parsed sources")
}

fn facts(module_path: &str, source: &str) -> SourceEventFacts {
    SourceEventFacts::build(&parsed_sources(&[(module_path, source)]))
        .expect("source-event facts should build")
}

fn function_owner(name: &str) -> ExpressionOwnerKey {
    ExpressionOwnerKey::Function(name.to_string())
}

fn statement_key(module_path: &str, owner: &ExpressionOwnerKey, index: u32) -> SourceEventKey {
    SourceEventKey::Statement(SourceStatementKey::new(module_path, owner.clone(), index))
}

fn expression_key(module_path: &str, owner: &ExpressionOwnerKey, index: u32) -> SourceEventKey {
    SourceEventKey::Expression(ExpressionKey::new(module_path, owner.clone(), index))
}

fn source_text<'a>(source: &'a str, site: &InstructionSourceSite) -> &'a str {
    let InstructionSourceSite::Source { span } = site else {
        panic!("source collection must not produce synthetic sites")
    };
    &source[span.start.offset.expect("start offset") as usize
        ..span.end.offset.expect("end offset") as usize]
}

#[test]
fn statement_and_expression_facts_use_their_exact_syntax_spans() {
    let source = r#"function run() -> void {
  let value = 123
}"#;
    let events = facts("pkg.main", source);
    let owner = function_owner("run");
    let legacy_expression_key = ExpressionKey::new("pkg.main", owner.clone(), 0);
    let statement = events
        .fact(&statement_key("pkg.main", &owner, 0))
        .expect("let statement fact");
    let expression = events
        .fact(&SourceEventKey::Expression(legacy_expression_key.clone()))
        .expect("literal expression fact");

    assert_eq!(source_text(source, statement.site()), "let value = 123");
    assert_eq!(source_text(source, expression.site()), "123");
    assert_ne!(statement.site(), expression.site());
    assert_eq!(
        statement.key().attribution_class(),
        StatementAttributionClass::Statement
    );
    assert_eq!(
        expression.key().attribution_class(),
        StatementAttributionClass::Expression
    );
    assert!(events
        .expression_sources()
        .fact(&legacy_expression_key)
        .is_some());
}

#[test]
fn equal_spans_keep_distinct_typed_source_keys() {
    let source = r#"function run() -> void {
  ping()
}"#;
    let events = facts("pkg.main", source);
    let owner = function_owner("run");
    let statement_key = statement_key("pkg.main", &owner, 0);
    let expression_key = expression_key("pkg.main", &owner, 0);
    let statement = events.fact(&statement_key).expect("expression statement");
    let expression = events.fact(&expression_key).expect("call expression");

    assert_eq!(statement.site(), expression.site());
    assert_ne!(statement_key, expression_key);
    assert_eq!(source_text(source, statement.site()), "ping()");
}

#[test]
fn zero_expression_statements_and_nested_blocks_have_statement_facts() {
    let source = r#"function run(flag: bool) -> void {
  while flag {
    if flag {
      continue
    }
    break
  }
  return
}"#;
    let events = facts("pkg.flow", source);
    let owner = function_owner("run");
    let expected = [
        "while flag {\n    if flag {\n      continue\n    }\n    break\n  }",
        "if flag {\n      continue\n    }",
        "continue",
        "break",
        "return",
    ];

    for (index, expected_text) in expected.into_iter().enumerate() {
        let fact = events
            .fact(&statement_key("pkg.flow", &owner, index as u32))
            .unwrap_or_else(|| panic!("missing statement preorder index {index}"));
        assert_eq!(source_text(source, fact.site()), expected_text);
    }
    assert!(events
        .fact(&statement_key("pkg.flow", &owner, expected.len() as u32))
        .is_none());
    assert!(events
        .fact(&expression_key("pkg.flow", &owner, 2))
        .is_none());
}

#[test]
fn module_owner_and_independent_preorders_are_deterministic() {
    let source = r#"function run() -> void {
  let value = 1
  return
}"#;
    let parsed = parsed_sources(&[("pkg.alpha", source), ("pkg.beta", source)]);
    let first = SourceEventFacts::build(&parsed).expect("first source walk");
    let second = SourceEventFacts::build(&parsed).expect("second source walk");
    assert_eq!(
        first.iter().cloned().collect::<Vec<_>>(),
        second.iter().cloned().collect::<Vec<_>>()
    );

    let owner = function_owner("run");
    for module_path in ["pkg.alpha", "pkg.beta"] {
        assert!(first.fact(&statement_key(module_path, &owner, 0)).is_some());
        assert!(first.fact(&statement_key(module_path, &owner, 1)).is_some());
        assert!(first
            .fact(&expression_key(module_path, &owner, 0))
            .is_some());
        assert!(first
            .fact(&expression_key(module_path, &owner, 1))
            .is_none());
    }
}

#[test]
fn source_authority_never_produces_generated_or_synthetic_events() {
    let source = r#"function run() -> void {
  return
}"#;
    let events = facts("pkg.boundary", source);

    assert!(!events.is_empty());
    for fact in events.iter() {
        assert!(matches!(
            fact.key().attribution_class(),
            StatementAttributionClass::Statement | StatementAttributionClass::Expression
        ));
        assert!(matches!(fact.site(), InstructionSourceSite::Source { .. }));
    }
}
