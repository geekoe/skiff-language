use crate::shared::ast::Expr;

use super::*;

pub(super) fn collect_package_expr_std_type_violations(
    path: &str,
    expr: &Expr,
    imported_std_roots: &BTreeSet<&str>,
    dependency_roots: &BTreeSet<&str>,
    package_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    match expr {
        Expr::Generic { callee, type_args } => {
            for type_arg in type_args {
                collect_package_std_type_name_violations(
                    path,
                    &type_arg.name,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            collect_package_expr_std_type_violations(
                path,
                callee,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::InterfaceBox { value, interface } => {
            collect_package_std_type_name_violations(
                path,
                &interface.name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_expr_std_type_violations(
                path,
                value,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::Record {
            type_name,
            type_args,
            fields,
        } => {
            collect_package_std_type_name_violations(
                path,
                type_name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            for type_arg in type_args {
                collect_package_std_type_name_violations(
                    path,
                    &type_arg.name,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            for (_, value) in fields {
                collect_package_expr_std_type_violations(
                    path,
                    value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_package_expr_std_type_violations(
                path,
                left,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_expr_std_type_violations(
                path,
                right,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::Unary { expr, .. } | Expr::Field { object: expr, .. } => {
            collect_package_expr_std_type_violations(
                path,
                expr,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::Call { callee, args } => {
            collect_package_expr_std_type_violations(
                path,
                callee,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            for arg in args {
                collect_package_expr_std_type_violations(
                    path,
                    arg,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_package_expr_std_type_violations(
                    path,
                    &entry.value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
        }
        Expr::Patch { target, operations } => {
            collect_package_std_type_name_violations(
                path,
                &target.name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            for operation in operations {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_package_expr_std_type_violations(
                            path,
                            value,
                            imported_std_roots,
                            dependency_roots,
                            package_type_names,
                            violations,
                        );
                    }
                }
            }
        }
        Expr::Throw { value } => collect_package_expr_std_type_violations(
            path,
            value,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        ),
        Expr::Rethrow { exception } => collect_package_expr_std_type_violations(
            path,
            exception,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        ),
        Expr::Catch {
            catch_type,
            try_expr,
        } => {
            collect_package_std_type_name_violations(
                path,
                &catch_type.name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_expr_std_type_violations(
                path,
                try_expr,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::DbOperation(operation) => {
            collect_package_std_type_name_violations(
                path,
                &operation.target.name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_db_operation_std_type_violations(
                path,
                operation,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::DbQuery(query) => {
            collect_package_std_type_name_violations(
                path,
                &query.target.name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_db_query_std_type_violations(
                path,
                &query.query,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::DbTransaction(transaction) => {
            collect_package_block_std_type_violations(
                path,
                &transaction.body,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::DbLeaseClaim(claim) => {
            collect_package_std_type_name_violations(
                path,
                &claim.target.name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_expr_std_type_violations(
                path,
                &claim.key,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_block_std_type_violations(
                path,
                &claim.body,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::DbLeaseRead(read) => {
            collect_package_std_type_name_violations(
                path,
                &read.target.name,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
            collect_package_expr_std_type_violations(
                path,
                &read.key,
                imported_std_roots,
                dependency_roots,
                package_type_names,
                violations,
            );
        }
        Expr::Literal(_) | Expr::Identifier(_) | Expr::RemotePublicInstanceSource(_) => {}
    }
}

fn collect_package_db_operation_std_type_violations(
    path: &str,
    operation: &crate::shared::ast::DbOperation,
    imported_std_roots: &BTreeSet<&str>,
    dependency_roots: &BTreeSet<&str>,
    package_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    if let Some(selector) = &operation.selector {
        match selector {
            crate::shared::ast::DbSelector::Key { value } => {
                collect_package_expr_std_type_violations(
                    path,
                    value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                )
            }
            crate::shared::ast::DbSelector::Query { query } => {
                collect_package_db_query_std_type_violations(
                    path,
                    query,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                )
            }
        }
    }
    if let Some(query) = &operation.query {
        collect_package_db_query_std_type_violations(
            path,
            query,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        );
    }
    for body in [&operation.body, &operation.insert_body]
        .into_iter()
        .flatten()
    {
        match body {
            crate::shared::ast::DbBody::ObjectFields { fields } => {
                for field in fields {
                    collect_package_expr_std_type_violations(
                        path,
                        &field.value,
                        imported_std_roots,
                        dependency_roots,
                        package_type_names,
                        violations,
                    );
                }
            }
            crate::shared::ast::DbBody::Values { value } => {
                collect_package_expr_std_type_violations(
                    path,
                    value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                )
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
                    collect_package_expr_std_type_violations(
                        path,
                        value,
                        imported_std_roots,
                        dependency_roots,
                        package_type_names,
                        violations,
                    );
                }
                crate::shared::ast::DbChangeOp::Unset { .. } => {}
            }
        }
    }
}

fn collect_package_db_query_std_type_violations(
    path: &str,
    query: &crate::shared::ast::DbQueryBlock,
    imported_std_roots: &BTreeSet<&str>,
    dependency_roots: &BTreeSet<&str>,
    package_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    for clause in &query.where_clauses {
        match clause {
            crate::shared::ast::DbWhereClause::Predicate { predicate } => {
                collect_package_expr_std_type_violations(
                    path,
                    predicate,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            crate::shared::ast::DbWhereClause::Conditional {
                condition,
                predicate,
            } => {
                collect_package_expr_std_type_violations(
                    path,
                    condition,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
                collect_package_expr_std_type_violations(
                    path,
                    predicate,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
        }
    }
    if let Some(limit) = &query.limit {
        collect_package_expr_std_type_violations(
            path,
            limit,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        );
    }
    if let Some(offset) = &query.offset {
        collect_package_expr_std_type_violations(
            path,
            offset,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        );
    }
    if let Some(after) = &query.after {
        collect_package_expr_std_type_violations(
            path,
            after,
            imported_std_roots,
            dependency_roots,
            package_type_names,
            violations,
        );
    }
}
