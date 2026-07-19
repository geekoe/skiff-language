use crate::shared::ast::Expr;

use super::type_index::{PackageDbTypeIndex, PackageDbTypeRecord};

pub(super) fn validate_record_field_path(
    context: &str,
    type_name: &str,
    field_path: &[String],
    record: &PackageDbTypeRecord<'_>,
    type_index: &PackageDbTypeIndex<'_>,
    violations: &mut Vec<String>,
) {
    if field_path.is_empty() {
        violations.push(format!("{context} for {type_name} cannot be empty"));
        return;
    }
    let mut current = record;
    for (index, segment) in field_path.iter().enumerate() {
        let Some(field) = current.fields.iter().find(|field| field.name == *segment) else {
            violations.push(format!(
                "{context} {} on {} references unknown field {}",
                field_path.join("."),
                type_name,
                segment
            ));
            return;
        };
        if index == field_path.len() - 1 {
            return;
        }
        let Some(next_record) = type_index.resolve_from_module(current.module_path, &field.ty.name)
        else {
            violations.push(format!(
                "{context} {} on {} cannot traverse non-record field {}",
                field_path.join("."),
                type_name,
                segment
            ));
            return;
        };
        current = next_record;
    }
}

pub(super) fn collect_db_index_where_field_paths(expr: &Expr, visit: &mut impl FnMut(Vec<String>)) {
    if let Some(path) = expr_field_path(expr) {
        visit(path);
        return;
    }
    match expr {
        Expr::Binary { left, right, .. } => {
            collect_db_index_where_field_paths(left, visit);
            collect_db_index_where_field_paths(right, visit);
        }
        Expr::Unary { expr, .. } => collect_db_index_where_field_paths(expr, visit),
        Expr::Call { callee, args } => {
            collect_db_index_where_field_paths(callee, visit);
            for arg in args {
                collect_db_index_where_field_paths(arg, visit);
            }
        }
        Expr::Generic { callee, .. } => collect_db_index_where_field_paths(callee, visit),
        Expr::InterfaceBox { value, .. } => collect_db_index_where_field_paths(value, visit),
        Expr::Record { fields, .. } => {
            for (_, value) in fields {
                collect_db_index_where_field_paths(value, visit);
            }
        }
        Expr::ObjectLiteral { entries } => {
            for entry in entries {
                collect_db_index_where_field_paths(&entry.value, visit);
            }
        }
        Expr::Patch { operations, .. } => {
            for operation in operations {
                match operation {
                    crate::shared::ast::PatchOperation::Set { value, .. }
                    | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                        collect_db_index_where_field_paths(value, visit);
                    }
                }
            }
        }
        Expr::Throw { value } => collect_db_index_where_field_paths(value, visit),
        Expr::Rethrow { exception } => collect_db_index_where_field_paths(exception, visit),
        Expr::Catch { try_expr, .. } => collect_db_index_where_field_paths(try_expr, visit),
        Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Field { .. } => {}
    }
}

fn expr_field_path(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Identifier(name) => Some(vec![name.clone()]),
        Expr::Field { object, field } => {
            let mut path = expr_field_path(object)?;
            path.push(field.clone());
            Some(path)
        }
        _ => None,
    }
}
