use super::span::{expr_source_spans, expr_source_spans_from_span, parsed_leaf_expr, ParsedExpr};
use super::*;

fn object_literal_key_name(key: &crate::ast::ObjectLiteralKey) -> Option<String> {
    match key {
        crate::ast::ObjectLiteralKey::Name(name) => Some(name.clone()),
    }
}

impl Parser {
    pub(super) fn parse_expression(&mut self) -> Result<ParsedExpr> {
        let condition = self.parse_binary(0)?;
        if !self.match_symbol("?") {
            return Ok(condition);
        }
        let then_expr = self.parse_expression()?;
        if !self.match_symbol(":") {
            return Err(CompileError::syntax(
                "expected `:` separating ternary branches",
                self.peek().span.start,
            ));
        }
        let else_expr = self.parse_expression()?;
        let span = SourceSpan {
            start: condition.spans.span.start,
            end: else_expr.spans.span.end,
        };
        Ok(ParsedExpr::new(
            Expr::Ternary {
                condition: Box::new(condition.expr),
                then_expr: Box::new(then_expr.expr),
                else_expr: Box::new(else_expr.expr),
            },
            span,
            vec![condition.spans, then_expr.spans, else_expr.spans],
        ))
    }

    /// Parses the expression of a statement header (`if`, `while`,
    /// `for ... in`, `match`). A `{` directly following the header expression
    /// is always the body or arms, so record/patch constructs in the header
    /// must be wrapped in parentheses.
    pub(super) fn parse_header_expression(&mut self) -> Result<ParsedExpr> {
        let saved_header = self.in_statement_header;
        self.in_statement_header = true;
        let result = self.parse_expression();
        self.in_statement_header = saved_header;
        result
    }

    /// Parses an expression in a nested expression slot where a `{` after a
    /// path is a nominal construct. Callers restore the previous slot mode on
    /// both success and error.
    pub(super) fn parse_slot_expression(&mut self) -> Result<ParsedExpr> {
        let saved_header = self.in_statement_header;
        self.in_statement_header = false;
        let result = self.parse_expression();
        self.in_statement_header = saved_header;
        result
    }

    pub(super) fn parse_binary(&mut self, min_prec: u8) -> Result<ParsedExpr> {
        let mut left = self.parse_unary()?;
        while let Some((op, prec)) = self.peek_binary_op() {
            if prec < min_prec {
                break;
            }
            self.advance();
            let right = self.parse_binary(prec + 1)?;
            let span = SourceSpan {
                start: left.spans.span.start,
                end: right.spans.span.end,
            };
            left = ParsedExpr::new(
                Expr::Binary {
                    op,
                    left: Box::new(left.expr),
                    right: Box::new(right.expr),
                },
                span,
                vec![left.spans, right.spans],
            );
        }
        Ok(left)
    }

    pub(super) fn parse_unary(&mut self) -> Result<ParsedExpr> {
        if self.match_symbol("!") {
            let start = self.previous().span.start;
            let expr = self.parse_unary()?;
            return Ok(ParsedExpr::new(
                Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr.expr),
                },
                SourceSpan {
                    start,
                    end: expr.spans.span.end,
                },
                vec![expr.spans],
            ));
        }
        self.parse_postfix()
    }

    pub(super) fn parse_postfix(&mut self) -> Result<ParsedExpr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.check_dependency_source_address_suffix(&expr) {
                expr = self.parse_dependency_source_address(expr)?;
                continue;
            }
            if self.match_symbol(".") {
                let field = self.expect_ident("expected field name after .")?;
                let span = SourceSpan {
                    start: expr.spans.span.start,
                    end: self.previous().span.end,
                };
                expr = ParsedExpr::new(
                    Expr::Field {
                        object: Box::new(expr.expr),
                        field,
                    },
                    span,
                    vec![expr.spans],
                );
                continue;
            }
            if self.check_symbol("<") && self.looks_like_generic_call_suffix() {
                let type_args = self.parse_generic_args()?;
                let span = SourceSpan {
                    start: expr.spans.span.start,
                    end: self.previous().span.end,
                };
                expr = ParsedExpr::new(
                    Expr::Generic {
                        callee: Box::new(expr.expr),
                        type_args,
                    },
                    span,
                    vec![expr.spans],
                );
                continue;
            }
            if self.match_symbol("(") {
                let mut args = Vec::new();
                let mut children = vec![expr.spans];
                if !self.check_symbol(")") {
                    loop {
                        if self.match_ident("inout") {
                            let (place_expr, place_spans) =
                                self.parse_slot_expression()?.into_parts();
                            children.push(place_spans);
                            args.push(CallArg::InOutPlace { expr: place_expr });
                        } else {
                            let arg = self.parse_slot_expression()?;
                            children.push(arg.spans);
                            args.push(CallArg::Value(arg.expr));
                        }
                        if !self.match_symbol(",") {
                            break;
                        }
                    }
                }
                self.expect_symbol(")")?;
                let span = SourceSpan {
                    start: children[0].span.start,
                    end: self.previous().span.end,
                };
                expr = ParsedExpr::new(
                    Expr::Call {
                        callee: Box::new(expr.expr),
                        args,
                    },
                    span,
                    children,
                );
                continue;
            }
            if self.match_ident("as") {
                let as_start = self.previous().span.start;
                if self.check_ident("any") {
                    return Err(CompileError::syntax(
                        "`as` expects an interface selector; use `as I`, not `as any I`",
                        self.peek().span.start,
                    ));
                }
                let interface = self.parse_type()?;
                let span = SourceSpan {
                    start: expr.spans.span.start,
                    end: self.previous().span.end,
                };
                if interface.name.trim().is_empty() {
                    return Err(CompileError::syntax(
                        "expected interface selector after `as`",
                        as_start,
                    ));
                }
                expr = ParsedExpr::new(
                    Expr::InterfaceBox {
                        value: Box::new(expr.expr),
                        interface,
                    },
                    span,
                    vec![expr.spans],
                );
                continue;
            }
            if self.check_symbol("{") && !self.in_statement_header {
                if let Some(target) = Self::patch_construct_target(&expr.expr) {
                    self.advance();
                    let (operations, operation_spans) = self.parse_patch_operations()?;
                    let mut children = vec![expr.spans];
                    children.extend(operation_spans);
                    let span = SourceSpan {
                        start: children[0].span.start,
                        end: self.previous().span.end,
                    };
                    expr = ParsedExpr::new(Expr::Patch { target, operations }, span, children);
                    continue;
                }
                if let Some((type_name, type_args)) = Self::nominal_construct_parts(&expr.expr) {
                    self.advance();
                    let (fields, field_spans, record_fields) =
                        self.parse_record_construct_fields()?;
                    let mut children = vec![expr.spans];
                    children.extend(field_spans);
                    let span = SourceSpan {
                        start: children[0].span.start,
                        end: self.previous().span.end,
                    };
                    expr = ParsedExpr::with_children_and_parts(
                        Expr::Record {
                            type_name,
                            type_args,
                            fields,
                        },
                        span,
                        children,
                        Vec::new(),
                        record_fields,
                    );
                    continue;
                }
            }
            break;
        }
        Ok(expr)
    }

    pub(super) fn patch_construct_target(expr: &Expr) -> Option<TypeRef> {
        let Expr::Generic { callee, type_args } = expr else {
            return None;
        };
        let Expr::Identifier(name) = callee.as_ref() else {
            return None;
        };
        if name != "patch" || type_args.len() != 1 {
            return None;
        }
        type_args.first().cloned()
    }

    pub(super) fn looks_like_generic_call_suffix(&mut self) -> bool {
        let snapshot = self.snapshot();
        let result =
            self.parse_generic_args().is_ok() && (self.check_symbol("(") || self.check_symbol("{"));
        self.restore(snapshot);
        result
    }

    pub(super) fn nominal_construct_parts(expr: &Expr) -> Option<(String, Vec<TypeRef>)> {
        let (callee, type_args) = match expr {
            Expr::Generic { callee, type_args } => (callee.as_ref(), type_args.clone()),
            _ => (expr, Vec::new()),
        };
        let type_name = expr_path(without_generic(callee))?;
        Some((type_name, type_args))
    }

    pub(super) fn parse_generic_args(&mut self) -> Result<Vec<TypeRef>> {
        self.expect_symbol("<")?;
        let mut type_args = Vec::new();
        if !self.check_symbol(">") {
            loop {
                type_args.push(self.parse_type()?);
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(">")?;
        Ok(type_args)
    }

    pub(super) fn parse_primary(&mut self) -> Result<ParsedExpr> {
        let token = self.advance().clone();
        let start = token.span.start;
        match token.kind {
            TokenKind::Number(value) => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::Number(value)),
                token.span,
            )),
            TokenKind::Duration(_) => Err(CompileError::syntax(
                "duration literal is only allowed as a timeout duration",
                token.span.start,
            )),
            TokenKind::String(value) => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::String(value)),
                token.span,
            )),
            TokenKind::Ident(value) if value == "true" => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::Bool(true)),
                token.span,
            )),
            TokenKind::Ident(value) if value == "false" => Ok(parsed_leaf_expr(
                Expr::Literal(Literal::Bool(false)),
                token.span,
            )),
            TokenKind::Ident(value) if value == "null" => {
                Ok(parsed_leaf_expr(Expr::Literal(Literal::Null), token.span))
            }
            TokenKind::Ident(value) if value == "value" && self.check_symbol("{") => {
                self.parse_value_block_expression(start, false)
            }
            TokenKind::Ident(value)
                if value == "value"
                    && (self.check_ident("timeout")
                        || self.check_ident("concurrent")
                        || self.check_ident("serial")
                        || self.check_ident("value")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; use `value`, `concurrent value`, `timeout(...) value`, or `timeout(...) concurrent value`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "concurrent" && self.check_ident("value") => {
                self.advance();
                self.parse_value_block_expression(start, true)
            }
            TokenKind::Ident(value)
                if value == "concurrent"
                    && (self.check_ident("timeout")
                        || self.check_ident("serial")
                        || self.check_ident("concurrent")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; expression form is `concurrent value { ... }`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "timeout" && self.check_symbol("(") => {
                self.parse_timeout_expression(start)
            }
            TokenKind::Ident(value)
                if value == "timeout"
                    && (self.check_ident("value")
                        || self.check_ident("concurrent")
                        || self.check_ident("serial")
                        || self.check_ident("timeout")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; timeout must be followed by `(duration)`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "serial" && self.check_symbol("{") => {
                Err(CompileError::syntax(
                    "serial is a statement and cannot be used as an expression",
                    token.span.start,
                ))
            }
            TokenKind::Ident(value)
                if value == "serial"
                    && (self.check_ident("value")
                        || self.check_ident("concurrent")
                        || self.check_ident("timeout")
                        || self.check_ident("serial")) =>
            {
                Err(CompileError::syntax(
                    "noncanonical modifier order; serial is only `serial { ... }`",
                    self.peek().span.start,
                ))
            }
            TokenKind::Ident(value) if value == "concurrent" && self.check_symbol("{") => {
                Err(CompileError::syntax(
                    "concurrent is a statement and cannot be used as an expression",
                    token.span.start,
                ))
            }
            TokenKind::Ident(value) if value == "throw" => {
                let value = self.parse_slot_expression()?;
                Ok(ParsedExpr::new(
                    Expr::Throw {
                        value: Box::new(value.expr),
                    },
                    SourceSpan {
                        start,
                        end: value.spans.span.end,
                    },
                    vec![value.spans],
                ))
            }
            TokenKind::Ident(value) if value == "rethrow" => {
                let exception = self.parse_slot_expression()?;
                Ok(ParsedExpr::new(
                    Expr::Rethrow {
                        exception: Box::new(exception.expr),
                    },
                    SourceSpan {
                        start,
                        end: exception.spans.span.end,
                    },
                    vec![exception.spans],
                ))
            }
            TokenKind::Ident(value) if value == "catch" => self.parse_catch_expression(start),
            TokenKind::Ident(value) if value == "db" => {
                let saved_header = self.in_statement_header;
                self.in_statement_header = false;
                let result = self.parse_db_expression(token.span);
                self.in_statement_header = saved_header;
                result
            }
            TokenKind::Ident(value) if value == "process" => Err(CompileError::syntax(
                "process has been removed; use actors and dispatch instead",
                token.span.start,
            )),
            TokenKind::Ident(value) if value == "dispatch" => {
                self.parse_dispatch_expression(start)
            }
            TokenKind::Ident(value) => {
                if self.check_symbol("{") && !self.in_statement_header {
                    self.advance();
                    let (fields, children, record_fields) = self.parse_record_construct_fields()?;
                    Ok(ParsedExpr::with_children_and_parts(
                        Expr::Record {
                            type_name: value,
                            type_args: Vec::new(),
                            fields,
                        },
                        SourceSpan {
                            start,
                            end: self.previous().span.end,
                        },
                        children,
                        Vec::new(),
                        record_fields,
                    ))
                } else {
                    Ok(parsed_leaf_expr(Expr::Identifier(value), token.span))
                }
            }
            TokenKind::Symbol(value) if value == "(" => {
                let expr = self.parse_slot_expression()?;
                self.expect_symbol(")")?;
                Ok(expr)
            }
            TokenKind::Symbol(value) if value == "{" => {
                let (entries, children, record_fields) = self.parse_object_literal_entries()?;
                Ok(ParsedExpr::with_children_and_parts(
                    Expr::ObjectLiteral { entries },
                    SourceSpan {
                        start,
                        end: self.previous().span.end,
                    },
                    children,
                    Vec::new(),
                    record_fields,
                ))
            }
            _ => Err(CompileError::syntax(
                "expected expression",
                token.span.start,
            )),
        }
    }

    pub(super) fn parse_dispatch_expression(
        &mut self,
        start: SourceLocation,
    ) -> Result<ParsedExpr> {
        let call = self.parse_expression()?;
        if !matches!(call.expr, Expr::Call { .. }) {
            return Err(CompileError::syntax(
                "dispatch expects a call expression",
                call.spans.span.start,
            ));
        }
        let mut children = vec![call.spans];
        let mut timing = None;
        if self.match_ident("after") {
            self.expect_symbol("(")?;
            let (timing_expr, timing_spans) = self.parse_dispatch_timing_operand()?;
            self.expect_symbol(")")?;
            children.push(timing_spans);
            timing = Some(DispatchTiming::After(Box::new(timing_expr)));
        } else if self.match_ident("at") {
            self.expect_symbol("(")?;
            let (timing_expr, timing_spans) = self.parse_dispatch_timing_operand()?;
            self.expect_symbol(")")?;
            children.push(timing_spans);
            timing = Some(DispatchTiming::At(Box::new(timing_expr)));
        }
        if timing.is_some() && (self.check_ident("after") || self.check_ident("at")) {
            return Err(CompileError::syntax(
                "dispatch accepts at most one timing clause",
                self.peek().span.start,
            ));
        }
        let end = children.last().map(|child| child.span.end).unwrap_or(start);
        Ok(ParsedExpr::new(
            Expr::Dispatch {
                call: Box::new(call.expr),
                timing,
            },
            SourceSpan { start, end },
            children,
        ))
    }

    fn parse_dispatch_timing_operand(&mut self) -> Result<(Expr, ExprSourceSpans)> {
        if self.check_duration_literal() {
            let token = self.advance().clone();
            let TokenKind::Duration(duration) = token.kind else {
                unreachable!("checked duration literal token");
            };
            let milliseconds = duration
                .checked_milliseconds_allow_zero()
                .map_err(|error| CompileError::syntax(error.to_string(), duration.span.start))?;
            let span = duration.span;
            let callee = ParsedExpr::new(
                Expr::Field {
                    object: Box::new(Expr::Identifier("Duration".to_string())),
                    field: "milliseconds".to_string(),
                },
                span,
                vec![expr_source_spans_from_span(span)],
            );
            let argument = ParsedExpr::new(
                Expr::Literal(Literal::Number(milliseconds as f64)),
                span,
                Vec::new(),
            );
            let call = ParsedExpr::new(
                Expr::Call {
                    callee: Box::new(callee.expr),
                    args: vec![CallArg::Value(argument.expr)],
                },
                span,
                vec![callee.spans, argument.spans],
            );
            return Ok(call.into_parts());
        }
        let value = self.parse_slot_expression()?;
        Ok(value.into_parts())
    }

    pub(super) fn parse_timeout_duration(&mut self) -> Result<DurationLiteral> {
        self.expect_symbol("(")?;
        let token = self.advance().clone();
        let TokenKind::Duration(duration) = token.kind else {
            return Err(CompileError::syntax(
                "expected a duration literal in timeout(...)",
                token.span.start,
            ));
        };
        duration
            .checked_milliseconds()
            .map_err(|error| CompileError::syntax(error.to_string(), duration.span.start))?;
        self.expect_symbol(")")?;
        Ok(duration)
    }

    pub(super) fn parse_timeout_expression(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let duration = self.parse_timeout_duration()?;
        self.parse_timeout_value_after_duration(start, duration)
    }

    pub(super) fn parse_timeout_value_after_duration(
        &mut self,
        start: SourceLocation,
        duration: DurationLiteral,
    ) -> Result<ParsedExpr> {
        let value = if self.match_ident("value") {
            let value_start = self.previous().span.start;
            self.parse_value_block_expression(value_start, false)?
        } else if self.match_ident("concurrent") {
            let concurrent_start = self.previous().span.start;
            if !self.match_ident("value") {
                return Err(CompileError::syntax(
                    "noncanonical modifier order; use `timeout(...) concurrent value { ... }`",
                    self.peek().span.start,
                ));
            }
            self.parse_value_block_expression(concurrent_start, true)?
        } else {
            return Err(CompileError::syntax(
                "noncanonical modifier order; timeout value form must be `timeout(...) value { ... }` or `timeout(...) concurrent value { ... }`",
                self.peek().span.start,
            ));
        };
        let end = value.spans.span.end;
        Ok(ParsedExpr::new(
            Expr::Timeout {
                duration,
                value: Box::new(value.expr),
            },
            SourceSpan { start, end },
            vec![value.spans],
        ))
    }

    pub(super) fn parse_value_block_expression(
        &mut self,
        start: SourceLocation,
        concurrent: bool,
    ) -> Result<ParsedExpr> {
        let saved_header = self.in_statement_header;
        self.in_statement_header = false;
        let result = self.parse_value_block_expression_inner(start, concurrent);
        self.in_statement_header = saved_header;
        result
    }

    fn parse_value_block_expression_inner(
        &mut self,
        start: SourceLocation,
        concurrent: bool,
    ) -> Result<ParsedExpr> {
        if self.check_ident("timeout")
            || self.check_ident("concurrent")
            || self.check_ident("serial")
            || self.check_ident("value")
        {
            return Err(CompileError::syntax(
                "noncanonical modifier order; use `value`, `concurrent value`, `timeout(...) value`, or `timeout(...) concurrent value`",
                self.peek().span.start,
            ));
        }
        self.expect_symbol("{")?;
        let block_start = self.previous().span.start;
        let mut statements = Vec::new();
        let mut statement_spans = Vec::new();
        let mut statement_terminated = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            if self.check_symbol("{") {
                return Err(CompileError::syntax(
                    "value block object literal tail must be parenthesized",
                    self.peek().span.start,
                ));
            }
            let mut statement = self.parse_statement(false)?;
            let terminated = self.match_symbol(";");
            if terminated {
                statement.spans.span.end = self.previous().span.end;
            }
            statements.push(statement.stmt);
            statement_spans.push(statement.spans);
            statement_terminated.push(terminated);
        }
        self.expect_symbol("}")?;
        let block_end = self.previous().span.end;
        let missing_tail = || {
            CompileError::syntax(
                "value block requires a tail expression",
                self.previous().span.start,
            )
        };
        let Some(last_statement) = statements.pop() else {
            return Err(missing_tail());
        };
        let Some(last_spans) = statement_spans.pop() else {
            unreachable!("value block statement and span counts must match");
        };
        let Some(last_terminated) = statement_terminated.pop() else {
            unreachable!("value block statement and terminator counts must match");
        };
        if last_terminated {
            return Err(missing_tail());
        }
        let (tail, tail_spans) = match last_statement {
            Stmt::Expr(tail) => {
                let mut tail_expression_spans = last_spans.expressions.into_iter();
                let tail_spans = tail_expression_spans
                    .next()
                    .expect("expression statement must carry expression spans");
                debug_assert!(tail_expression_spans.next().is_none());
                (tail, tail_spans)
            }
            Stmt::Throw { value } => (
                Expr::Throw {
                    value: Box::new(value),
                },
                expr_source_spans(last_spans.span, last_spans.expressions),
            ),
            Stmt::Rethrow { exception } => (
                Expr::Rethrow {
                    exception: Box::new(exception),
                },
                expr_source_spans(last_spans.span, last_spans.expressions),
            ),
            _ => return Err(missing_tail()),
        };
        let body_spans = BlockSourceSpans {
            span: SourceSpan {
                start: block_start,
                end: block_end,
            },
            statements: statement_spans,
        };
        let value = ValueBlock {
            body: Block { statements },
            tail: Box::new(tail),
        };
        Ok(ParsedExpr::with_children_and_parts(
            if concurrent {
                Expr::ConcurrentValue(value)
            } else {
                Expr::ValueBlock(value)
            },
            SourceSpan {
                start,
                end: block_end,
            },
            vec![tail_spans],
            vec![body_spans],
            Vec::new(),
        ))
    }

    pub(super) fn check_dependency_source_address_suffix(&self, expr: &ParsedExpr) -> bool {
        if !matches!(expr.expr, Expr::Identifier(_)) {
            return false;
        }
        let slash = self.peek();
        if !matches!(&slash.kind, TokenKind::Symbol(value) if value == "/") {
            return false;
        }
        let Some(segment) = self.tokens.get(self.current + 1) else {
            return false;
        };
        matches!(segment.kind, TokenKind::Ident(_))
            && contiguous_locations(expr.spans.span.end, slash.span.start)
            && contiguous_locations(slash.span.end, segment.span.start)
    }

    pub(super) fn parse_dependency_source_address(
        &mut self,
        expr: ParsedExpr,
    ) -> Result<ParsedExpr> {
        let Expr::Identifier(dependency_ref) = expr.expr else {
            unreachable!("remote source suffix is only checked for identifiers");
        };
        self.advance();
        let first_segment = self.expect_ident("expected public instance key after /")?;
        let mut segments = vec![first_segment];
        let mut end = self.previous().span.end;
        while self.check_symbol("/") {
            let slash_token = self.peek().clone();
            let Some(next) = self.tokens.get(self.current + 1) else {
                break;
            };
            if !matches!(next.kind, TokenKind::Ident(_))
                || !contiguous_locations(end, slash_token.span.start)
                || !contiguous_locations(slash_token.span.end, next.span.start)
            {
                break;
            }
            self.advance();
            segments.push(self.expect_ident("expected public instance key segment after /")?);
            end = self.previous().span.end;
        }
        let span = SourceSpan {
            start: expr.spans.span.start,
            end,
        };
        Ok(ParsedExpr::new(
            Expr::DependencySourceAddress(DependencySourceAddress {
                dependency_ref,
                public_path: segments.join("."),
            }),
            span,
            Vec::new(),
        ))
    }
    pub(super) fn parse_catch_expression(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        self.expect_symbol("<")?;
        let catch_type = self.parse_type()?;
        self.expect_symbol(">")?;
        self.expect_symbol("(")?;
        let try_expr = self.parse_slot_expression()?;
        self.expect_symbol(")")?;
        Ok(ParsedExpr::new(
            Expr::Catch {
                catch_type,
                try_expr: Box::new(try_expr.expr),
            },
            SourceSpan {
                start,
                end: self.previous().span.end,
            },
            vec![try_expr.spans],
        ))
    }

    #[allow(
        clippy::type_complexity,
        reason = "the tuple keeps the three parallel field and source-span vectors synchronized"
    )]
    pub(super) fn parse_record_construct_fields(
        &mut self,
    ) -> Result<(
        Vec<(String, Expr)>,
        Vec<ExprSourceSpans>,
        Vec<RecordFieldSourceSpans>,
    )> {
        let mut fields = Vec::new();
        let mut spans = Vec::new();
        let mut record_fields = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let field = self.expect_ident("expected record field name")?;
                let field_name_span = self.previous().span;
                self.expect_symbol(":")?;
                let field_value = self.parse_slot_expression()?;
                record_fields.push(RecordFieldSourceSpans {
                    name: field.clone(),
                    name_span: field_name_span,
                    value_span: field_value.spans.span,
                });
                spans.push(field_value.spans);
                fields.push((field, field_value.expr));
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        Ok((fields, spans, record_fields))
    }

    pub(super) fn parse_patch_operations(
        &mut self,
    ) -> Result<(Vec<crate::ast::PatchOperation>, Vec<ExprSourceSpans>)> {
        let mut operations = Vec::new();
        let mut spans = Vec::new();
        while !self.check_symbol("}") {
            let op = self.expect_ident("expected patch operation")?;
            let path = self.parse_field_path("expected patch field path")?;
            match op.as_str() {
                "set" => {
                    self.expect_symbol("=")?;
                    let (value_expr, value_spans) = self.parse_slot_expression()?.into_parts();
                    spans.push(value_spans);
                    operations.push(crate::ast::PatchOperation::Set {
                        path,
                        value: value_expr,
                    });
                }
                "inc" => {
                    self.expect_ident_value("by")?;
                    let (value_expr, value_spans) = self.parse_slot_expression()?.into_parts();
                    spans.push(value_spans);
                    operations.push(crate::ast::PatchOperation::Inc {
                        path,
                        value: value_expr,
                    });
                }
                _ => {
                    return Err(CompileError::syntax(
                        "expected patch operation set or inc",
                        self.previous().span.start,
                    ));
                }
            }
            let _ = self.match_statement_terminator();
        }
        self.expect_symbol("}")?;
        Ok((operations, spans))
    }

    pub(super) fn parse_object_literal_entries(
        &mut self,
    ) -> Result<(
        Vec<crate::ast::ObjectLiteralEntry>,
        Vec<ExprSourceSpans>,
        Vec<RecordFieldSourceSpans>,
    )> {
        let mut entries = Vec::new();
        let mut spans = Vec::new();
        let mut record_fields = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let (key, key_span) = self.parse_object_literal_key()?;
                self.expect_symbol(":")?;
                let value = self.parse_slot_expression()?;
                let field_name = object_literal_key_name(&key);
                if let Some(field_name) = field_name {
                    record_fields.push(RecordFieldSourceSpans {
                        name: field_name,
                        name_span: key_span,
                        value_span: value.spans.span,
                    });
                }
                spans.push(value.spans);
                entries.push(crate::ast::ObjectLiteralEntry {
                    key,
                    key_span: Some(key_span),
                    value: value.expr,
                });
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        Ok((entries, spans, record_fields))
    }

    pub(super) fn parse_object_literal_key(
        &mut self,
    ) -> Result<(crate::ast::ObjectLiteralKey, SourceSpan)> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Ident(value) => Ok((crate::ast::ObjectLiteralKey::Name(value), token.span)),
            TokenKind::Symbol(value) if value == "[" => Err(CompileError::syntax(
                "computed object literal keys are not supported; construct an empty object and call set",
                token.span.start,
            )),
            _ => Err(CompileError::syntax(
                "expected object literal key",
                token.span.start,
            )),
        }
    }
}

fn contiguous_locations(left: SourceLocation, right: SourceLocation) -> bool {
    left.line == right.line && left.column == right.column
}
