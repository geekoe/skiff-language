use std::collections::BTreeMap;

use crate::{
    shared::ast::{Block, BlockSourceSpans, ExprSourceSpans, MatchArm, Stmt, StmtSourceSpans},
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceFact, ExpressionSourceMap,
};

use super::{
    spans::source_instruction_site, SourceEventFact, SourceEventFacts, SourceEventKey,
    SourceOwnerInventory, SourceStatementKey,
};

pub(super) struct OwnerCollector<'a> {
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    next_statement_index: u32,
    next_expression_index: u32,
    events: &'a mut BTreeMap<SourceEventKey, SourceEventFact>,
    expressions: &'a mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
}

pub(super) fn collect_source_events(
    parsed_sources: &[crate::parsed_sources::ParsedCompilerSource],
) -> Result<SourceEventFacts, String> {
    let mut owners = SourceOwnerInventory::new();
    let mut events = BTreeMap::new();
    let mut expressions = BTreeMap::new();
    for parsed in parsed_sources {
        if parsed.source().is_test_file {
            continue;
        }
        super::owners::collect_source(
            parsed.source().module_path.as_str(),
            parsed.ast(),
            &mut owners,
            &mut events,
            &mut expressions,
        )?;
    }
    Ok(SourceEventFacts {
        owners,
        facts: events,
        expression_sources: ExpressionSourceMap::from_facts(expressions),
    })
}

impl OwnerCollector<'_> {
    pub(super) fn visit_block(
        &mut self,
        block: &Block,
        spans: &BlockSourceSpans,
    ) -> Result<(), String> {
        if block.statements.len() != spans.statements.len() {
            return Err(self.error(format!(
                "block statement span count {} does not match AST statement count {}",
                spans.statements.len(),
                block.statements.len()
            )));
        }
        for (statement, statement_spans) in block.statements.iter().zip(&spans.statements) {
            self.visit_statement(statement, statement_spans)?;
        }
        Ok(())
    }

    fn visit_statement(&mut self, stmt: &Stmt, spans: &StmtSourceSpans) -> Result<(), String> {
        self.record_statement(spans)?;
        let mut expressions = spans.expressions.iter();
        let mut blocks = spans.blocks.iter();
        match stmt {
            Stmt::CompilerTestEffectRegister {
                target_probe,
                expect,
                step_expect,
                outcome,
                ..
            } => {
                self.visit_expr(
                    target_probe,
                    next_statement_expression(&mut expressions, "test effect target")?,
                )?;
                if let Some(expect) = expect {
                    self.visit_expr(
                        expect,
                        next_statement_expression(&mut expressions, "test effect expectation")?,
                    )?;
                }
                if let Some(step_expect) = step_expect {
                    self.visit_expr(
                        step_expect,
                        next_statement_expression(
                            &mut expressions,
                            "test effect sequence step expectation",
                        )?,
                    )?;
                }
                match outcome {
                    crate::shared::ast::TestEffectStepOutcome::Respond { value }
                    | crate::shared::ast::TestEffectStepOutcome::Throw { value } => self
                        .visit_expr(
                            value,
                            next_statement_expression(&mut expressions, "test effect outcome")?,
                        )?,
                    crate::shared::ast::TestEffectStepOutcome::Stream { events } => {
                        for value in events {
                            self.visit_expr(
                                value,
                                next_statement_expression(
                                    &mut expressions,
                                    "test effect stream event",
                                )?,
                            )?;
                        }
                    }
                }
            }
            Stmt::Assert { condition, .. } => self.visit_expr(
                condition,
                next_statement_expression(&mut expressions, "assert condition")?,
            )?,
            Stmt::Let { value, .. } => self.visit_expr(
                value,
                next_statement_expression(&mut expressions, "let value")?,
            )?,
            Stmt::Assign { target, value } => {
                self.visit_expr(
                    target,
                    next_statement_expression(&mut expressions, "assign target")?,
                )?;
                self.visit_expr(
                    value,
                    next_statement_expression(&mut expressions, "assign value")?,
                )?;
            }
            Stmt::Timeout { body, .. } => {
                self.visit_block(body, next_statement_block(&mut blocks, "timeout body")?)?
            }
            Stmt::Concurrent { body } => {
                self.visit_block(body, next_statement_block(&mut blocks, "concurrent body")?)?
            }
            Stmt::Serial { body } => {
                self.visit_block(body, next_statement_block(&mut blocks, "serial body")?)?
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.visit_expr(
                    condition,
                    next_statement_expression(&mut expressions, "if condition")?,
                )?;
                self.visit_block(
                    then_block,
                    next_statement_block(&mut blocks, "if then block")?,
                )?;
                if let Some(else_block) = else_block {
                    self.visit_block(
                        else_block,
                        next_statement_block(&mut blocks, "if else block")?,
                    )?;
                }
            }
            Stmt::For { iterable, body, .. } => {
                self.visit_expr(
                    iterable,
                    next_statement_expression(&mut expressions, "for iterable")?,
                )?;
                self.visit_block(body, next_statement_block(&mut blocks, "for body")?)?;
            }
            Stmt::While { condition, body } => {
                self.visit_expr(
                    condition,
                    next_statement_expression(&mut expressions, "while condition")?,
                )?;
                self.visit_block(body, next_statement_block(&mut blocks, "while body")?)?;
            }
            Stmt::Match { value, arms } => {
                self.visit_expr(
                    value,
                    next_statement_expression(&mut expressions, "match value")?,
                )?;
                for (index, arm) in arms.iter().enumerate() {
                    self.visit_match_arm(
                        arm,
                        next_statement_block(&mut blocks, &format!("match arm {index}"))?,
                    )?;
                }
            }
            Stmt::DbTransaction { body } => self.visit_block(
                body,
                next_statement_block(&mut blocks, "db transaction stmt body")?,
            )?,
            Stmt::Throw { value } => self.visit_expr(
                value,
                next_statement_expression(&mut expressions, "throw stmt value")?,
            )?,
            Stmt::Rethrow { exception } => self.visit_expr(
                exception,
                next_statement_expression(&mut expressions, "rethrow stmt exception")?,
            )?,
            Stmt::Emit(value) => self.visit_expr(
                value,
                next_statement_expression(&mut expressions, "emit value")?,
            )?,
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.visit_expr(
                        value,
                        next_statement_expression(&mut expressions, "return value")?,
                    )?;
                }
            }
            Stmt::Expr(value) => self.visit_expr(
                value,
                next_statement_expression(&mut expressions, "stmt expr")?,
            )?,
            Stmt::Break | Stmt::Continue => {}
        }
        ensure_exhausted(expressions, || {
            self.error("unused statement expression span")
        })?;
        ensure_exhausted(blocks, || self.error("unused statement block span"))?;
        Ok(())
    }

    fn visit_match_arm(&mut self, arm: &MatchArm, spans: &BlockSourceSpans) -> Result<(), String> {
        self.visit_block(&arm.body, spans)
    }

    fn record_statement(&mut self, spans: &StmtSourceSpans) -> Result<(), String> {
        let key = SourceStatementKey::new(
            self.module_path,
            self.owner.clone(),
            self.next_statement_index,
        );
        self.next_statement_index = self
            .next_statement_index
            .checked_add(1)
            .ok_or_else(|| self.error("too many statements in owner"))?;
        self.record_event(SourceEventKey::Statement(key), spans.span)
    }

    pub(super) fn record_expression(&mut self, spans: &ExprSourceSpans) -> Result<u32, String> {
        let preorder_index = self.next_expression_index;
        let key = ExpressionKey::new(self.module_path, self.owner.clone(), preorder_index);
        self.next_expression_index = self
            .next_expression_index
            .checked_add(1)
            .ok_or_else(|| self.error("too many expressions in owner"))?;
        self.record_event(SourceEventKey::Expression(key.clone()), spans.span)?;
        if self
            .expressions
            .insert(
                key.clone(),
                ExpressionSourceFact {
                    span: spans.span,
                    record_fields: spans.record_fields.clone(),
                },
            )
            .is_some()
        {
            return Err(self.error(format!("duplicate expression source key {key:?}")));
        }
        Ok(preorder_index)
    }

    fn record_event(
        &mut self,
        key: SourceEventKey,
        span: crate::shared::error::SourceSpan,
    ) -> Result<(), String> {
        let fact = SourceEventFact {
            key: key.clone(),
            site: source_instruction_site(span).map_err(|message| self.error(message))?,
        };
        if self.events.insert(key.clone(), fact).is_some() {
            return Err(self.error(format!("duplicate source event key {key:?}")));
        }
        Ok(())
    }

    pub(super) fn error(&self, message: impl Into<String>) -> String {
        format!(
            "source event model mismatch in module {} owner {:?}: {}",
            self.module_path,
            self.owner,
            message.into()
        )
    }
}

impl<'a> OwnerCollector<'a> {
    pub(super) fn new(
        module_path: &'a str,
        owner: ExpressionOwnerKey,
        events: &'a mut BTreeMap<SourceEventKey, SourceEventFact>,
        expressions: &'a mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
    ) -> Self {
        Self {
            module_path,
            owner,
            next_statement_index: 0,
            next_expression_index: 0,
            events,
            expressions,
        }
    }
}

fn next_statement_expression<'a>(
    expressions: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    label: &str,
) -> Result<&'a ExprSourceSpans, String> {
    expressions
        .next()
        .ok_or_else(|| format!("missing statement expression span for {label}"))
}

fn next_statement_block<'a>(
    blocks: &mut impl Iterator<Item = &'a BlockSourceSpans>,
    label: &str,
) -> Result<&'a BlockSourceSpans, String> {
    blocks
        .next()
        .ok_or_else(|| format!("missing statement block span for {label}"))
}

pub(super) fn ensure_exhausted<T>(
    mut values: impl Iterator<Item = T>,
    error: impl FnOnce() -> String,
) -> Result<(), String> {
    if values.next().is_some() {
        Err(error())
    } else {
        Ok(())
    }
}
