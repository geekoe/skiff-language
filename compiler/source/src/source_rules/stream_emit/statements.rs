use std::collections::BTreeMap;

use crate::shared::ast::{Expr, ForBinding, Stmt};

use super::{
    types::{
        collect_emit_expression_call_violations, infer_expr_type, iterable_item_type,
        types_compatible_for_annotation, types_compatible_for_emit,
    },
    validate_emit_usage_in_block,
};

pub(super) fn validate_emit_usage_in_stmt(
    path: &str,
    function_name: &str,
    stream_chunk: Option<&str>,
    stmt: &Stmt,
    function_return_types: &BTreeMap<String, String>,
    env: &mut BTreeMap<String, String>,
    violations: &mut Vec<String>,
) {
    match stmt {
        Stmt::Let {
            name, ty, value, ..
        } => {
            collect_emit_expression_call_violations(path, value, violations);
            let inferred = infer_expr_type(value, env, function_return_types);
            if let (Some(annotation), Some(initializer)) = (ty.as_ref(), inferred.as_ref()) {
                if !types_compatible_for_annotation(initializer, &annotation.name) {
                    violations.push(format!(
                        "{path}: local binding {name} annotation type mismatch in {function_name}: expected {}, found {initializer}",
                        annotation.name
                    ));
                }
            }
            if let Some(bound_type) = inferred {
                env.insert(name.clone(), bound_type);
            }
        }
        Stmt::Assign { target, value } => {
            collect_emit_expression_call_violations(path, target, violations);
            collect_emit_expression_call_violations(path, value, violations);
            if let Expr::Identifier(name) = target {
                if let Some(ty) = infer_expr_type(value, env, function_return_types) {
                    env.insert(name.clone(), ty);
                }
            }
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
        } => {
            collect_emit_expression_call_violations(path, condition, violations);
            let mut then_env = env.clone();
            validate_emit_usage_in_block(
                path,
                function_name,
                stream_chunk,
                then_block,
                function_return_types,
                &mut then_env,
                violations,
            );
            if let Some(else_block) = else_block {
                let mut else_env = env.clone();
                validate_emit_usage_in_block(
                    path,
                    function_name,
                    stream_chunk,
                    else_block,
                    function_return_types,
                    &mut else_env,
                    violations,
                );
            }
        }
        Stmt::For {
            binding,
            iterable,
            body,
        } => {
            collect_emit_expression_call_violations(path, iterable, violations);
            let iterable_type = infer_expr_type(iterable, env, function_return_types);
            let mut body_env = env.clone();
            match binding {
                ForBinding::Item { item } => {
                    if let Some(item_type) = iterable_type.as_deref().and_then(iterable_item_type) {
                        body_env.insert(item.clone(), item_type);
                    }
                }
                ForBinding::Entry { key, value } => {
                    if let Some((key_type, value_type)) = iterable_type
                        .as_deref()
                        .and_then(super::types::map_entry_types)
                    {
                        body_env.insert(key.clone(), key_type.to_string());
                        body_env.insert(value.clone(), value_type.to_string());
                    }
                }
            }
            validate_emit_usage_in_block(
                path,
                function_name,
                stream_chunk,
                body,
                function_return_types,
                &mut body_env,
                violations,
            );
        }
        Stmt::Match { value, arms } => {
            collect_emit_expression_call_violations(path, value, violations);
            for arm in arms {
                let mut arm_env = env.clone();
                validate_emit_usage_in_block(
                    path,
                    function_name,
                    stream_chunk,
                    &arm.body,
                    function_return_types,
                    &mut arm_env,
                    violations,
                );
            }
        }
        Stmt::Assert { condition, .. } => {
            collect_emit_expression_call_violations(path, condition, violations);
        }
        Stmt::DbTransaction { body } => {
            let mut transaction_env = env.clone();
            validate_emit_usage_in_block(
                path,
                function_name,
                stream_chunk,
                body,
                function_return_types,
                &mut transaction_env,
                violations,
            );
        }
        Stmt::Emit(value) => {
            collect_emit_expression_call_violations(path, value, violations);
            let Some(expected) = stream_chunk else {
                violations.push(format!(
                    "{path}: emit can only be used in a Stream<T> producer; function {function_name} returns a non-stream type"
                ));
                return;
            };
            let Some(actual) = infer_expr_type(value, env, function_return_types) else {
                violations.push(format!(
                    "{path}: cannot infer emit chunk type in {function_name}; emit expression must have a known Stream<T> chunk type"
                ));
                return;
            };
            if !types_compatible_for_emit(&actual, expected) {
                violations.push(format!(
                    "{path}: emit chunk type mismatch in {function_name}: expected {expected}, found {actual}"
                ));
            }
        }
        Stmt::Expr(value) => {
            collect_emit_expression_call_violations(path, value, violations);
        }
        Stmt::Spawn { call } => {
            collect_emit_expression_call_violations(path, call, violations);
        }
        Stmt::Return(value) => {
            if let Some(value) = value {
                collect_emit_expression_call_violations(path, value, violations);
            }
        }
        Stmt::Throw { value } => {
            collect_emit_expression_call_violations(path, value, violations);
        }
        Stmt::Rethrow { exception } => {
            collect_emit_expression_call_violations(path, exception, violations);
        }
        Stmt::Break | Stmt::Continue => {}
    }
}
