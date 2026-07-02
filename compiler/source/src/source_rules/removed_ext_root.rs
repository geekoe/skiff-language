use crate::{
    shared::ast::{
        Block, DbBody, DbChangeOp, DbOperation, DbQueryBlock, DbSelector, DbWhereClause, Expr,
        FunctionDecl, InterfaceOperation, PatchOperation, SourceFile, Stmt, TypeRef,
    },
    shared::ast_utils::source_expressions_reference_dotted_root,
};

pub fn collect_service_removed_ext_root_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    if ast
        .imports
        .iter()
        .any(|import| matches!(import.path.as_slice(), [root, ..] if root == "ext"))
        || source_expressions_reference_dotted_root(ast, "ext")
        || ast_references_ext_type(ast)
    {
        violations.push(format!("{path}: ext root has been removed"));
    }
}

fn ast_references_ext_type(ast: &SourceFile) -> bool {
    ast.types.iter().any(|ty| {
        ty.implements.iter().any(type_ref_uses_ext)
            || ty.alias.as_ref().is_some_and(type_ref_uses_ext)
            || ty.fields.iter().any(|field| type_ref_uses_ext(&field.ty))
    }) || ast
        .aliases
        .iter()
        .any(|alias| type_ref_uses_ext(&alias.target_type))
        || ast.consts.iter().any(|constant| {
            constant.ty.as_ref().is_some_and(type_ref_uses_ext)
                || expr_type_args_use_ext(&constant.value)
        })
        || ast.interfaces.iter().any(|interface| {
            interface
                .operations
                .iter()
                .any(interface_operation_uses_ext_type)
        })
        || ast
            .function_signatures
            .iter()
            .any(interface_operation_uses_ext_type)
        || ast.functions.iter().any(|function| {
            operation_uses_ext_type(function) || block_type_args_use_ext(&function.body)
        })
        || ast.impls.iter().any(|implementation| {
            type_name_uses_ext(&implementation.target)
                || implementation
                    .methods
                    .iter()
                    .any(interface_operation_uses_ext_type)
                || implementation.method_bodies.iter().any(|method| {
                    operation_uses_ext_type(method) || block_type_args_use_ext(&method.body)
                })
        })
}

fn operation_uses_ext_type(operation: &FunctionDecl) -> bool {
    operation
        .params
        .iter()
        .any(|param| type_ref_uses_ext(&param.ty))
        || type_ref_uses_ext(&operation.return_type)
}

fn interface_operation_uses_ext_type(operation: &InterfaceOperation) -> bool {
    operation
        .params
        .iter()
        .any(|param| type_ref_uses_ext(&param.ty))
        || type_ref_uses_ext(&operation.return_type)
}

fn type_ref_uses_ext(ty: &TypeRef) -> bool {
    type_name_uses_ext(&ty.name)
}

fn type_name_uses_ext(name: &str) -> bool {
    name.trim() == "ext" || name.trim().starts_with("ext.")
}

fn block_type_args_use_ext(block: &Block) -> bool {
    block.statements.iter().any(stmt_type_args_use_ext)
}

fn stmt_type_args_use_ext(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { ty, value, .. } => {
            ty.as_ref().is_some_and(type_ref_uses_ext) || expr_type_args_use_ext(value)
        }
        Stmt::Assign { target, value } => {
            expr_type_args_use_ext(target) || expr_type_args_use_ext(value)
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            expr_type_args_use_ext(condition)
                || block_type_args_use_ext(then_block)
                || else_block.as_ref().is_some_and(block_type_args_use_ext)
        }
        Stmt::For { iterable, body, .. } => {
            expr_type_args_use_ext(iterable) || block_type_args_use_ext(body)
        }
        Stmt::Match { value, arms } => {
            expr_type_args_use_ext(value)
                || arms.iter().any(|arm| block_type_args_use_ext(&arm.body))
        }
        Stmt::DbTransaction { body } => block_type_args_use_ext(body),
        Stmt::Assert { condition, .. } => expr_type_args_use_ext(condition),
        Stmt::Emit(value) | Stmt::Expr(value) | Stmt::Throw { value } => {
            expr_type_args_use_ext(value)
        }
        Stmt::Return(value) => value.as_ref().is_some_and(expr_type_args_use_ext),
        Stmt::Rethrow { exception } => expr_type_args_use_ext(exception),
        Stmt::Spawn { call } => expr_type_args_use_ext(call),
        Stmt::Break | Stmt::Continue => false,
    }
}

fn expr_type_args_use_ext(expr: &Expr) -> bool {
    match expr {
        Expr::Generic { callee, type_args } => {
            type_args.iter().any(type_ref_uses_ext) || expr_type_args_use_ext(callee)
        }
        Expr::InterfaceBox { value, interface } => {
            type_ref_uses_ext(interface) || expr_type_args_use_ext(value)
        }
        Expr::Record {
            type_name,
            type_args,
            fields,
        } => {
            type_name_uses_ext(type_name)
                || type_args.iter().any(type_ref_uses_ext)
                || fields
                    .iter()
                    .any(|(_, value)| expr_type_args_use_ext(value))
        }
        Expr::Binary { left, right, .. } => {
            expr_type_args_use_ext(left) || expr_type_args_use_ext(right)
        }
        Expr::Unary { expr, .. } | Expr::Field { object: expr, .. } => expr_type_args_use_ext(expr),
        Expr::Call { callee, args } => {
            expr_type_args_use_ext(callee) || args.iter().any(expr_type_args_use_ext)
        }
        Expr::ObjectLiteral { entries } => entries
            .iter()
            .any(|entry| expr_type_args_use_ext(&entry.value)),
        Expr::Patch { target, operations } => {
            type_ref_uses_ext(target)
                || operations.iter().any(|operation| match operation {
                    PatchOperation::Set { value, .. } | PatchOperation::Inc { value, .. } => {
                        expr_type_args_use_ext(value)
                    }
                })
        }
        Expr::Throw { value } => expr_type_args_use_ext(value),
        Expr::Rethrow { exception } => expr_type_args_use_ext(exception),
        Expr::Catch {
            catch_type,
            try_expr,
        } => type_ref_uses_ext(catch_type) || expr_type_args_use_ext(try_expr),
        Expr::DbOperation(operation) => {
            type_ref_uses_ext(&operation.target) || db_operation_type_args_use_ext(operation)
        }
        Expr::DbQuery(query) => {
            type_ref_uses_ext(&query.target) || db_query_type_args_use_ext(&query.query)
        }
        Expr::DbTransaction(transaction) => block_type_args_use_ext(&transaction.body),
        Expr::DbLeaseClaim(claim) => {
            type_ref_uses_ext(&claim.target)
                || expr_type_args_use_ext(&claim.key)
                || block_type_args_use_ext(&claim.body)
        }
        Expr::DbLeaseRead(read) => {
            type_ref_uses_ext(&read.target) || expr_type_args_use_ext(&read.key)
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => false,
    }
}

fn db_operation_type_args_use_ext(operation: &DbOperation) -> bool {
    operation
        .selector
        .as_ref()
        .is_some_and(|selector| match selector {
            DbSelector::Key { value } => expr_type_args_use_ext(value),
            DbSelector::Query { query } => db_query_type_args_use_ext(query),
        })
        || operation
            .query
            .as_ref()
            .is_some_and(db_query_type_args_use_ext)
        || [&operation.body, &operation.insert_body]
            .into_iter()
            .flatten()
            .any(|body| match body {
                DbBody::ObjectFields { fields } => fields
                    .iter()
                    .any(|field| expr_type_args_use_ext(&field.value)),
                DbBody::Values { value } => expr_type_args_use_ext(value),
            })
        || operation.change.as_ref().is_some_and(|change| {
            change.ops.iter().any(|op| match op {
                DbChangeOp::Set { value, .. }
                | DbChangeOp::Inc { value, .. }
                | DbChangeOp::AddToSet { value, .. }
                | DbChangeOp::Remove { value, .. } => expr_type_args_use_ext(value),
                DbChangeOp::Unset { .. } => false,
            })
        })
}

fn db_query_type_args_use_ext(query: &DbQueryBlock) -> bool {
    query.where_clauses.iter().any(|clause| match clause {
        DbWhereClause::Predicate { predicate } => expr_type_args_use_ext(predicate),
        DbWhereClause::Conditional {
            condition,
            predicate,
        } => expr_type_args_use_ext(condition) || expr_type_args_use_ext(predicate),
    }) || query
        .limit
        .as_ref()
        .is_some_and(|expr| expr_type_args_use_ext(expr))
        || query
            .offset
            .as_ref()
            .is_some_and(|expr| expr_type_args_use_ext(expr))
        || query
            .after
            .as_ref()
            .is_some_and(|expr| expr_type_args_use_ext(expr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::parser::parse_source_with_bodies_tolerant;

    fn collect(source: &str) -> Vec<String> {
        let ast = parse_source_with_bodies_tolerant(source).unwrap();
        let mut violations = Vec::new();
        collect_service_removed_ext_root_violations("service/api.skiff", &ast, &mut violations);
        violations
    }

    fn assert_removed_ext_root(source: &str) {
        assert_eq!(
            collect(source),
            vec!["service/api.skiff: ext root has been removed"]
        );
    }

    #[test]
    fn rejects_ext_import_root() {
        assert_removed_ext_root(
            r#"
                import ext

                type User {
                    name: string,
                }
            "#,
        );
    }

    #[test]
    fn rejects_ext_in_type_refs_and_generic_type_args() {
        assert_removed_ext_root(
            r#"
                type Envelope {
                    payload: Array<ext.legacy.Payload>,
                }

                function decode(value: string) -> string {
                    return parse<ext.legacy.Payload>(value)
                }
            "#,
        );
    }

    #[test]
    fn rejects_ext_expression_root() {
        assert_removed_ext_root(
            r#"
                function read() -> string {
                    return ext.legacy.read()
                }
            "#,
        );
    }
}
