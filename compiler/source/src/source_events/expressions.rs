use crate::shared::ast::{
    BlockSourceSpans, DbBody, DbChangeOp, DbQueryBlock, DbSelector, DbWhereClause, DispatchTiming,
    Expr, ExprSourceSpans,
};

use super::collector::{ensure_exhausted, OwnerCollector};

impl OwnerCollector<'_> {
    pub(super) fn visit_expr(
        &mut self,
        expression: &Expr,
        spans: &ExprSourceSpans,
    ) -> Result<(), String> {
        let preorder_index = self.record_expression(spans)?;
        let mut children = spans.children.iter();
        let mut blocks = spans.blocks.iter();
        match expression {
            Expr::Literal(_) | Expr::Identifier(_) | Expr::DependencySourceAddress(_) => {}
            Expr::Binary { left, right, .. } => {
                self.visit_expr(
                    left,
                    next_expression_child(&mut children, "binary left").map_err(|message| {
                        self.error(format!(
                            "{message}; visiting {} expression at preorder index {preorder_index}, span {:?}",
                            expression_kind(expression),
                            spans.span
                        ))
                    })?,
                )?;
                self.visit_expr(
                    right,
                    next_expression_child(&mut children, "binary right").map_err(|message| {
                        self.error(format!(
                            "{message}; visiting {} expression at preorder index {preorder_index}, span {:?}",
                            expression_kind(expression),
                            spans.span
                        ))
                    })?,
                )?;
            }
            Expr::Unary { expr, .. } => {
                self.visit_expr(expr, next_expression_child(&mut children, "unary operand")?)?
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_expr(
                    condition,
                    next_expression_child(&mut children, "ternary condition")?,
                )?;
                self.visit_expr(
                    then_expr,
                    next_expression_child(&mut children, "ternary then branch")?,
                )?;
                self.visit_expr(
                    else_expr,
                    next_expression_child(&mut children, "ternary else branch")?,
                )?;
            }
            Expr::Call { callee, args } => {
                self.visit_expr(callee, next_expression_child(&mut children, "call callee")?)?;
                for (index, argument) in args.iter().enumerate() {
                    self.visit_expr(
                        argument.expr(),
                        next_expression_child(&mut children, &format!("call arg {index}"))?,
                    )?;
                }
            }
            Expr::Generic { callee, .. } => self.visit_expr(
                callee,
                next_expression_child(&mut children, "generic callee")?,
            )?,
            Expr::InterfaceBox { value, .. } => self.visit_expr(
                value,
                next_expression_child(&mut children, "interface box value")?,
            )?,
            Expr::Field { object, .. } => self.visit_expr(
                object,
                next_expression_child(&mut children, "field object")?,
            )?,
            Expr::Index { object, index } => {
                self.visit_expr(
                    object,
                    next_expression_child(&mut children, "index object")?,
                )?;
                self.visit_expr(
                    index,
                    next_expression_child(&mut children, "index selector")?,
                )?;
            }
            Expr::Record { fields, .. } => {
                if spans.record_fields.len() != fields.len() {
                    return Err(self.error(format!(
                        "record field span count {} does not match AST field count {}",
                        spans.record_fields.len(),
                        fields.len()
                    )));
                }
                skip_syntactic_target_child(&mut children, spans.children.len(), fields.len())?;
                for (index, (_, value)) in fields.iter().enumerate() {
                    self.visit_expr(
                        value,
                        next_expression_child(&mut children, &format!("record field {index}"))?,
                    )?;
                }
            }
            Expr::ObjectLiteral { entries } => {
                if spans.record_fields.len() != entries.len() {
                    return Err(self.error(format!(
                        "object literal field span count {} does not match AST entry count {}",
                        spans.record_fields.len(),
                        entries.len()
                    )));
                }
                for (index, entry) in entries.iter().enumerate() {
                    self.visit_expr(
                        &entry.value,
                        next_expression_child(
                            &mut children,
                            &format!("object literal entry {index}"),
                        )?,
                    )?;
                }
            }
            Expr::Patch { operations, .. } => {
                skip_syntactic_target_child(&mut children, spans.children.len(), operations.len())?;
                for (index, operation) in operations.iter().enumerate() {
                    match operation {
                        crate::shared::ast::PatchOperation::Set { value, .. }
                        | crate::shared::ast::PatchOperation::Inc { value, .. } => self
                            .visit_expr(
                                value,
                                next_expression_child(
                                    &mut children,
                                    &format!("patch operation {index}"),
                                )?,
                            )?,
                    }
                }
            }
            Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
                self.visit_block(
                    &value.body,
                    next_expression_block(&mut blocks, "value body")?,
                )?;
                self.visit_expr(
                    &value.tail,
                    next_expression_child(&mut children, "value tail")?,
                )?;
            }
            Expr::Timeout { value, .. } => self.visit_expr(
                value,
                next_expression_child(&mut children, "timeout value")?,
            )?,
            Expr::Throw { value } => {
                self.visit_expr(value, next_expression_child(&mut children, "throw value")?)?
            }
            Expr::Rethrow { exception } => self.visit_expr(
                exception,
                next_expression_child(&mut children, "rethrow exception")?,
            )?,
            Expr::Catch { try_expr, .. } => self.visit_expr(
                try_expr,
                next_expression_child(&mut children, "catch try expr")?,
            )?,
            Expr::DbOperation(operation) => {
                if let Some(selector) = &operation.selector {
                    self.visit_db_selector(selector, &mut children)?;
                }
                if let Some(query) = operation.independent_query() {
                    self.visit_db_query(query, &mut children)?;
                }
                for body in [&operation.body, &operation.insert_body]
                    .into_iter()
                    .flatten()
                {
                    self.visit_db_body(body, &mut children)?;
                }
                if let Some(change) = &operation.change {
                    for (index, operation) in change.ops.iter().enumerate() {
                        match operation {
                            DbChangeOp::Set { value, .. }
                            | DbChangeOp::Inc { value, .. }
                            | DbChangeOp::AddToSet { value, .. }
                            | DbChangeOp::Remove { value, .. } => self.visit_expr(
                                value,
                                next_expression_child(
                                    &mut children,
                                    &format!("db change operation {index}"),
                                )?,
                            )?,
                            DbChangeOp::Unset { .. } => {}
                        }
                    }
                }
            }
            Expr::DbQuery(query) => self.visit_db_query(&query.query, &mut children)?,
            Expr::DbTransaction(transaction) => self.visit_block(
                &transaction.body,
                next_expression_block(&mut blocks, "db transaction body")?,
            )?,
            Expr::DbLeaseClaim(claim) => {
                self.visit_expr(
                    &claim.key,
                    next_expression_child(&mut children, "db lease key")?,
                )?;
                self.visit_block(
                    &claim.body,
                    next_expression_block(&mut blocks, "db lease claim body")?,
                )?;
            }
            Expr::DbLeaseRead(read) => self.visit_expr(
                &read.key,
                next_expression_child(&mut children, "db lease key")?,
            )?,
            Expr::Dispatch { call, timing } => {
                self.visit_expr(call, next_expression_child(&mut children, "dispatch call")?)?;
                if let Some(DispatchTiming::After(value) | DispatchTiming::At(value)) = timing {
                    self.visit_expr(
                        value,
                        next_expression_child(&mut children, "dispatch timing")?,
                    )?;
                }
            }
        }
        ensure_exhausted(children, || self.error("unused expression child span"))?;
        ensure_exhausted(blocks, || self.error("unused expression block span"))?;
        Ok(())
    }

    fn visit_db_selector<'a>(
        &mut self,
        selector: &DbSelector,
        children: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    ) -> Result<(), String> {
        match selector {
            DbSelector::Key { value } => {
                self.visit_expr(value, next_expression_child(children, "db selector key")?)
            }
            DbSelector::Query { query } => self.visit_db_query(query, children),
        }
    }

    fn visit_db_query<'a>(
        &mut self,
        query: &DbQueryBlock,
        children: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    ) -> Result<(), String> {
        for clause in &query.where_clauses {
            match clause {
                DbWhereClause::Predicate { predicate } => self.visit_expr(
                    predicate,
                    next_expression_child(children, "db where predicate")?,
                )?,
                DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    self.visit_expr(
                        condition,
                        next_expression_child(children, "db where condition")?,
                    )?;
                    self.visit_expr(
                        predicate,
                        next_expression_child(children, "db where predicate")?,
                    )?;
                }
            }
        }
        if let Some(limit) = &query.limit {
            self.visit_expr(limit, next_expression_child(children, "db query limit")?)?;
        }
        if let Some(offset) = &query.offset {
            self.visit_expr(offset, next_expression_child(children, "db query offset")?)?;
        }
        if let Some(after) = &query.after {
            self.visit_expr(after, next_expression_child(children, "db query after")?)?;
        }
        Ok(())
    }

    fn visit_db_body<'a>(
        &mut self,
        body: &DbBody,
        children: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    ) -> Result<(), String> {
        match body {
            DbBody::ObjectFields { fields } => {
                for (index, field) in fields.iter().enumerate() {
                    self.visit_expr(
                        &field.value,
                        next_expression_child(children, &format!("db body field {index}"))?,
                    )?;
                }
            }
            DbBody::Values { value } => {
                self.visit_expr(value, next_expression_child(children, "db body values")?)?
            }
        }
        Ok(())
    }
}

fn next_expression_child<'a>(
    children: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    label: &str,
) -> Result<&'a ExprSourceSpans, String> {
    children
        .next()
        .ok_or_else(|| format!("missing expression child span for {label}"))
}

fn next_expression_block<'a>(
    blocks: &mut impl Iterator<Item = &'a BlockSourceSpans>,
    label: &str,
) -> Result<&'a BlockSourceSpans, String> {
    blocks
        .next()
        .ok_or_else(|| format!("missing expression block span for {label}"))
}

fn skip_syntactic_target_child<'a>(
    children: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    child_count: usize,
    ast_child_count: usize,
) -> Result<(), String> {
    if child_count == ast_child_count {
        return Ok(());
    }
    if child_count == ast_child_count + 1 {
        children
            .next()
            .ok_or_else(|| "missing syntactic target span".to_string())?;
        return Ok(());
    }
    Err(format!(
        "expression span child count {child_count} does not match AST child count {ast_child_count}"
    ))
}

fn expression_kind(expression: &Expr) -> &'static str {
    match expression {
        Expr::Literal(_) => "literal",
        Expr::Identifier(_) => "identifier",
        Expr::DependencySourceAddress(_) => "dependency source address",
        Expr::Binary { .. } => "binary",
        Expr::Unary { .. } => "unary",
        Expr::Ternary { .. } => "ternary",
        Expr::Call { .. } => "call",
        Expr::Generic { .. } => "generic",
        Expr::InterfaceBox { .. } => "interface box",
        Expr::Field { .. } => "field",
        Expr::Index { .. } => "index",
        Expr::Record { .. } => "record",
        Expr::ObjectLiteral { .. } => "object literal",
        Expr::Patch { .. } => "patch",
        Expr::ValueBlock(_) => "value block",
        Expr::ConcurrentValue(_) => "concurrent value",
        Expr::Timeout { .. } => "timeout",
        Expr::Throw { .. } => "throw",
        Expr::Rethrow { .. } => "rethrow",
        Expr::Catch { .. } => "catch",
        Expr::DbOperation(_) => "db operation",
        Expr::DbQuery(_) => "db query",
        Expr::DbTransaction(_) => "db transaction",
        Expr::DbLeaseClaim(_) => "db lease claim",
        Expr::DbLeaseRead(_) => "db lease read",
        Expr::Dispatch { .. } => "dispatch",
    }
}
