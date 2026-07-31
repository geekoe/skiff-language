use std::collections::BTreeMap;

use skiff_compiler_source::{
    ExpressionKey, MaterializedObjectField, ObjectMaterializationKind, ResolvedTypeRef,
    TargetTypedObjectMaterialization,
};
use skiff_syntax::error::Result;

use crate::file_ir::{LiteralIr, TypeRefIr};

use super::{materialization_error, FunctionLowerer};

impl FunctionLowerer<'_> {
    pub(super) fn validate_object_materialization_fact(
        &self,
        object_key: &ExpressionKey,
        expected_target: Option<&TypeRefIr>,
        materialization: &TargetTypedObjectMaterialization,
        expression_fields: &BTreeMap<String, TypeRefIr>,
    ) -> Result<BTreeMap<String, MaterializedObjectField>> {
        // This boundary only compares source-owned facts and canonical identities.
        // Assignability and union-branch selection remain exclusively in source typing.
        if expected_target.is_some_and(|expected_target| {
            !self.same_canonical_type(expected_target, &materialization.resolved_target.ir)
        }) {
            return Err(materialization_error(
                object_key,
                format!(
                    "resolved target {:?} does not match the current expected target {:?}",
                    materialization.resolved_target.ir,
                    expected_target.expect("checked as Some")
                ),
            ));
        }

        let canonical_fields =
            self.canonical_materialization_fields(object_key, materialization, expression_fields)?;
        let fact_field_names = materialization
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        let canonical_field_names = canonical_fields
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if fact_field_names != canonical_field_names {
            return Err(materialization_error(
                object_key,
                format!(
                    "fact fields {fact_field_names:?} do not match canonical fields {canonical_field_names:?}"
                ),
            ));
        }

        let mut by_name = BTreeMap::new();
        for field in &materialization.fields {
            let canonical_type = canonical_fields
                .get(&field.name)
                .expect("field names were checked above");
            if !self.same_canonical_type(&field.ty.ir, canonical_type) {
                return Err(materialization_error(
                    object_key,
                    format!(
                        "field `{}` fact type {:?} does not match canonical target field type {canonical_type:?}",
                        field.name, field.ty.ir
                    ),
                ));
            }
            if by_name.insert(field.name.clone(), field.clone()).is_some() {
                return Err(materialization_error(
                    object_key,
                    format!("contains duplicate canonical field `{}`", field.name),
                ));
            }
        }
        Ok(by_name)
    }

    fn canonical_materialization_fields(
        &self,
        object_key: &ExpressionKey,
        materialization: &TargetTypedObjectMaterialization,
        expression_fields: &BTreeMap<String, TypeRefIr>,
    ) -> Result<BTreeMap<String, TypeRefIr>> {
        let target = non_nullable_materialization_target(&materialization.resolved_target.ir);
        match &materialization.kind {
            ObjectMaterializationKind::Record { construct_target } => {
                if !self.same_canonical_type(&target, &construct_target.ir) {
                    return Err(materialization_error(
                        object_key,
                        format!(
                            "record construct target {:?} does not match resolved target {:?}",
                            construct_target.ir, materialization.resolved_target.ir
                        ),
                    ));
                }
                self.record_shape_fields(object_key, construct_target, "record construct target")
            }
            ObjectMaterializationKind::DiscriminatedUnionBranch { branch } => {
                let target = ResolvedTypeRef::with_text(
                    target,
                    materialization.resolved_target.to_string(),
                );
                let target_shape = self
                    .type_resolution
                    .type_shape_ir(&target, &self.type_resolution_context())
                    .unwrap_or_else(|| target.ir.clone());
                let branch_identity_matches = match &target_shape {
                    TypeRefIr::Union { .. } => {
                        union_contains_branch(&target_shape, &branch.ir, |left, right| {
                            self.same_canonical_type(left, right)
                        })
                    }
                    // Generic prelude unions can remain opaque TypeRefIr leaves at
                    // this boundary. Do not parse or re-infer them in lowering: the
                    // target is anchored above and the selected branch is validated
                    // as an exact record shape below.
                    opaque if self.same_canonical_type(opaque, &target.ir) => {
                        map_materialization_value_type(opaque).is_none()
                    }
                    _ => false,
                };
                if !branch_identity_matches {
                    return Err(materialization_error(
                        object_key,
                        format!(
                            "union branch {:?} is not a canonical branch of resolved target {:?} with shape {target_shape:?}",
                            branch.ir, materialization.resolved_target.ir
                        ),
                    ));
                }
                self.record_shape_fields(object_key, branch, "discriminated union branch")
            }
            ObjectMaterializationKind::Map => {
                let value_type = map_materialization_value_type(&target).ok_or_else(|| {
                    materialization_error(
                        object_key,
                        format!(
                            "Map materialization kind is inconsistent with resolved target {:?}",
                            materialization.resolved_target.ir
                        ),
                    )
                })?;
                Ok(expression_fields
                    .keys()
                    .map(|name| (name.clone(), value_type.clone()))
                    .collect())
            }
        }
    }

    fn record_shape_fields(
        &self,
        object_key: &ExpressionKey,
        target: &ResolvedTypeRef,
        target_label: &str,
    ) -> Result<BTreeMap<String, TypeRefIr>> {
        let shape = self
            .type_resolution
            .type_shape_ir(target, &self.type_resolution_context())
            .unwrap_or_else(|| target.ir.clone());
        let TypeRefIr::Record { fields } = shape else {
            return Err(materialization_error(
                object_key,
                format!(
                    "{target_label} {:?} has no canonical record shape",
                    target.ir
                ),
            ));
        };
        Ok(fields)
    }

    pub(super) fn same_canonical_type(&self, left: &TypeRefIr, right: &TypeRefIr) -> bool {
        self.type_resolution
            .canonicalize_type_ref_for_module(self.module_path, left)
            == self
                .type_resolution
                .canonicalize_type_ref_for_module(self.module_path, right)
    }
}

fn non_nullable_materialization_target(target: &TypeRefIr) -> TypeRefIr {
    match target {
        TypeRefIr::Nullable { inner } => inner.as_ref().clone(),
        TypeRefIr::Union { items } => {
            let non_null = items
                .iter()
                .filter(|item| !type_is_null(item))
                .collect::<Vec<_>>();
            match non_null.as_slice() {
                [only] => (*only).clone(),
                _ => target.clone(),
            }
        }
        _ => target.clone(),
    }
}

fn map_materialization_value_type(target: &TypeRefIr) -> Option<TypeRefIr> {
    match target {
        TypeRefIr::Builtin { name, args }
            if name == "Map"
                && matches!(
                    args.as_slice(),
                    [TypeRefIr::Builtin { name, args: key_args }, _]
                        if name == "string" && key_args.is_empty()
                ) =>
        {
            args.get(1).cloned()
        }
        TypeRefIr::Builtin { name, args }
            if args.is_empty() && matches!(name.as_str(), "Json" | "JsonObject") =>
        {
            Some(TypeRefIr::Builtin {
                name: "Json".to_string(),
                args: Vec::new(),
            })
        }
        _ => None,
    }
}

fn union_contains_branch(
    target: &TypeRefIr,
    branch: &TypeRefIr,
    same_type: impl Copy + Fn(&TypeRefIr, &TypeRefIr) -> bool,
) -> bool {
    match target {
        TypeRefIr::Union { items } => items
            .iter()
            .any(|item| union_contains_branch(item, branch, same_type)),
        _ => same_type(target, branch),
    }
}

pub(super) fn type_is_nullable(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Nullable { .. })
        || matches!(ty, TypeRefIr::Union { items } if items.iter().any(type_is_null))
}

fn type_is_null(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if name == "null" && args.is_empty())
        || matches!(
            ty,
            TypeRefIr::Literal {
                value: LiteralIr::Null
            }
        )
}
