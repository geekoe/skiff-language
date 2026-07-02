use std::collections::BTreeMap;

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::ast::{
        Block, BlockSourceSpans, DbBody, DbChangeOp, DbIndexWhereSourceSpans, DbQueryBlock,
        DbSelector, DbWhereClause, Expr, ExprSourceSpans, MatchArm, RecordFieldSourceSpans,
        SourceFile, Stmt, StmtSourceSpans,
    },
    shared::error::SourceSpan,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExpressionKey {
    module_path: String,
    owner: ExpressionOwnerKey,
    preorder_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExpressionOwnerKey {
    Function(String),
    ImplMethod { type_name: String, method: String },
    Const(String),
    Test(String),
    DbIndexWhere { db: String, index: String },
}

#[derive(Clone, Debug)]
pub struct ExpressionSourceFact {
    pub span: SourceSpan,
    pub record_fields: Vec<RecordFieldSourceSpans>,
}

#[derive(Clone, Debug, Default)]
pub struct ExpressionSourceMap {
    facts: BTreeMap<ExpressionKey, ExpressionSourceFact>,
}

struct OwnerCollector<'a> {
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    next_index: u32,
    facts: &'a mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
}

impl ExpressionKey {
    pub fn new(
        module_path: impl Into<String>,
        owner: ExpressionOwnerKey,
        preorder_index: u32,
    ) -> Self {
        Self {
            module_path: module_path.into(),
            owner,
            preorder_index,
        }
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    pub fn owner(&self) -> &ExpressionOwnerKey {
        &self.owner
    }

    pub fn preorder_index(&self) -> u32 {
        self.preorder_index
    }
}

impl ExpressionSourceMap {
    pub fn build(parsed_sources: &[ParsedCompilerSource]) -> Result<Self, String> {
        let mut facts = BTreeMap::new();
        for parsed in parsed_sources {
            if parsed.source().is_test_file {
                continue;
            }
            collect_source_expression_spans(
                parsed.source().module_path.as_str(),
                parsed.ast(),
                &mut facts,
            )?;
        }
        Ok(Self { facts })
    }

    pub fn fact(&self, key: &ExpressionKey) -> Option<&ExpressionSourceFact> {
        self.facts.get(key)
    }

    #[cfg(test)]
    pub fn facts(&self) -> &BTreeMap<ExpressionKey, ExpressionSourceFact> {
        &self.facts
    }
}

impl OwnerCollector<'_> {
    fn visit_expr(&mut self, expr: &Expr, spans: &ExprSourceSpans) -> Result<(), String> {
        let preorder_index = self.next_index;
        let key = ExpressionKey::new(
            self.module_path.to_string(),
            self.owner.clone(),
            preorder_index,
        );
        self.next_index = self
            .next_index
            .checked_add(1)
            .ok_or_else(|| self.error("too many expressions in owner"))?;
        self.facts.insert(
            key,
            ExpressionSourceFact {
                span: spans.span,
                record_fields: spans.record_fields.clone(),
            },
        );

        let mut children = spans.children.iter();
        let mut blocks = spans.blocks.iter();
        match expr {
            Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
            Expr::Binary { left, right, .. } => {
                self.visit_expr(
                    left,
                    next_expr_child(&mut children, "binary left").map_err(|message| {
                        self.error(format!(
                            "{message}; visiting {} expression at preorder index {preorder_index}, span {:?}",
                            expr_kind(expr),
                            spans.span
                        ))
                    })?,
                )?;
                self.visit_expr(
                    right,
                    next_expr_child(&mut children, "binary right").map_err(|message| {
                        self.error(format!(
                            "{message}; visiting {} expression at preorder index {preorder_index}, span {:?}",
                            expr_kind(expr),
                            spans.span
                        ))
                    })?,
                )?;
            }
            Expr::Unary { expr, .. } => {
                self.visit_expr(expr, next_expr_child(&mut children, "unary operand")?)?
            }
            Expr::Call { callee, args } => {
                self.visit_expr(callee, next_expr_child(&mut children, "call callee")?)?;
                for (index, arg) in args.iter().enumerate() {
                    self.visit_expr(
                        arg,
                        next_expr_child(&mut children, &format!("call arg {index}"))?,
                    )?;
                }
            }
            Expr::Generic { callee, .. } => {
                self.visit_expr(callee, next_expr_child(&mut children, "generic callee")?)?
            }
            Expr::InterfaceBox { value, .. } => self.visit_expr(
                value,
                next_expr_child(&mut children, "interface box value")?,
            )?,
            Expr::Field { object, .. } => {
                self.visit_expr(object, next_expr_child(&mut children, "field object")?)?
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
                        next_expr_child(&mut children, &format!("record field {index}"))?,
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
                        next_expr_child(&mut children, &format!("object literal entry {index}"))?,
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
                                next_expr_child(
                                    &mut children,
                                    &format!("patch operation {index}"),
                                )?,
                            )?,
                    }
                }
            }
            Expr::Throw { value } => {
                self.visit_expr(value, next_expr_child(&mut children, "throw value")?)?
            }
            Expr::Rethrow { exception } => self.visit_expr(
                exception,
                next_expr_child(&mut children, "rethrow exception")?,
            )?,
            Expr::Catch { try_expr, .. } => {
                self.visit_expr(try_expr, next_expr_child(&mut children, "catch try expr")?)?
            }
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
                    for (index, op) in change.ops.iter().enumerate() {
                        match op {
                            DbChangeOp::Set { value, .. }
                            | DbChangeOp::Inc { value, .. }
                            | DbChangeOp::AddToSet { value, .. }
                            | DbChangeOp::Remove { value, .. } => self.visit_expr(
                                value,
                                next_expr_child(
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
            Expr::DbTransaction(transaction) => {
                self.visit_block(
                    &transaction.body,
                    next_block_child(&mut blocks, "db transaction body")?,
                )?;
            }
            Expr::DbLeaseClaim(claim) => {
                self.visit_expr(&claim.key, next_expr_child(&mut children, "db lease key")?)?;
                self.visit_block(
                    &claim.body,
                    next_block_child(&mut blocks, "db lease claim body")?,
                )?;
            }
            Expr::DbLeaseRead(read) => {
                self.visit_expr(&read.key, next_expr_child(&mut children, "db lease key")?)?;
            }
        }
        assert_no_remaining_expr_children(children, || self.error("unused expression child span"))?;
        assert_no_remaining_block_children(blocks, || self.error("unused expression block span"))?;
        Ok(())
    }

    fn visit_block(&mut self, block: &Block, spans: &BlockSourceSpans) -> Result<(), String> {
        if block.statements.len() != spans.statements.len() {
            return Err(self.error(format!(
                "block statement span count {} does not match AST statement count {}",
                spans.statements.len(),
                block.statements.len()
            )));
        }
        for (stmt, spans) in block.statements.iter().zip(&spans.statements) {
            self.visit_stmt(stmt, spans)?;
        }
        Ok(())
    }

    fn visit_stmt(&mut self, stmt: &Stmt, spans: &StmtSourceSpans) -> Result<(), String> {
        let mut expressions = spans.expressions.iter();
        let mut blocks = spans.blocks.iter();
        match stmt {
            Stmt::Assert { condition, .. } => self.visit_expr(
                condition,
                next_stmt_expr(&mut expressions, "assert condition")?,
            )?,
            Stmt::Let { value, .. } => {
                self.visit_expr(value, next_stmt_expr(&mut expressions, "let value")?)?
            }
            Stmt::Assign { target, value } => {
                self.visit_expr(target, next_stmt_expr(&mut expressions, "assign target")?)?;
                self.visit_expr(value, next_stmt_expr(&mut expressions, "assign value")?)?;
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.visit_expr(condition, next_stmt_expr(&mut expressions, "if condition")?)?;
                self.visit_block(then_block, next_stmt_block(&mut blocks, "if then block")?)?;
                if let Some(else_block) = else_block {
                    self.visit_block(else_block, next_stmt_block(&mut blocks, "if else block")?)?;
                }
            }
            Stmt::For { iterable, body, .. } => {
                self.visit_expr(iterable, next_stmt_expr(&mut expressions, "for iterable")?)?;
                self.visit_block(body, next_stmt_block(&mut blocks, "for body")?)?;
            }
            Stmt::Match { value, arms } => {
                self.visit_expr(value, next_stmt_expr(&mut expressions, "match value")?)?;
                for (index, arm) in arms.iter().enumerate() {
                    self.visit_match_arm(
                        arm,
                        next_stmt_block(&mut blocks, &format!("match arm {index}"))?,
                    )?;
                }
            }
            Stmt::DbTransaction { body } => self.visit_block(
                body,
                next_stmt_block(&mut blocks, "db transaction stmt body")?,
            )?,
            Stmt::Throw { value } => {
                self.visit_expr(value, next_stmt_expr(&mut expressions, "throw stmt value")?)?
            }
            Stmt::Rethrow { exception } => self.visit_expr(
                exception,
                next_stmt_expr(&mut expressions, "rethrow stmt exception")?,
            )?,
            Stmt::Emit(value) => {
                self.visit_expr(value, next_stmt_expr(&mut expressions, "emit value")?)?
            }
            Stmt::Spawn { call } => {
                self.visit_expr(call, next_stmt_expr(&mut expressions, "spawn call")?)?
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.visit_expr(value, next_stmt_expr(&mut expressions, "return value")?)?;
                }
            }
            Stmt::Expr(value) => {
                self.visit_expr(value, next_stmt_expr(&mut expressions, "stmt expr")?)?
            }
            Stmt::Break | Stmt::Continue => {}
        }
        assert_no_remaining_stmt_expressions(expressions, || {
            self.error("unused statement expression span")
        })?;
        assert_no_remaining_stmt_blocks(blocks, || self.error("unused statement block span"))?;
        Ok(())
    }

    fn visit_match_arm(&mut self, arm: &MatchArm, spans: &BlockSourceSpans) -> Result<(), String> {
        self.visit_block(&arm.body, spans)
    }

    fn visit_db_selector<'b>(
        &mut self,
        selector: &DbSelector,
        children: &mut impl Iterator<Item = &'b ExprSourceSpans>,
    ) -> Result<(), String> {
        match selector {
            DbSelector::Key { value } => {
                self.visit_expr(value, next_expr_child(children, "db selector key")?)
            }
            DbSelector::Query { query } => self.visit_db_query(query, children),
        }
    }

    fn visit_db_query<'b>(
        &mut self,
        query: &DbQueryBlock,
        children: &mut impl Iterator<Item = &'b ExprSourceSpans>,
    ) -> Result<(), String> {
        for clause in &query.where_clauses {
            match clause {
                DbWhereClause::Predicate { predicate } => {
                    self.visit_expr(predicate, next_expr_child(children, "db where predicate")?)?
                }
                DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    self.visit_expr(condition, next_expr_child(children, "db where condition")?)?;
                    self.visit_expr(predicate, next_expr_child(children, "db where predicate")?)?;
                }
            }
        }
        if let Some(limit) = &query.limit {
            self.visit_expr(limit, next_expr_child(children, "db query limit")?)?;
        }
        if let Some(offset) = &query.offset {
            self.visit_expr(offset, next_expr_child(children, "db query offset")?)?;
        }
        if let Some(after) = &query.after {
            self.visit_expr(after, next_expr_child(children, "db query after")?)?;
        }
        Ok(())
    }

    fn visit_db_body<'b>(
        &mut self,
        body: &DbBody,
        children: &mut impl Iterator<Item = &'b ExprSourceSpans>,
    ) -> Result<(), String> {
        match body {
            DbBody::ObjectFields { fields } => {
                for (index, field) in fields.iter().enumerate() {
                    self.visit_expr(
                        &field.value,
                        next_expr_child(children, &format!("db body field {index}"))?,
                    )?;
                }
            }
            DbBody::Values { value } => {
                self.visit_expr(value, next_expr_child(children, "db body values")?)?
            }
        }
        Ok(())
    }

    fn error(&self, message: impl Into<String>) -> String {
        format!(
            "expression span model mismatch in module {} owner {:?}: {}",
            self.module_path,
            self.owner,
            message.into()
        )
    }
}

fn collect_source_expression_spans(
    module_path: &str,
    ast: &SourceFile,
    facts: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    let mut function_span_index = 0usize;
    for function in &ast.functions {
        if function.is_native || function.is_provider {
            continue;
        }
        let spans = ast
            .source_spans
            .functions
            .get(function_span_index)
            .ok_or_else(|| {
                format!(
                    "expression span model mismatch in module {module_path}: missing function body span for {}",
                    function.name
                )
            })?;
        collect_owner_block(
            module_path,
            ExpressionOwnerKey::Function(function.name.clone()),
            &function.body,
            &spans.body,
            facts,
        )?;
        function_span_index += 1;
    }
    assert_len(
        function_span_index,
        ast.source_spans.functions.len(),
        module_path,
        "function body spans",
    )?;

    let mut impl_span_index = 0usize;
    for implementation in &ast.impls {
        for method in &implementation.method_bodies {
            if method.is_native || method.is_provider {
                continue;
            }
            let spans = ast
                .source_spans
                .impl_methods
                .get(impl_span_index)
                .ok_or_else(|| {
                    format!(
                        "expression span model mismatch in module {module_path}: missing impl method span for {}",
                        impl_method_declaration_name(&implementation.target, &method.name)
                    )
                })?;
            collect_owner_block(
                module_path,
                ExpressionOwnerKey::ImplMethod {
                    type_name: implementation.target.clone(),
                    method: method.name.clone(),
                },
                &method.body,
                &spans.body,
                facts,
            )?;
            impl_span_index += 1;
        }
    }
    assert_len(
        impl_span_index,
        ast.source_spans.impl_methods.len(),
        module_path,
        "impl method body spans",
    )?;

    for (constant, spans) in ast.consts.iter().zip(&ast.source_spans.consts) {
        collect_owner_expr(
            module_path,
            ExpressionOwnerKey::Const(constant.name.clone()),
            &constant.value,
            spans,
            facts,
        )?;
    }
    assert_len(
        ast.consts.len(),
        ast.source_spans.consts.len(),
        module_path,
        "const initializer spans",
    )?;

    for (test, spans) in ast.tests.iter().zip(&ast.source_spans.tests) {
        collect_owner_block(
            module_path,
            ExpressionOwnerKey::Test(test.name.clone()),
            &test.body,
            &spans.body,
            facts,
        )?;
    }
    assert_len(
        ast.tests.len(),
        ast.source_spans.tests.len(),
        module_path,
        "test body spans",
    )?;

    for where_spans in &ast.source_spans.db_index_wheres {
        let where_expr = db_index_where_expr(ast, where_spans, module_path)?;
        collect_owner_expr(
            module_path,
            ExpressionOwnerKey::DbIndexWhere {
                db: where_spans.db_name.clone(),
                index: where_spans.index_name.clone(),
            },
            where_expr,
            &where_spans.expression,
            facts,
        )?;
    }

    Ok(())
}

fn collect_owner_block(
    module_path: &str,
    owner: ExpressionOwnerKey,
    block: &Block,
    spans: &BlockSourceSpans,
    facts: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    let mut collector = OwnerCollector {
        module_path,
        owner,
        next_index: 0,
        facts,
    };
    collector.visit_block(block, spans)
}

fn collect_owner_expr(
    module_path: &str,
    owner: ExpressionOwnerKey,
    expr: &Expr,
    spans: &ExprSourceSpans,
    facts: &mut BTreeMap<ExpressionKey, ExpressionSourceFact>,
) -> Result<(), String> {
    let mut collector = OwnerCollector {
        module_path,
        owner,
        next_index: 0,
        facts,
    };
    collector.visit_expr(expr, spans)
}

fn db_index_where_expr<'a>(
    ast: &'a SourceFile,
    spans: &DbIndexWhereSourceSpans,
    module_path: &str,
) -> Result<&'a Expr, String> {
    ast.dbs
        .iter()
        .find(|db| db.name == spans.db_name)
        .and_then(|db| {
            db.indexes
                .iter()
                .find(|index| index.name == spans.index_name)
                .and_then(|index| index.where_expr.as_ref())
        })
        .ok_or_else(|| {
            format!(
                "expression span model mismatch in module {module_path}: missing db index where expr for {}.{}",
                spans.db_name, spans.index_name
            )
        })
}

fn assert_len(
    ast_len: usize,
    span_len: usize,
    module_path: &str,
    label: &str,
) -> Result<(), String> {
    if ast_len == span_len {
        return Ok(());
    }
    Err(format!(
        "expression span model mismatch in module {module_path}: {label} count {span_len} does not match AST count {ast_len}"
    ))
}

fn next_expr_child<'a>(
    children: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    label: &str,
) -> Result<&'a ExprSourceSpans, String> {
    children
        .next()
        .ok_or_else(|| format!("missing expression child span for {label}"))
}

fn expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Literal(_) => "literal",
        Expr::Identifier(_) => "identifier",
        Expr::RemotePublicInstanceSource(_) => "remote public instance source",
        Expr::Binary { .. } => "binary",
        Expr::Unary { .. } => "unary",
        Expr::Call { .. } => "call",
        Expr::Generic { .. } => "generic",
        Expr::InterfaceBox { .. } => "interface box",
        Expr::Field { .. } => "field",
        Expr::Record { .. } => "record",
        Expr::ObjectLiteral { .. } => "object literal",
        Expr::Patch { .. } => "patch",
        Expr::Throw { .. } => "throw",
        Expr::Rethrow { .. } => "rethrow",
        Expr::Catch { .. } => "catch",
        Expr::DbOperation(_) => "db operation",
        Expr::DbQuery(_) => "db query",
        Expr::DbTransaction(_) => "db transaction",
        Expr::DbLeaseClaim(_) => "db lease claim",
        Expr::DbLeaseRead(_) => "db lease read",
    }
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

fn next_block_child<'a>(
    blocks: &mut impl Iterator<Item = &'a BlockSourceSpans>,
    label: &str,
) -> Result<&'a BlockSourceSpans, String> {
    blocks
        .next()
        .ok_or_else(|| format!("missing expression block span for {label}"))
}

fn next_stmt_expr<'a>(
    expressions: &mut impl Iterator<Item = &'a ExprSourceSpans>,
    label: &str,
) -> Result<&'a ExprSourceSpans, String> {
    expressions
        .next()
        .ok_or_else(|| format!("missing statement expression span for {label}"))
}

fn next_stmt_block<'a>(
    blocks: &mut impl Iterator<Item = &'a BlockSourceSpans>,
    label: &str,
) -> Result<&'a BlockSourceSpans, String> {
    blocks
        .next()
        .ok_or_else(|| format!("missing statement block span for {label}"))
}

fn assert_no_remaining_expr_children<'a>(
    mut children: impl Iterator<Item = &'a ExprSourceSpans>,
    error: impl FnOnce() -> String,
) -> Result<(), String> {
    if children.next().is_some() {
        Err(error())
    } else {
        Ok(())
    }
}

fn assert_no_remaining_block_children<'a>(
    mut blocks: impl Iterator<Item = &'a BlockSourceSpans>,
    error: impl FnOnce() -> String,
) -> Result<(), String> {
    if blocks.next().is_some() {
        Err(error())
    } else {
        Ok(())
    }
}

fn assert_no_remaining_stmt_expressions<'a>(
    mut expressions: impl Iterator<Item = &'a ExprSourceSpans>,
    error: impl FnOnce() -> String,
) -> Result<(), String> {
    if expressions.next().is_some() {
        Err(error())
    } else {
        Ok(())
    }
}

fn assert_no_remaining_stmt_blocks<'a>(
    mut blocks: impl Iterator<Item = &'a BlockSourceSpans>,
    error: impl FnOnce() -> String,
) -> Result<(), String> {
    if blocks.next().is_some() {
        Err(error())
    } else {
        Ok(())
    }
}
