use super::span::{ParsedBlock, ParsedStmt};
use super::*;

impl Parser {
    pub(super) fn parse_block(&mut self, in_test: bool) -> Result<ParsedBlock> {
        let start = self.peek().span.start;
        self.expect_symbol("{")?;
        let mut statements = Vec::new();
        let mut statement_spans = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            let mut statement = self.parse_statement(in_test)?;
            if self.match_symbol(";") {
                statement.spans.span.end = self.previous().span.end;
            }
            statements.push(statement.stmt);
            statement_spans.push(statement.spans);
        }
        self.expect_symbol("}")?;
        let end = self.previous().span.end;
        Ok(ParsedBlock {
            block: Block { statements },
            spans: BlockSourceSpans {
                span: SourceSpan { start, end },
                statements: statement_spans,
            },
        })
    }

    pub(super) fn parse_statement(&mut self, in_test: bool) -> Result<ParsedStmt> {
        if self.match_ident("const") {
            return self.parse_let(false, self.previous().span.start);
        }
        if self.match_ident("let") {
            return self.parse_let(true, self.previous().span.start);
        }
        if self.match_ident("timeout") {
            return self.parse_timeout_statement(in_test, self.previous().span.start);
        }
        if self.match_ident("concurrent") {
            return self.parse_concurrent_statement(in_test, self.previous().span.start);
        }
        if self.match_ident("serial") {
            return self.parse_serial_statement(in_test, self.previous().span.start);
        }
        if self.match_ident("if") {
            return self.parse_if(in_test, self.previous().span.start);
        }
        if self.match_ident("for") {
            return self.parse_for(in_test, self.previous().span.start);
        }
        if self.match_ident("while") {
            return self.parse_while(in_test, self.previous().span.start);
        }
        if self.match_ident("match") {
            return self.parse_match(in_test, self.previous().span.start);
        }
        if self.match_ident("assert") {
            let start = self.previous().span.start;
            if !in_test {
                return Err(CompileError::syntax(
                    "assert can only be used in test blocks",
                    self.previous().span.start,
                ));
            }
            return self.parse_assert_statement(start);
        }
        if self.match_ident("return") {
            let start = self.previous().span.start;
            if self.check_symbol("}") || self.check_symbol(";") {
                return Ok(ParsedStmt::leaf(
                    Stmt::Return(None),
                    SourceSpan {
                        start,
                        end: self.previous().span.end,
                    },
                ));
            }
            let (value_expr, value_spans) = self.parse_expression()?.into_parts();
            let end = value_spans.span.end;
            return Ok(ParsedStmt::with_expression(
                Stmt::Return(Some(value_expr)),
                SourceSpan { start, end },
                value_spans,
            ));
        }
        if self.match_ident("spawn") {
            let start = self.previous().span.start;
            let call = self.parse_expression()?;
            if !matches!(call.expr, Expr::Call { .. }) {
                return Err(CompileError::syntax(
                    "spawn statement expects a call expression",
                    call.spans.span.start,
                ));
            }
            let (call_expr, call_spans) = call.into_parts();
            let end = call_spans.span.end;
            return Ok(ParsedStmt::with_expression(
                Stmt::Spawn { call: call_expr },
                SourceSpan { start, end },
                call_spans,
            ));
        }
        if self.match_ident("throw") {
            let start = self.previous().span.start;
            let (value_expr, value_spans) = self.parse_expression()?.into_parts();
            let end = value_spans.span.end;
            return Ok(ParsedStmt::with_expression(
                Stmt::Throw { value: value_expr },
                SourceSpan { start, end },
                value_spans,
            ));
        }
        if self.match_ident("rethrow") {
            let start = self.previous().span.start;
            let (exception_expr, exception_spans) = self.parse_expression()?.into_parts();
            let end = exception_spans.span.end;
            return Ok(ParsedStmt::with_expression(
                Stmt::Rethrow {
                    exception: exception_expr,
                },
                SourceSpan { start, end },
                exception_spans,
            ));
        }
        if self.match_ident("emit") {
            let start = self.previous().span.start;
            let value = if self.match_symbol("(") {
                let value = self.parse_expression()?;
                self.expect_symbol(")")?;
                value
            } else {
                self.parse_expression()?
            };
            let (value_expr, value_spans) = value.into_parts();
            let end = self.previous().span.end;
            return Ok(ParsedStmt::with_expression(
                Stmt::Emit(value_expr),
                SourceSpan { start, end },
                value_spans,
            ));
        }
        if self.match_ident("break") {
            let span = self.previous().span;
            return Ok(ParsedStmt::leaf(Stmt::Break, span));
        }
        if self.match_ident("continue") {
            let span = self.previous().span;
            return Ok(ParsedStmt::leaf(Stmt::Continue, span));
        }
        let expr = self.parse_expression()?;
        if self.match_symbol("=") {
            let (target_expr, target_spans) = expr.into_parts();
            let (value_expr, value_spans) = self.parse_expression()?.into_parts();
            let span = SourceSpan {
                start: target_spans.span.start,
                end: value_spans.span.end,
            };
            return Ok(ParsedStmt::new(
                Stmt::Assign {
                    target: target_expr,
                    value: value_expr,
                },
                span,
                vec![target_spans, value_spans],
                Vec::new(),
            ));
        }
        Ok(ParsedStmt::expr(expr))
    }

    pub(super) fn parse_timeout_statement(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        let duration = self.parse_timeout_duration()?;
        if self.check_ident("value") || self.check_ident("concurrent") {
            return self
                .parse_timeout_value_after_duration(start, duration)
                .map(ParsedStmt::expr);
        }
        if !self.check_symbol("{") {
            return Err(CompileError::syntax(
                "expected timeout body",
                self.peek().span.start,
            ));
        }
        let (body_expr, body_spans) = self.parse_block(in_test)?.into_parts();
        let end = body_spans.span.end;
        Ok(ParsedStmt::with_block(
            Stmt::Timeout {
                duration,
                body: body_expr,
            },
            SourceSpan { start, end },
            body_spans,
        ))
    }

    pub(super) fn parse_concurrent_statement(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        if self.match_ident("value") {
            return self
                .parse_value_block_expression(start, true)
                .map(ParsedStmt::expr);
        }
        if !self.check_symbol("{") {
            let message = if self.check_ident("timeout")
                || self.check_ident("serial")
                || self.check_ident("concurrent")
            {
                "noncanonical modifier order; use `timeout(...) concurrent value { ... }`"
            } else {
                "expected concurrent body"
            };
            return Err(CompileError::syntax(message, self.peek().span.start));
        }
        let (body_expr, body_spans) = self.parse_block(in_test)?.into_parts();
        let end = body_spans.span.end;
        Ok(ParsedStmt::with_block(
            Stmt::Concurrent { body: body_expr },
            SourceSpan { start, end },
            body_spans,
        ))
    }

    pub(super) fn parse_serial_statement(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        if !self.check_symbol("{") {
            return Err(CompileError::syntax(
                "expected serial body",
                self.peek().span.start,
            ));
        }
        let (body_expr, body_spans) = self.parse_block(in_test)?.into_parts();
        let end = body_spans.span.end;
        Ok(ParsedStmt::with_block(
            Stmt::Serial { body: body_expr },
            SourceSpan { start, end },
            body_spans,
        ))
    }

    pub(super) fn parse_assert_statement(&mut self, start: SourceLocation) -> Result<ParsedStmt> {
        let (condition_expr, condition_spans) = self.parse_expression()?.into_parts();
        let message = if self.match_symbol(",") {
            Some(self.expect_string("expected assert message string")?)
        } else {
            None
        };
        let end = if message.is_some() {
            self.previous().span.end
        } else {
            condition_spans.span.end
        };
        Ok(ParsedStmt::with_expression(
            Stmt::Assert {
                condition: condition_expr,
                message,
            },
            SourceSpan { start, end },
            condition_spans,
        ))
    }

    pub(super) fn parse_let(&mut self, mutable: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let name = self.expect_ident("expected binding name")?;
        let ty = if self.match_symbol(":") {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect_symbol("=")?;
        let (value_expr, value_spans) = self.parse_expression()?.into_parts();
        let end = value_spans.span.end;
        Ok(ParsedStmt::with_expression(
            Stmt::Let {
                mutable,
                name,
                ty,
                value: value_expr,
            },
            SourceSpan { start, end },
            value_spans,
        ))
    }

    pub(super) fn parse_if(&mut self, in_test: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let (condition_expr, condition_spans) = self.parse_header_expression()?.into_parts();
        let then_block = self.parse_block(in_test)?;
        let else_block = if self.match_ident("else") {
            if self.match_ident("if") {
                let nested_if = self.parse_if(in_test, self.previous().span.start)?;
                Some(ParsedBlock::from_stmt(nested_if))
            } else {
                Some(self.parse_block(in_test)?)
            }
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map(|block| block.spans.span.end)
            .unwrap_or(then_block.spans.span.end);
        let (then_expr, then_spans) = then_block.into_parts();
        let (else_expr, blocks) = match else_block {
            Some(block) => {
                let (block_expr, block_spans) = block.into_parts();
                (Some(block_expr), vec![then_spans, block_spans])
            }
            None => (None, vec![then_spans]),
        };
        Ok(ParsedStmt::new(
            Stmt::If {
                condition: condition_expr,
                then_block: then_expr,
                else_block: else_expr,
            },
            SourceSpan { start, end },
            vec![condition_spans],
            blocks,
        ))
    }

    pub(super) fn parse_for(&mut self, in_test: bool, start: SourceLocation) -> Result<ParsedStmt> {
        let first = self.expect_ident("expected loop item name")?;
        let binding = if self.match_symbol(",") {
            let value = self.expect_ident("expected loop value name")?;
            ForBinding::Entry { key: first, value }
        } else {
            ForBinding::Item { item: first }
        };
        self.expect_ident_value("in")?;
        let (iterable_expr, iterable_spans) = self.parse_header_expression()?.into_parts();
        let (body_expr, body_spans) = self.parse_block(in_test)?.into_parts();
        let end = body_spans.span.end;
        Ok(ParsedStmt::with_expression_and_block(
            Stmt::For {
                binding,
                iterable: iterable_expr,
                body: body_expr,
            },
            SourceSpan { start, end },
            iterable_spans,
            body_spans,
        ))
    }

    pub(super) fn parse_while(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        let (condition_expr, condition_spans) = self.parse_header_expression()?.into_parts();
        let (body_expr, body_spans) = self.parse_block(in_test)?.into_parts();
        let end = body_spans.span.end;
        Ok(ParsedStmt::with_expression_and_block(
            Stmt::While {
                condition: condition_expr,
                body: body_expr,
            },
            SourceSpan { start, end },
            condition_spans,
            body_spans,
        ))
    }

    pub(super) fn parse_match(
        &mut self,
        in_test: bool,
        start: SourceLocation,
    ) -> Result<ParsedStmt> {
        let (value_expr, value_spans) = self.parse_header_expression()?.into_parts();
        let mut arms = Vec::new();
        let mut blocks = Vec::new();
        self.expect_symbol("{")?;
        while !self.check_symbol("}") && !self.is_at_end() {
            let pattern = self.parse_pattern()?;
            self.expect_symbol("=>")?;
            let body = self.parse_block(in_test)?;
            blocks.push(body.spans);
            arms.push(MatchArm {
                pattern,
                body: body.block,
            });
        }
        self.expect_symbol("}")?;
        let end = self.previous().span.end;
        Ok(ParsedStmt::new(
            Stmt::Match {
                value: value_expr,
                arms,
            },
            SourceSpan { start, end },
            vec![value_spans],
            blocks,
        ))
    }
}
