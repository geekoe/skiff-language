use std::collections::BTreeMap;

use crate::file_ir::{LiteralIr, TypeRefIr};
use skiff_artifact_model::{builtin_receiver_op_spec_by_name, BuiltinReceiverPublicReturnType};
use skiff_compiler_source::{type_text_with_args, ExpressionKey};
use skiff_syntax::{
    ast::{DbBlockMode, Expr, Literal, TypeRef},
    type_expr::TypeExpr,
    type_syntax::generic_parts,
};

use super::callable_return_types::CallableReturnType;
use super::db_lowering::{
    db_lease_read_result_type_ir, db_lease_read_result_type_text, db_operation_result_type_ir,
    db_query_type_ref,
};
use super::function_lowering::{expr_path, FunctionLowerer};
use super::type_lowering::{
    canonical_runtime_receiver_root, lower_type_ref, lower_type_text, type_ref_ir_type_text,
};

fn array_item_type_text(type_text: &str) -> Option<&str> {
    let parts = generic_parts(type_text)?;
    if !matches!(parts.root.trim(), "Array" | "Stream") || parts.args.len() != 1 {
        return None;
    }
    Some(parts.args[0].trim())
}

fn single_for_item_type_text(type_text: &str) -> Option<&str> {
    let parts = generic_parts(type_text)?;
    match parts.root.trim() {
        "Array" | "Stream" if parts.args.len() == 1 => Some(parts.args[0].trim()),
        "Map" if parts.args.len() == 2 => Some(parts.args[0].trim()),
        _ => None,
    }
}

fn array_item_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Native { name, args } = ty else {
        return None;
    };
    if !matches!(name.as_str(), "Array" | "Stream") || args.len() != 1 {
        return None;
    }
    args.first().cloned()
}

fn single_for_item_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    let TypeRefIr::Native { name, args } = ty else {
        return None;
    };
    match name.as_str() {
        "Array" | "Stream" if args.len() == 1 => args.first().cloned(),
        "Map" if args.len() == 2 => args.first().cloned(),
        _ => None,
    }
}

fn type_root_text(type_text: &str) -> &str {
    let type_text = type_text.trim();
    let type_text = type_text.strip_suffix('?').unwrap_or(type_text).trim();
    generic_parts(type_text)
        .map(|parts| parts.root.trim())
        .unwrap_or(type_text)
}

fn map_value_type_text(type_text: &str) -> Option<String> {
    let parts = generic_parts(type_text.trim())?;
    (parts.root.trim() == "Map" && parts.args.len() == 2).then(|| parts.args[1].trim().to_string())
}

fn map_key_type_text(type_text: &str) -> Option<String> {
    let parts = generic_parts(type_text.trim())?;
    (parts.root.trim() == "Map" && parts.args.len() == 2).then(|| parts.args[0].trim().to_string())
}

fn map_entry_type_text(type_text: &str) -> Option<(String, String)> {
    let parts = generic_parts(type_text.trim())?;
    (parts.root.trim() == "Map" && parts.args.len() == 2).then(|| {
        (
            parts.args[0].trim().to_string(),
            parts.args[1].trim().to_string(),
        )
    })
}

fn map_entry_type_ir(ty: &TypeRefIr) -> Option<(TypeRefIr, TypeRefIr)> {
    let TypeRefIr::Native { name, args } = ty else {
        return None;
    };
    (name == "Map" && args.len() == 2).then(|| (args[0].clone(), args[1].clone()))
}

fn receiver_type_root_from_ir(ty: &TypeRefIr) -> Option<String> {
    match ty {
        TypeRefIr::Native { name, .. } => Some(canonical_runtime_receiver_root(name).to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::String { .. },
        } => Some("string".to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Number { .. },
        } => Some("number".to_string()),
        TypeRefIr::Nullable { inner } => receiver_type_root_from_ir(inner),
        _ => None,
    }
}

fn builtin_receiver_call_return_type_for_root(
    root: &str,
    receiver_type: &str,
    method_name: &str,
) -> Option<String> {
    let root = canonical_runtime_receiver_root(root);
    let spec = builtin_receiver_op_spec_by_name(root, method_name)?;
    match spec.public_return_type {
        BuiltinReceiverPublicReturnType::Fixed(name) => Some(name.to_string()),
        BuiltinReceiverPublicReturnType::Receiver => Some(receiver_type.trim().to_string()),
        BuiltinReceiverPublicReturnType::ArrayItem => {
            array_item_type_text(receiver_type).map(str::to_string)
        }
        BuiltinReceiverPublicReturnType::MapValue => map_value_type_text(receiver_type),
        BuiltinReceiverPublicReturnType::MapKeyArray => {
            map_key_type_text(receiver_type).map(|key| format!("Array<{key}>"))
        }
    }
}

impl<'a> FunctionLowerer<'a> {
    fn callable_return_signature(&self, path: &str) -> Option<&CallableReturnType> {
        if !path.contains('.') {
            let module_qualified = format!("{}.{}", self.module_path, path);
            if let Some(signature) = self.callable_return_types.get(&module_qualified) {
                return Some(signature);
            }
        }
        self.callable_return_types.get(path).or_else(|| {
            path.strip_prefix("root.")
                .and_then(|source_path| self.callable_return_types.get(source_path))
        })
    }

    fn callable_return_type_text(&self, path: &str, type_args: &[TypeRef]) -> Option<String> {
        let signature = self.callable_return_signature(path)?;
        let substitutions = signature
            .type_params
            .iter()
            .zip(type_args)
            .map(|(name, ty)| (name.clone(), ty.name.clone()))
            .collect::<BTreeMap<_, _>>();
        let substituted = TypeExpr::parse_lossy(&signature.return_type)
            .map_named_types(|name| {
                substitutions
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.to_string())
            })
            .to_type_string();
        if signature.module_path == self.module_path {
            return Some(substituted);
        }
        Some(
            TypeExpr::parse_lossy(&substituted)
                .map_named_types(|name| {
                    if signature.local_type_names.contains(name) {
                        format!("root.{}.{}", signature.module_path, name)
                    } else {
                        name.to_string()
                    }
                })
                .to_type_string(),
        )
    }

    fn builtin_receiver_call_return_type(
        &self,
        receiver_type: &str,
        method_name: &str,
    ) -> Option<String> {
        let root = lower_type_text(
            receiver_type,
            self.type_indices,
            self.local_db_objects,
            self.publication_db_metadata,
            self.package_aliases,
            self.external_type_symbols,
            self.source_alias_targets,
            self.value_type_context(),
        )
        .ok()
        .and_then(|ty| receiver_type_root_from_ir(&ty))
        .unwrap_or_else(|| type_root_text(receiver_type).to_string());
        builtin_receiver_call_return_type_for_root(&root, receiver_type, method_name)
    }

    fn current_call_receiver_type_text(&self, callee: &Expr) -> Option<String> {
        let offset = 1 + receiver_object_offset_in_callee(callee)?;
        self.expression_type_at_offset(offset)
            .map(|(source_text, _)| source_text)
    }

    pub(super) fn expression_type_at_offset(&self, offset: u32) -> Option<(String, TypeRefIr)> {
        let expression_types = self.expression_types?;
        let owner = self.expression_owner.as_ref()?;
        let preorder_index = self.next_expression_index.checked_add(offset)?;
        let key = ExpressionKey::new(self.module_path.to_string(), owner.clone(), preorder_index);
        let ty = expression_types.fact(&key)?.ty.as_ref()?;
        Some((ty.source_text.clone(), ty.ir.clone()))
    }

    pub(super) fn next_expression_type(&self) -> Option<(String, TypeRefIr)> {
        self.expression_type_at_offset(0)
    }

    pub(super) fn next_expression_type_text(&self) -> Option<String> {
        self.next_expression_type()
            .map(|(source_text, _)| source_text)
    }

    pub(super) fn next_expression_type_ir(&self) -> Option<TypeRefIr> {
        self.next_expression_type().map(|(_, ty)| ty)
    }

    pub(super) fn next_expression_array_item_type(&self) -> Option<(String, TypeRefIr)> {
        let (source_text, ty) = self.next_expression_type()?;
        let item_text = array_item_type_text(&source_text)?.to_string();
        let item_ty = array_item_type_ir(&ty)?;
        Some((item_text, item_ty))
    }

    pub(super) fn next_expression_single_for_item_type(&self) -> Option<(String, TypeRefIr)> {
        let (source_text, ty) = self.next_expression_type()?;
        let item_text = single_for_item_type_text(&source_text)?.to_string();
        let item_ty = single_for_item_type_ir(&ty)?;
        Some((item_text, item_ty))
    }

    pub(super) fn next_expression_map_entry_type_text(&self) -> Option<(String, String)> {
        let (source_text, ty) = self.next_expression_type()?;
        let (key_text, value_text) = map_entry_type_text(&source_text)?;
        let (_key_ty, _value_ty) = map_entry_type_ir(&ty)?;
        Some((key_text, value_text))
    }

    pub(super) fn infer_array_item_type_text(&self, expr: &Expr) -> Option<String> {
        self.infer_expr_type_text(expr)
            .and_then(|type_text| array_item_type_text(&type_text).map(str::to_string))
    }

    pub(super) fn infer_single_for_item_type(&self, expr: &Expr) -> Option<(String, TypeRefIr)> {
        let type_text = self.infer_expr_type_text(expr)?;
        let item_text = single_for_item_type_text(&type_text)?.to_string();
        let item_ty = single_for_item_type_ir(&self.infer_expr_type_ir(expr)?)?;
        Some((item_text, item_ty))
    }

    pub(super) fn infer_map_entry_type_text(&self, expr: &Expr) -> Option<(String, String)> {
        self.infer_expr_type_text(expr)
            .and_then(|type_text| map_entry_type_text(&type_text))
    }

    pub(super) fn infer_expr_type_text(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Identifier(name) => self
                .bindings
                .get(name)
                .and_then(|binding| binding.type_text.clone()),
            Expr::RemotePublicInstanceSource(_) => None,
            Expr::Field { object, field } => {
                let object_ty = self.infer_expr_type_ir(object)?;
                self.field_type_for_receiver_type(&object_ty, field)
                    .map(|ty| type_ref_ir_type_text(&ty))
            }
            Expr::Record {
                type_name,
                type_args,
                ..
            } => Some(type_text_with_args(type_name, type_args)),
            Expr::Call { callee, .. } => {
                let raw_callee = callee.as_ref();
                let (callee, type_args) = match raw_callee {
                    Expr::Generic { callee, type_args } => (callee.as_ref(), type_args.as_slice()),
                    _ => (raw_callee, &[][..]),
                };
                if let Expr::Field { object, field } = callee {
                    if let Some(receiver_type) = self
                        .current_call_receiver_type_text(raw_callee)
                        .or_else(|| self.infer_expr_type_text(object))
                    {
                        if let Some(return_type) =
                            self.builtin_receiver_call_return_type(&receiver_type, field)
                        {
                            return Some(return_type);
                        }
                    }
                }
                let path = expr_path(callee)?;
                if let Some(return_type) = self.callable_return_type_text(&path, type_args) {
                    return Some(return_type);
                }
                match path.as_str() {
                    "Array.empty" => type_args.first().map(|ty| format!("Array<{}>", ty.name)),
                    "Map.empty" => {
                        let key = type_args.first()?;
                        let value = type_args.get(1)?;
                        Some(format!("Map<{}, {}>", key.name, value.name))
                    }
                    "db.get" | "db.require" | "db.create" | "db.append" | "db.upsert" => {
                        type_args.first().map(|ty| ty.name.clone())
                    }
                    "db.findMany" | "db.createMany" | "db.create_many" | "db.appendMany"
                    | "db.append_many" => type_args.first().map(|ty| format!("Array<{}>", ty.name)),
                    "db.exists" => Some("bool".to_string()),
                    "db.count" => Some("number".to_string()),
                    _ => None,
                }
            }
            Expr::Generic { callee, .. } => self.infer_expr_type_text(callee),
            Expr::InterfaceBox { interface, .. } => Some(format!("any {}", interface.name)),
            Expr::Literal(Literal::String(_)) => Some("string".to_string()),
            Expr::Literal(Literal::Number(_)) => Some("number".to_string()),
            Expr::Literal(Literal::Bool(_)) => Some("bool".to_string()),
            Expr::Literal(Literal::Null) => Some("null".to_string()),
            Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::ObjectLiteral { .. }
            | Expr::Patch { .. }
            | Expr::Throw { .. }
            | Expr::Rethrow { .. }
            | Expr::Catch { .. }
            | Expr::DbQuery(_) => None,
            Expr::DbOperation(operation) => self.db_operation_result_type_text(operation),
            Expr::DbTransaction(transaction) => match transaction.mode {
                DbBlockMode::Effect => Some("null".to_string()),
                DbBlockMode::Value => None,
            },
            Expr::DbLeaseClaim(_) => Some("bool".to_string()),
            Expr::DbLeaseRead(_) => Some(db_lease_read_result_type_text()),
        }
    }

    pub(super) fn infer_expr_type_ir(&self, expr: &Expr) -> Option<TypeRefIr> {
        match expr {
            Expr::DbOperation(operation) => {
                let db_metadata = self
                    .resolve_db_operation_target(&operation.target.name)
                    .ok()?;
                let target = lower_type_ref(
                    &operation.target,
                    self.type_indices,
                    self.local_db_objects,
                    self.publication_db_metadata,
                    self.package_aliases,
                    self.external_type_symbols,
                    self.source_alias_targets,
                    self.db_target_type_context(),
                )
                .ok()?;
                let projection = operation
                    .projection
                    .as_ref()
                    .map(|projection| self.lower_db_projection(db_metadata, projection))
                    .transpose()
                    .ok()?;
                Some(
                    db_operation_result_type_ir(
                        operation,
                        target,
                        projection.as_ref(),
                        Some(db_metadata),
                    )
                    .ok()?,
                )
            }
            Expr::DbQuery(query) => {
                let target = lower_type_ref(
                    &query.target,
                    self.type_indices,
                    self.local_db_objects,
                    self.publication_db_metadata,
                    self.package_aliases,
                    self.external_type_symbols,
                    self.source_alias_targets,
                    self.db_target_type_context(),
                )
                .ok()?;
                Some(db_query_type_ref(target))
            }
            Expr::DbLeaseClaim(_) => Some(TypeRefIr::native("bool")),
            Expr::DbLeaseRead(_) => Some(db_lease_read_result_type_ir()),
            _ => self.infer_expr_type_text(expr).and_then(|type_text| {
                lower_type_text(
                    &type_text,
                    self.type_indices,
                    self.local_db_objects,
                    self.publication_db_metadata,
                    self.package_aliases,
                    self.external_type_symbols,
                    self.source_alias_targets,
                    self.value_type_context(),
                )
                .ok()
            }),
        }
    }
}

fn receiver_object_offset_in_callee(expr: &Expr) -> Option<u32> {
    match expr {
        Expr::Field { .. } => Some(1),
        Expr::Generic { callee, .. } => receiver_object_offset_in_callee(callee).map(|offset| {
            offset
                .checked_add(1)
                .expect("receiver expression preorder offset should fit in u32")
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_receiver_return_types_come_from_artifact_table_for_date_integer_ops() {
        for method in ["toEpochMilliseconds", "diffMilliseconds", "compare"] {
            assert_eq!(
                builtin_receiver_call_return_type_for_root("Date", "Date", method),
                Some("integer".to_string()),
                "Date.{method} should infer integer from artifact-model table"
            );
        }
        assert_eq!(
            builtin_receiver_call_return_type_for_root(
                "Duration",
                "std.time.Duration",
                "toMilliseconds"
            ),
            Some("integer".to_string())
        );
    }
}
