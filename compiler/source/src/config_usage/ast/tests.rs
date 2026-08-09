use crate::shared::{
    ast::{ExprSourceSpans, StmtSourceSpans, TestEffectStepOutcome},
    error::SourceSpan,
    parser::parse_source,
};

use super::*;

#[test]
fn compiler_test_effect_config_spans_skip_the_target_probe() {
    let parsed = parse_source(
        r#"
function values() -> void {
  let common = config.require<string>("effect.common")
  let step = config.require<string>("effect.step")
  let outcome = config.require<string>("effect.outcome")
}
"#,
    )
    .expect("fixture parses");
    let values = parsed.functions[0]
        .body
        .statements
        .iter()
        .map(|statement| match statement {
            Stmt::Let { value, .. } => value.clone(),
            other => panic!("expected let fixture, got {other:?}"),
        })
        .collect::<Vec<_>>();
    let value_spans = parsed.source_spans.functions[0]
        .body
        .statements
        .iter()
        .map(|statement| statement.expressions[0].clone())
        .collect::<Vec<_>>();
    let statement_span = parsed.source_spans.functions[0].body.statements[0].span;
    let target_probe_span = ExprSourceSpans {
        span: SourceSpan::synthetic(),
        children: Vec::new(),
        blocks: Vec::new(),
        record_fields: Vec::new(),
    };
    let block = Block {
        statements: vec![Stmt::CompilerTestEffectRegister {
            target: "dependency/run".to_string(),
            target_probe: Expr::Identifier("targetProbe".to_string()),
            declaration_start: true,
            expect: Some(values[0].clone()),
            step_expect: Some(values[1].clone()),
            outcome: TestEffectStepOutcome::Respond {
                value: values[2].clone(),
            },
        }],
    };
    let spans = BlockSourceSpans {
        span: parsed.source_spans.functions[0].body.span,
        statements: vec![StmtSourceSpans {
            span: statement_span,
            expressions: vec![
                target_probe_span,
                value_spans[0].clone(),
                value_spans[1].clone(),
                value_spans[2].clone(),
            ],
            blocks: Vec::new(),
        }],
    };
    let mut uses = Vec::new();
    let mut presence_uses = Vec::new();
    let mut violations = Vec::new();

    collect_config_uses_in_block(
        ConfigSourcePaths {
            diagnostic: "fixture.test.skiff",
            source: "fixture.test.skiff",
        },
        &block,
        Some(&spans),
        &BTreeMap::new(),
        &mut uses,
        &mut presence_uses,
        &mut violations,
    );

    assert!(violations.is_empty(), "{violations:?}");
    assert!(presence_uses.is_empty());
    assert_eq!(
        uses.iter()
            .map(|usage| usage.path.as_str())
            .collect::<Vec<_>>(),
        ["effect.common", "effect.step", "effect.outcome"]
    );
    assert_eq!(
        uses.iter()
            .map(|usage| usage.source_span)
            .collect::<Vec<_>>(),
        value_spans
            .iter()
            .map(|spans| Some(ConfigSourceSpan::from(spans.span)))
            .collect::<Vec<_>>()
    );
}
