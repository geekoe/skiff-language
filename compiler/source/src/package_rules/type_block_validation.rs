use crate::shared::ast::{Block, Stmt};

use super::*;

pub(super) fn collect_package_block_std_type_violations(
    path: &str,
    block: &Block,
    imported_std_roots: &BTreeSet<&str>,
    dependency_roots: &BTreeSet<&str>,
    package_type_names: &BTreeSet<String>,
    violations: &mut Vec<String>,
) {
    for stmt in &block.statements {
        match stmt {
            Stmt::Let { ty, value, .. } => {
                if let Some(ty) = ty {
                    collect_package_std_type_name_violations(
                        path,
                        &ty.name,
                        imported_std_roots,
                        dependency_roots,
                        package_type_names,
                        violations,
                    );
                }
                collect_package_expr_std_type_violations(
                    path,
                    value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            Stmt::Assign { target, value } => {
                collect_package_expr_std_type_violations(
                    path,
                    target,
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
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                collect_package_expr_std_type_violations(
                    path,
                    condition,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
                collect_package_block_std_type_violations(
                    path,
                    then_block,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
                if let Some(else_block) = else_block {
                    collect_package_block_std_type_violations(
                        path,
                        else_block,
                        imported_std_roots,
                        dependency_roots,
                        package_type_names,
                        violations,
                    );
                }
            }
            Stmt::For { iterable, body, .. } => {
                collect_package_expr_std_type_violations(
                    path,
                    iterable,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
                collect_package_block_std_type_violations(
                    path,
                    body,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            Stmt::Match { value, arms } => {
                collect_package_expr_std_type_violations(
                    path,
                    value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
                for arm in arms {
                    collect_package_block_std_type_violations(
                        path,
                        &arm.body,
                        imported_std_roots,
                        dependency_roots,
                        package_type_names,
                        violations,
                    );
                }
            }
            Stmt::DbTransaction { body } => {
                collect_package_block_std_type_violations(
                    path,
                    body,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            Stmt::Assert { condition, .. } => {
                collect_package_expr_std_type_violations(
                    path,
                    condition,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            Stmt::Spawn { call: value } | Stmt::Emit(value) | Stmt::Expr(value) => {
                collect_package_expr_std_type_violations(
                    path,
                    value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
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
            Stmt::Throw { value } => {
                collect_package_expr_std_type_violations(
                    path,
                    value,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            Stmt::Rethrow { exception } => {
                collect_package_expr_std_type_violations(
                    path,
                    exception,
                    imported_std_roots,
                    dependency_roots,
                    package_type_names,
                    violations,
                );
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }
}
