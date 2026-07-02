#[cfg(test)]
use std::collections::BTreeMap;

use crate::{shared::ast::Expr, shared::ast_utils::expr_path};

#[cfg(test)]
use crate::{
    shared::prelude_registry::prelude_registry,
    shared::type_syntax::{generic_inner, generic_parts, split_top_level},
};

pub(super) fn collect_emit_expression_call_violations(
    path: &str,
    expr: &Expr,
    violations: &mut Vec<String>,
) {
    if let Expr::Call { callee, .. } = expr {
        if expr_path(callee).as_deref() == Some("emit") {
            violations.push(format!(
                "{path}: emit is a stream statement and cannot be used as an expression"
            ));
        }
    }
    match expr {
        Expr::Call { callee, args } => {
            collect_emit_expression_call_violations(path, callee, violations);
            for arg in args {
                collect_emit_expression_call_violations(path, arg, violations);
            }
        }
        Expr::Generic { callee, .. } | Expr::Unary { expr: callee, .. } => {
            collect_emit_expression_call_violations(path, callee, violations);
        }
        Expr::InterfaceBox { value, .. } => {
            collect_emit_expression_call_violations(path, value, violations);
        }
        Expr::Binary { left, right, .. } => {
            collect_emit_expression_call_violations(path, left, violations);
            collect_emit_expression_call_violations(path, right, violations);
        }
        Expr::Field { object, .. } => {
            collect_emit_expression_call_violations(path, object, violations);
        }
        Expr::Record { fields, .. } => {
            for (_, value) in fields {
                collect_emit_expression_call_violations(path, value, violations);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_emit_expression_call_violations(path, &entry.value, violations);
            }
        }
        Expr::Patch { operations, .. } => {
            for operation in operations {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_emit_expression_call_violations(path, value, violations);
                    }
                }
            }
        }
        Expr::Throw { value } => {
            collect_emit_expression_call_violations(path, value, violations);
        }
        Expr::Rethrow { exception } => {
            collect_emit_expression_call_violations(path, exception, violations);
        }
        Expr::Catch { try_expr, .. } => {
            collect_emit_expression_call_violations(path, try_expr, violations);
        }
        Expr::DbOperation(operation) => {
            collect_emit_db_operation_violations(path, operation, violations)
        }
        Expr::DbQuery(query) => collect_emit_db_query_violations(path, &query.query, violations),
        Expr::DbTransaction(transaction) => {
            for stmt in &transaction.body.statements {
                collect_emit_stmt_violations(path, stmt, violations);
            }
        }
        Expr::DbLeaseClaim(claim) => {
            collect_emit_expression_call_violations(path, &claim.key, violations);
            for stmt in &claim.body.statements {
                collect_emit_stmt_violations(path, stmt, violations);
            }
        }
        Expr::DbLeaseRead(read) => {
            collect_emit_expression_call_violations(path, &read.key, violations);
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
    }
}

pub(super) fn collect_emit_expression_call_violations_in_block(
    path: &str,
    body: &crate::shared::ast::Block,
    violations: &mut Vec<String>,
) {
    for stmt in &body.statements {
        collect_emit_stmt_violations(path, stmt, violations);
    }
}

fn collect_emit_stmt_violations(
    path: &str,
    stmt: &crate::shared::ast::Stmt,
    violations: &mut Vec<String>,
) {
    match stmt {
        crate::shared::ast::Stmt::Let { value, .. }
        | crate::shared::ast::Stmt::Spawn { call: value }
        | crate::shared::ast::Stmt::Expr(value)
        | crate::shared::ast::Stmt::Emit(value) => {
            collect_emit_expression_call_violations(path, value, violations)
        }
        crate::shared::ast::Stmt::Return(value) => {
            if let Some(value) = value {
                collect_emit_expression_call_violations(path, value, violations);
            }
        }
        crate::shared::ast::Stmt::Assign { target, value } => {
            collect_emit_expression_call_violations(path, target, violations);
            collect_emit_expression_call_violations(path, value, violations);
        }
        crate::shared::ast::Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            collect_emit_expression_call_violations(path, condition, violations);
            for stmt in &then_block.statements {
                collect_emit_stmt_violations(path, stmt, violations);
            }
            if let Some(else_block) = else_block {
                for stmt in &else_block.statements {
                    collect_emit_stmt_violations(path, stmt, violations);
                }
            }
        }
        crate::shared::ast::Stmt::For { iterable, body, .. } => {
            collect_emit_expression_call_violations(path, iterable, violations);
            for stmt in &body.statements {
                collect_emit_stmt_violations(path, stmt, violations);
            }
        }
        crate::shared::ast::Stmt::Match { value, arms } => {
            collect_emit_expression_call_violations(path, value, violations);
            for arm in arms {
                for stmt in &arm.body.statements {
                    collect_emit_stmt_violations(path, stmt, violations);
                }
            }
        }
        crate::shared::ast::Stmt::DbTransaction { body } => {
            for stmt in &body.statements {
                collect_emit_stmt_violations(path, stmt, violations);
            }
        }
        crate::shared::ast::Stmt::Assert { condition, .. } => {
            collect_emit_expression_call_violations(path, condition, violations)
        }
        crate::shared::ast::Stmt::Throw { value } => {
            collect_emit_expression_call_violations(path, value, violations)
        }
        crate::shared::ast::Stmt::Rethrow { exception } => {
            collect_emit_expression_call_violations(path, exception, violations)
        }
        crate::shared::ast::Stmt::Break | crate::shared::ast::Stmt::Continue => {}
    }
}

fn collect_emit_db_operation_violations(
    path: &str,
    operation: &crate::shared::ast::DbOperation,
    violations: &mut Vec<String>,
) {
    if let Some(selector) = &operation.selector {
        match selector {
            crate::shared::ast::DbSelector::Key { value } => {
                collect_emit_expression_call_violations(path, value, violations)
            }
            crate::shared::ast::DbSelector::Query { query } => {
                collect_emit_db_query_violations(path, query, violations)
            }
        }
    }
    if let Some(query) = &operation.query {
        collect_emit_db_query_violations(path, query, violations);
    }
    for body in [&operation.body, &operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            crate::shared::ast::DbBody::ObjectFields { fields } => {
                for field in fields {
                    collect_emit_expression_call_violations(path, &field.value, violations);
                }
            }
            crate::shared::ast::DbBody::Values { value } => {
                collect_emit_expression_call_violations(path, value, violations)
            }
        }
    }
    if let Some(change) = &operation.change {
        for op in &change.ops {
            match op {
                crate::shared::ast::DbChangeOp::Set { value, .. }
                | crate::shared::ast::DbChangeOp::Inc { value, .. }
                | crate::shared::ast::DbChangeOp::AddToSet { value, .. }
                | crate::shared::ast::DbChangeOp::Remove { value, .. } => {
                    collect_emit_expression_call_violations(path, value, violations)
                }
                crate::shared::ast::DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn collect_emit_db_query_violations(
    path: &str,
    query: &crate::shared::ast::DbQueryBlock,
    violations: &mut Vec<String>,
) {
    for clause in &query.where_clauses {
        match clause {
            crate::shared::ast::DbWhereClause::Predicate { predicate } => {
                collect_emit_expression_call_violations(path, predicate, violations);
            }
            crate::shared::ast::DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                collect_emit_expression_call_violations(path, condition, violations);
                collect_emit_expression_call_violations(path, predicate, violations);
            }
        }
    }
    if let Some(limit) = &query.limit {
        collect_emit_expression_call_violations(path, limit, violations);
    }
    if let Some(offset) = &query.offset {
        collect_emit_expression_call_violations(path, offset, violations);
    }
    if let Some(after) = &query.after {
        collect_emit_expression_call_violations(path, after, violations);
    }
}

#[cfg(test)]
pub(super) fn infer_expr_type(
    expr: &Expr,
    env: &BTreeMap<String, String>,
    function_return_types: &BTreeMap<String, String>,
) -> Option<String> {
    match expr {
        Expr::Literal(crate::shared::ast::Literal::String(_)) => Some("string".to_string()),
        Expr::Literal(crate::shared::ast::Literal::Number(_)) => Some("number".to_string()),
        Expr::Literal(crate::shared::ast::Literal::Bool(_)) => Some("bool".to_string()),
        Expr::Literal(crate::shared::ast::Literal::Null) => Some("null".to_string()),
        Expr::Identifier(name) => env.get(name).cloned(),
        Expr::RemotePublicInstanceSource(_) => None,
        Expr::Record { type_name, .. } => Some(type_name.clone()),
        Expr::Call { callee, .. } => expr_path(callee).and_then(|path| {
            prelude_registry()
                .native_return_type(&path)
                .or_else(|| function_return_types.get(&path).cloned())
        }),
        Expr::Generic { callee, .. } => infer_expr_type(callee, env, function_return_types),
        Expr::InterfaceBox { value, interface } => {
            infer_expr_type(value, env, function_return_types)
                .map(|_| format!("any {}", interface.name))
        }
        Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Field { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::Patch { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. } => None,
        Expr::DbOperation(operation) => Some(db_operation_result_type(operation)),
        Expr::DbQuery(query) => Some(format!("DbQuery<{}>", query.target.name)),
        Expr::DbTransaction(transaction) => match transaction.mode {
            crate::shared::ast::DbBlockMode::Effect => Some("null".to_string()),
            crate::shared::ast::DbBlockMode::Value => {
                transaction.body.statements.last().and_then(|stmt| {
                    if let crate::shared::ast::Stmt::Expr(value) = stmt {
                        infer_expr_type(value, env, function_return_types)
                    } else {
                        None
                    }
                })
            }
        },
        Expr::DbLeaseClaim(_) => Some("bool".to_string()),
        Expr::DbLeaseRead(_) => {
            Some("{ expiresAt: string, owner: string, requestId: string }?".to_string())
        }
    }
}

#[cfg(test)]
fn db_operation_result_type(operation: &crate::shared::ast::DbOperation) -> String {
    match operation.op {
        crate::shared::ast::DbOperationKind::Find if operation.many => {
            format!(
                "Array<{}>",
                db_read_projection_record_type(operation, &operation.target.name)
            )
        }
        crate::shared::ast::DbOperationKind::Find
        | crate::shared::ast::DbOperationKind::Optional => {
            format!(
                "{}?",
                db_read_projection_record_type(operation, &operation.target.name)
            )
        }
        crate::shared::ast::DbOperationKind::Insert if operation.many => {
            "DbInsertManyResult".to_string()
        }
        crate::shared::ast::DbOperationKind::Update if operation.many => {
            "DbUpdateManyResult".to_string()
        }
        crate::shared::ast::DbOperationKind::Delete if operation.many => {
            "DbDeleteManyResult".to_string()
        }
        crate::shared::ast::DbOperationKind::Require => {
            db_read_projection_record_type(operation, &operation.target.name)
        }
        crate::shared::ast::DbOperationKind::Insert => {
            db_full_projection_record_type(&operation.target.name)
        }
        crate::shared::ast::DbOperationKind::Update
        | crate::shared::ast::DbOperationKind::Replace => {
            format!(
                "{}?",
                db_full_projection_record_type(&operation.target.name)
            )
        }
        crate::shared::ast::DbOperationKind::Upsert => {
            format!(
                "DbUpsertResult<{}>",
                db_full_projection_record_type(&operation.target.name)
            )
        }
        crate::shared::ast::DbOperationKind::Delete
        | crate::shared::ast::DbOperationKind::Exists => "bool".to_string(),
        crate::shared::ast::DbOperationKind::Count => "number".to_string(),
    }
}

#[cfg(test)]
fn db_read_projection_record_type(
    operation: &crate::shared::ast::DbOperation,
    target_name: &str,
) -> String {
    if operation.projection.is_some() {
        format!("ReadonlyProjectionRecord<{target_name}>")
    } else {
        db_full_projection_record_type(target_name)
    }
}

#[cfg(test)]
fn db_full_projection_record_type(target_name: &str) -> String {
    format!("FullReadonlyProjectionRecord<{target_name}>")
}

#[cfg(test)]
pub(super) fn iterable_item_type(ty: &str) -> Option<String> {
    generic_inner(ty.trim(), "Stream")
        .or_else(|| generic_inner(ty.trim(), "Array"))
        .or_else(|| map_entry_types(ty).map(|(key, _value)| key))
        .map(str::to_string)
}

#[cfg(test)]
pub(super) fn map_entry_types(ty: &str) -> Option<(&str, &str)> {
    let parts = generic_parts(ty.trim())?;
    (parts.root.trim() == "Map" && parts.args.len() == 2)
        .then(|| (parts.args[0].trim(), parts.args[1].trim()))
}

#[cfg(test)]
pub(super) fn types_compatible_for_emit(actual: &str, expected: &str) -> bool {
    canonical_type_for_compare(actual) == canonical_type_for_compare(expected)
}

#[cfg(test)]
pub(super) fn types_compatible_for_annotation(actual: &str, expected: &str) -> bool {
    if canonical_type_for_compare(actual) == canonical_type_for_compare(expected) {
        return true;
    }
    actual.trim() == "null" && type_allows_null(expected)
}

#[cfg(test)]
fn type_allows_null(ty: &str) -> bool {
    let ty = ty.trim();
    if ty == "null" || ty.strip_suffix('?').is_some() {
        return true;
    }
    split_top_level(ty, '|')
        .into_iter()
        .any(|part| part.trim() == "null")
}

#[cfg(test)]
fn canonical_type_for_compare(ty: &str) -> String {
    let ty = ty.trim();
    if let Some(inner) = ty.strip_suffix('?') {
        return format!("{}?", canonical_type_for_compare(inner));
    }
    if let Some(parts) = generic_parts(ty) {
        let args = parts
            .args
            .iter()
            .map(|arg| canonical_type_for_compare(arg))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{}<{args}>", canonical_type_for_compare(parts.root));
    }
    prelude_registry()
        .known_type_symbol(ty)
        .unwrap_or_else(|| ty.to_string())
}
