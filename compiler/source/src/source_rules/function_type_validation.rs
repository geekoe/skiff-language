use crate::{
    shared::ast::{Block, Expr, InterfaceOperation, SourceFile, Stmt},
    shared::type_expr::TypeExpr,
};

pub fn collect_user_function_type_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    for ty in &ast.types {
        if let Some(alias) = &ty.alias {
            collect_function_type_name_violations(path, &alias.name, violations);
        }
        for field in &ty.fields {
            collect_function_type_name_violations(path, &field.ty.name, violations);
        }
    }
    for interface in &ast.interfaces {
        for operation in &interface.operations {
            collect_operation_function_type_violations(path, operation, violations);
        }
    }
    for operation in &ast.function_signatures {
        collect_operation_function_type_violations(path, operation, violations);
    }
    for function in &ast.functions {
        for param in &function.params {
            collect_function_type_name_violations(path, &param.ty.name, violations);
        }
        collect_function_type_name_violations(path, &function.return_type.name, violations);
        collect_block_function_type_violations(path, &function.body, violations);
    }
    for implementation in &ast.impls {
        for method in &implementation.methods {
            collect_operation_function_type_violations(path, method, violations);
        }
        for method in &implementation.method_bodies {
            for param in &method.params {
                collect_function_type_name_violations(path, &param.ty.name, violations);
            }
            collect_function_type_name_violations(path, &method.return_type.name, violations);
            collect_block_function_type_violations(path, &method.body, violations);
        }
    }
}

fn collect_operation_function_type_violations(
    path: &str,
    operation: &InterfaceOperation,
    violations: &mut Vec<String>,
) {
    for param in &operation.params {
        collect_function_type_name_violations(path, &param.ty.name, violations);
    }
    collect_function_type_name_violations(path, &operation.return_type.name, violations);
}

fn collect_function_type_name_violations(path: &str, ty: &str, violations: &mut Vec<String>) {
    let ty = TypeExpr::parse_lossy(ty);
    if !ty.contains_function_type() {
        return;
    }
    ty.for_each_function_type(|function_type| {
        violations.push(format!(
            "{path}: callback function type {} is only allowed in standard_library/platform native API metadata",
            function_type.to_type_string()
        ));
    });
}

fn collect_block_function_type_violations(path: &str, block: &Block, violations: &mut Vec<String>) {
    for statement in &block.statements {
        collect_stmt_function_type_violations(path, statement, violations);
    }
}

fn collect_stmt_function_type_violations(path: &str, stmt: &Stmt, violations: &mut Vec<String>) {
    match stmt {
        Stmt::Let { ty, value, .. } => {
            if let Some(ty) = ty {
                collect_function_type_name_violations(path, &ty.name, violations);
            }
            collect_expr_function_type_violations(path, value, violations);
        }
        Stmt::Assign { target, value } => {
            collect_expr_function_type_violations(path, target, violations);
            collect_expr_function_type_violations(path, value, violations);
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            collect_expr_function_type_violations(path, condition, violations);
            collect_block_function_type_violations(path, then_block, violations);
            if let Some(else_block) = else_block {
                collect_block_function_type_violations(path, else_block, violations);
            }
        }
        Stmt::For { iterable, body, .. } => {
            collect_expr_function_type_violations(path, iterable, violations);
            collect_block_function_type_violations(path, body, violations);
        }
        Stmt::Match { value, arms } => {
            collect_expr_function_type_violations(path, value, violations);
            for arm in arms {
                collect_block_function_type_violations(path, &arm.body, violations);
            }
        }
        Stmt::Assert { condition, .. } => {
            collect_expr_function_type_violations(path, condition, violations);
        }
        Stmt::DbTransaction { body } => {
            collect_block_function_type_violations(path, body, violations);
        }
        Stmt::Emit(value) | Stmt::Expr(value) => {
            collect_expr_function_type_violations(path, value, violations);
        }
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_expr_function_type_violations(path, value, violations);
            }
        }
        Stmt::Throw { value } => {
            collect_expr_function_type_violations(path, value, violations);
        }
        Stmt::Rethrow { exception } => {
            collect_expr_function_type_violations(path, exception, violations);
        }
        Stmt::Spawn { call } => {
            collect_expr_function_type_violations(path, call, violations);
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

fn collect_expr_function_type_violations(path: &str, expr: &Expr, violations: &mut Vec<String>) {
    match expr {
        Expr::Binary { left, right, .. } => {
            collect_expr_function_type_violations(path, left, violations);
            collect_expr_function_type_violations(path, right, violations);
        }
        Expr::Unary { expr, .. } => collect_expr_function_type_violations(path, expr, violations),
        Expr::Call { callee, args } => {
            collect_expr_function_type_violations(path, callee, violations);
            for arg in args {
                collect_expr_function_type_violations(path, arg, violations);
            }
        }
        Expr::Generic { callee, type_args } => {
            collect_expr_function_type_violations(path, callee, violations);
            for type_arg in type_args {
                collect_function_type_name_violations(path, &type_arg.name, violations);
            }
        }
        Expr::InterfaceBox { value, interface } => {
            collect_expr_function_type_violations(path, value, violations);
            collect_function_type_name_violations(path, &interface.name, violations);
        }
        Expr::Field { object, .. } => {
            collect_expr_function_type_violations(path, object, violations)
        }
        Expr::Record { fields, .. } => {
            for (_, value) in fields {
                collect_expr_function_type_violations(path, value, violations);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_expr_function_type_violations(path, &entry.value, violations);
            }
        }
        Expr::Patch { target, operations } => {
            collect_function_type_name_violations(path, &target.name, violations);
            for operation in operations {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_expr_function_type_violations(path, value, violations);
                    }
                }
            }
        }
        Expr::Throw { value } => collect_expr_function_type_violations(path, value, violations),
        Expr::Rethrow { exception } => {
            collect_expr_function_type_violations(path, exception, violations)
        }
        Expr::Catch {
            catch_type,
            try_expr,
        } => {
            collect_function_type_name_violations(path, &catch_type.name, violations);
            collect_expr_function_type_violations(path, try_expr, violations);
        }
        Expr::DbOperation(operation) => {
            collect_function_type_name_violations(path, &operation.target.name, violations);
            collect_db_operation_function_type_violations(path, operation, violations);
        }
        Expr::DbQuery(query) => {
            collect_function_type_name_violations(path, &query.target.name, violations);
            collect_db_query_function_type_violations(path, &query.query, violations);
        }
        Expr::DbTransaction(transaction) => {
            collect_block_function_type_violations(path, &transaction.body, violations);
        }
        Expr::DbLeaseClaim(claim) => {
            collect_function_type_name_violations(path, &claim.target.name, violations);
            collect_expr_function_type_violations(path, &claim.key, violations);
            collect_block_function_type_violations(path, &claim.body, violations);
        }
        Expr::DbLeaseRead(read) => {
            collect_function_type_name_violations(path, &read.target.name, violations);
            collect_expr_function_type_violations(path, &read.key, violations);
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
    }
}

fn collect_db_operation_function_type_violations(
    path: &str,
    operation: &crate::shared::ast::DbOperation,
    violations: &mut Vec<String>,
) {
    if let Some(selector) = &operation.selector {
        match selector {
            crate::shared::ast::DbSelector::Key { value } => {
                collect_expr_function_type_violations(path, value, violations)
            }
            crate::shared::ast::DbSelector::Query { query } => {
                collect_db_query_function_type_violations(path, query, violations)
            }
        }
    }
    if let Some(query) = &operation.query {
        collect_db_query_function_type_violations(path, query, violations);
    }
    for body in [&operation.body, &operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            crate::shared::ast::DbBody::ObjectFields { fields } => {
                for field in fields {
                    collect_expr_function_type_violations(path, &field.value, violations);
                }
            }
            crate::shared::ast::DbBody::Values { value } => {
                collect_expr_function_type_violations(path, value, violations)
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
                    collect_expr_function_type_violations(path, value, violations)
                }
                crate::shared::ast::DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn collect_db_query_function_type_violations(
    path: &str,
    query: &crate::shared::ast::DbQueryBlock,
    violations: &mut Vec<String>,
) {
    for clause in &query.where_clauses {
        match clause {
            crate::shared::ast::DbWhereClause::Predicate { predicate } => {
                collect_expr_function_type_violations(path, predicate, violations);
            }
            crate::shared::ast::DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                collect_expr_function_type_violations(path, condition, violations);
                collect_expr_function_type_violations(path, predicate, violations);
            }
        }
    }
    if let Some(limit) = &query.limit {
        collect_expr_function_type_violations(path, limit, violations);
    }
    if let Some(offset) = &query.offset {
        collect_expr_function_type_violations(path, offset, violations);
    }
    if let Some(after) = &query.after {
        collect_expr_function_type_violations(path, after, violations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(source: &str) -> Vec<String> {
        let ast = crate::shared::parser::parse_source(source).unwrap();
        let mut violations = Vec::new();
        collect_user_function_type_violations("test.skiff", &ast, &mut violations);
        violations
    }

    #[test]
    fn collects_type_field_and_alias_function_types() {
        let violations = collect(
            r#"
                type HandlerBox {
                    handler: fn(item: string) -> string
                }

                type Callback = fn() -> string
            "#,
        );

        assert_eq!(
            violations,
            vec![
                "test.skiff: callback function type fn(item: string) -> string is only allowed in standard_library/platform native API metadata",
                "test.skiff: callback function type fn() -> string is only allowed in standard_library/platform native API metadata",
            ]
        );
    }

    #[test]
    fn collects_function_param_and_return_function_types() {
        let violations = collect(
            r#"
                function run(callback: fn(item: string) -> string) -> fn(done: bool) -> void {
                    return callback
                }
            "#,
        );

        assert_eq!(
            violations,
            vec![
                "test.skiff: callback function type fn(item: string) -> string is only allowed in standard_library/platform native API metadata",
                "test.skiff: callback function type fn(done: bool) -> void is only allowed in standard_library/platform native API metadata",
            ]
        );
    }

    #[test]
    fn collects_local_annotation_and_generic_type_arg_function_types() {
        let violations = collect(
            r#"
                function run(factory: Factory) -> void {
                    let callback: fn(item: string) -> string = factory
                    factory<fn(value: string) -> string>()
                }
            "#,
        );

        assert_eq!(
            violations,
            vec![
                "test.skiff: callback function type fn(item: string) -> string is only allowed in standard_library/platform native API metadata",
                "test.skiff: callback function type fn(value: string) -> string is only allowed in standard_library/platform native API metadata",
            ]
        );
    }
}
