use super::span::ParsedExpr;
use super::*;

impl Parser {
    pub(super) fn parse_test_default_run_declaration(
        &mut self,
        start: SourceLocation,
    ) -> Result<(bool, SourceSpan)> {
        self.expect_ident_value("defaultRun")?;
        let value = if self.match_ident("true") {
            true
        } else if self.match_ident("false") {
            false
        } else {
            return Err(CompileError::syntax(
                "expected test defaultRun bool literal",
                self.peek().span.start,
            ));
        };
        let mut end = self.previous().span.end;
        if self.match_symbol(";") {
            end = self.previous().span.end;
        }
        Ok((value, SourceSpan { start, end }))
    }

    pub(super) fn parse_test_block(
        &mut self,
        name: String,
        start: SourceLocation,
    ) -> Result<crate::ast::TestDeclaration> {
        let (effects, effect_spans) = if self.match_ident("effects") {
            self.parse_test_effects()?
        } else {
            (Vec::new(), Vec::new())
        };
        let body = self.parse_block(true)?;
        let end = self.previous().span.end;
        self.source_spans.tests.push(ExecutableSourceSpans {
            effects: effect_spans,
            body: body.spans,
        });
        Ok(crate::ast::TestDeclaration {
            name,
            effects,
            body: body.block,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn parse_test_effects(
        &mut self,
    ) -> Result<(
        Vec<crate::ast::TestEffectDeclaration>,
        Vec<crate::ast::TestEffectSourceSpans>,
    )> {
        use std::collections::BTreeSet;

        self.expect_symbol("{")?;
        let mut effects = Vec::new();
        let mut effect_spans = Vec::new();
        let mut targets = BTreeSet::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            let start = self.peek().span.start;
            let target = self.parse_test_effect_target()?;
            if !targets.insert(target.clone()) {
                return Err(CompileError::syntax(
                    format!("duplicate test effect target `{target}`"),
                    start,
                ));
            }
            self.expect_symbol("{")?;
            let mut expect = None;
            let mut expect_spans = None;
            let mut outcome = None;
            let mut outcome_spans = None;
            while !self.check_symbol("}") && !self.is_at_end() {
                let field_location = self.peek().span.start;
                let field = self.expect_ident("expected test effect field")?;
                self.expect_symbol(":")?;
                match field.as_str() {
                    "expect" if expect.is_none() => {
                        let parsed = self.parse_expression()?;
                        expect = Some(parsed.expr);
                        expect_spans = Some(parsed.spans);
                    }
                    "expect" => {
                        return Err(CompileError::syntax(
                            "duplicate test effect `expect` field",
                            field_location,
                        ))
                    }
                    "respond" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome =
                            Some(crate::ast::TestEffectOutcome::Respond { value: parsed.expr });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Respond(
                            parsed.spans,
                        ));
                    }
                    "throw" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome = Some(crate::ast::TestEffectOutcome::Throw { value: parsed.expr });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Throw(
                            parsed.spans,
                        ));
                    }
                    "stream" if outcome.is_none() => {
                        let parsed = self.parse_test_effect_expression_sequence("stream")?;
                        outcome = Some(crate::ast::TestEffectOutcome::Stream {
                            events: parsed.iter().map(|value| value.expr.clone()).collect(),
                        });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Stream(
                            parsed.into_iter().map(|value| value.spans).collect(),
                        ));
                    }
                    "sequence" if outcome.is_none() => {
                        let (steps, step_spans) = self.parse_test_effect_sequence()?;
                        outcome = Some(crate::ast::TestEffectOutcome::Sequence { steps });
                        outcome_spans = Some(crate::ast::TestEffectOutcomeSourceSpans::Sequence {
                            steps: step_spans,
                        });
                    }
                    "respond" | "throw" | "stream" | "sequence" => {
                        return Err(CompileError::syntax(
                            "test effect must declare exactly one outcome field",
                            field_location,
                        ))
                    }
                    _ => {
                        return Err(CompileError::syntax(
                            format!("unknown test effect field `{field}`"),
                            field_location,
                        ))
                    }
                }
                self.match_symbol(",");
            }
            self.expect_symbol("}")?;
            let Some(outcome) = outcome else {
                return Err(CompileError::syntax(
                    "test effect requires an outcome field",
                    start,
                ));
            };
            let end = self.previous().span.end;
            effects.push(crate::ast::TestEffectDeclaration {
                target,
                expect,
                outcome,
                span: SourceSpan { start, end },
            });
            effect_spans.push(crate::ast::TestEffectSourceSpans {
                expect: expect_spans,
                outcome: outcome_spans.expect("parsed test effect outcome spans"),
            });
            self.match_symbol(",");
        }
        self.expect_symbol("}")?;
        Ok((effects, effect_spans))
    }

    pub(super) fn parse_test_effect_target(&mut self) -> Result<String> {
        let mut target = self.expect_ident("expected test effect target")?;
        if self.match_symbol("/") {
            target.push('/');
            target.push_str(&self.expect_ident("expected source module after /")?);
        }
        while self.match_symbol(".") {
            target.push('.');
            target.push_str(&self.expect_ident("expected test effect target segment")?);
        }
        Ok(target)
    }

    pub(super) fn parse_test_effect_sequence(
        &mut self,
    ) -> Result<(
        Vec<crate::ast::TestEffectSequenceStep>,
        Vec<crate::ast::TestEffectSequenceStepSourceSpans>,
    )> {
        self.expect_symbol("[")?;
        let mut steps = Vec::new();
        let mut step_spans = Vec::new();
        while !self.check_symbol("]") && !self.is_at_end() {
            let start = self.peek().span.start;
            self.expect_symbol("{")?;
            let mut expect = None;
            let mut expect_spans = None;
            let mut outcome = None;
            let mut outcome_spans = None;
            while !self.check_symbol("}") && !self.is_at_end() {
                let field_location = self.peek().span.start;
                let field = self.expect_ident("expected test effect sequence step field")?;
                self.expect_symbol(":")?;
                match field.as_str() {
                    "expect" if expect.is_none() => {
                        let parsed = self.parse_expression()?;
                        expect = Some(parsed.expr);
                        expect_spans = Some(parsed.spans);
                    }
                    "expect" => {
                        return Err(CompileError::syntax(
                            "duplicate test effect sequence step `expect` field",
                            field_location,
                        ))
                    }
                    "respond" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome =
                            Some(crate::ast::TestEffectStepOutcome::Respond { value: parsed.expr });
                        outcome_spans = Some(
                            crate::ast::TestEffectStepOutcomeSourceSpans::Respond(parsed.spans),
                        );
                    }
                    "throw" if outcome.is_none() => {
                        let parsed = self.parse_expression()?;
                        outcome =
                            Some(crate::ast::TestEffectStepOutcome::Throw { value: parsed.expr });
                        outcome_spans = Some(crate::ast::TestEffectStepOutcomeSourceSpans::Throw(
                            parsed.spans,
                        ));
                    }
                    "stream" if outcome.is_none() => {
                        let parsed = self.parse_test_effect_expression_sequence("stream")?;
                        outcome = Some(crate::ast::TestEffectStepOutcome::Stream {
                            events: parsed.iter().map(|value| value.expr.clone()).collect(),
                        });
                        outcome_spans = Some(crate::ast::TestEffectStepOutcomeSourceSpans::Stream(
                            parsed.into_iter().map(|value| value.spans).collect(),
                        ));
                    }
                    "respond" | "throw" | "stream" => {
                        return Err(CompileError::syntax(
                            "test effect sequence step must declare exactly one outcome field",
                            field_location,
                        ))
                    }
                    _ => {
                        return Err(CompileError::syntax(
                            format!("unknown test effect sequence step field `{field}`"),
                            field_location,
                        ))
                    }
                }
                self.match_symbol(",");
            }
            self.expect_symbol("}")?;
            let Some(outcome) = outcome else {
                return Err(CompileError::syntax(
                    "test effect sequence step requires an outcome field",
                    start,
                ));
            };
            steps.push(crate::ast::TestEffectSequenceStep { expect, outcome });
            step_spans.push(crate::ast::TestEffectSequenceStepSourceSpans {
                expect: expect_spans,
                outcome: outcome_spans.expect("parsed test effect sequence step outcome spans"),
            });
            if !self.match_symbol(",") {
                break;
            }
        }
        self.expect_symbol("]")?;
        if steps.is_empty() {
            return Err(CompileError::syntax(
                "test effect `sequence` cannot be empty",
                self.previous().span.start,
            ));
        }
        Ok((steps, step_spans))
    }

    pub(super) fn parse_test_effect_expression_sequence(
        &mut self,
        field: &str,
    ) -> Result<Vec<ParsedExpr>> {
        self.expect_symbol("[")?;
        let mut values = Vec::new();
        while !self.check_symbol("]") && !self.is_at_end() {
            values.push(self.parse_expression()?);
            if !self.match_symbol(",") {
                break;
            }
        }
        self.expect_symbol("]")?;
        if values.is_empty() {
            return Err(CompileError::syntax(
                format!("test effect `{field}` cannot be empty"),
                self.previous().span.start,
            ));
        }
        Ok(values)
    }
}
