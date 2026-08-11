use std::collections::BTreeMap;

use skiff_compiler_source::{ExpressionKey, ObjectFieldValueSource, ObjectMaterializationKind};
use skiff_syntax::{
    ast::ObjectLiteralEntry,
    error::{CompileError, Result},
};

use crate::file_ir::{ExprIr, ExprRefIr, LiteralIr, TypeRefIr};

use super::FunctionLowerer;
use fact_validation::type_is_nullable;

mod fact_validation;

impl FunctionLowerer<'_> {
    pub(super) fn lower_target_typed_object_literal(
        &mut self,
        expression_key: Option<&ExpressionKey>,
        entries: &[ObjectLiteralEntry],
        expected_target: Option<&TypeRefIr>,
    ) -> Result<ExprIr> {
        let object_key = expression_key.ok_or_else(|| {
            CompileError::Semantic(
                "target-typed object literal lowering requires an ExpressionKey".to_string(),
            )
        })?;
        let expression_types = self.expression_types.ok_or_else(|| {
            CompileError::Semantic(format!(
                "target-typed object literal lowering requires source-owned materialization facts; missing expression type model for ExpressionKey {object_key:?}"
            ))
        })?;
        let materialization = expression_types
            .object_materialization(object_key)
            .cloned()
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "target-typed object literal lowering requires source-owned materialization fact for ExpressionKey {object_key:?}"
                ))
            })?;
        let expression_fact = expression_types
            .fact(object_key)
            .and_then(|fact| fact.ty.as_ref())
            .ok_or_else(|| {
                materialization_error(
                    object_key,
                    "is missing its current object expression type fact",
                )
            })?;
        let expression_fields = match &expression_fact.ir {
            TypeRefIr::Record { fields } => fields.clone(),
            _ => materialization
                .source_fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.ir.clone()))
                .collect::<BTreeMap<_, _>>(),
        };
        let mut remaining_expression_fields = expression_fields;
        let materialized_by_name = self.validate_object_materialization_fact(
            object_key,
            expected_target,
            &materialization,
            &remaining_expression_fields,
        )?;

        let mut provided_values = BTreeMap::<ExpressionKey, (String, ExprRefIr)>::new();
        for entry in entries {
            let field_name = self.lower_object_literal_key(&entry.key)?;
            let materialized = materialized_by_name.get(&field_name).ok_or_else(|| {
                materialization_error(
                    object_key,
                    format!("has no canonical field `{field_name}` for a provided source field"),
                )
            })?;
            let ObjectFieldValueSource::Provided { expression } = &materialized.source else {
                return Err(materialization_error(
                    object_key,
                    format!("marks provided source field `{field_name}` as SyntheticNull"),
                ));
            };
            let value_key = self.peek_expression_key().ok_or_else(|| {
                materialization_error(
                    object_key,
                    format!("cannot identify provided field `{field_name}`"),
                )
            })?;
            if expression != &value_key {
                return Err(materialization_error(
                    object_key,
                    format!(
                        "field `{field_name}` points to expression {expression:?}, but lowering reached {value_key:?}"
                    ),
                ));
            }
            let expression_field_type = remaining_expression_fields
                .remove(&field_name)
                .ok_or_else(|| {
                    materialization_error(
                        object_key,
                        format!(
                            "current object expression type omits provided field `{field_name}`"
                        ),
                    )
                })?;
            let child_fact = expression_types
                .fact(&value_key)
                .and_then(|fact| fact.ty.as_ref())
                .ok_or_else(|| {
                    materialization_error(
                        object_key,
                        format!(
                            "provided field `{field_name}` is missing child expression type fact {value_key:?}"
                        ),
                    )
                })?;
            let expected_field_type = materialized.ty.ir.clone();
            if !self.same_canonical_type(&expression_field_type, &child_fact.ir)
                && !self.same_canonical_type(&expected_field_type, &child_fact.ir)
            {
                return Err(materialization_error(
                    object_key,
                    format!(
                        "provided field `{field_name}` current object type {expression_field_type:?} does not match child expression fact {:?}",
                        child_fact.ir
                    ),
                ));
            }

            let value = self.lower_expr_with_expected(&entry.value, Some(&expected_field_type))?;
            if provided_values
                .insert(value_key.clone(), (field_name.clone(), value))
                .is_some()
            {
                return Err(materialization_error(
                    object_key,
                    format!("contains duplicate provided expression fact {value_key:?}"),
                ));
            }
        }
        if !remaining_expression_fields.is_empty() {
            return Err(materialization_error(
                object_key,
                format!(
                    "current object expression type contains stale fields [{}]",
                    remaining_expression_fields
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }

        let mut lowered_fields = BTreeMap::new();
        for field in materialization.fields {
            let value = match field.source {
                ObjectFieldValueSource::Provided { expression } => {
                    let Some((provided_name, value)) = provided_values.remove(&expression) else {
                        return Err(materialization_error(
                            object_key,
                            format!(
                                "field `{}` references missing provided expression {expression:?}",
                                field.name
                            ),
                        ));
                    };
                    if provided_name != field.name {
                        return Err(materialization_error(
                            object_key,
                            format!(
                                "field `{}` references source field `{provided_name}` at {expression:?}",
                                field.name
                            ),
                        ));
                    }
                    value
                }
                ObjectFieldValueSource::SyntheticNull => {
                    if !type_is_nullable(&field.ty.ir) {
                        return Err(materialization_error(
                            object_key,
                            format!(
                                "non-nullable canonical field `{}` is marked SyntheticNull",
                                field.name
                            ),
                        ));
                    }
                    self.push_expr(
                        ExprIr::Literal {
                            value: LiteralIr::Null,
                        },
                        field.ty.ir.clone(),
                    )
                }
            };
            if lowered_fields.insert(field.name.clone(), value).is_some() {
                return Err(materialization_error(
                    object_key,
                    format!("contains duplicate canonical field `{}`", field.name),
                ));
            }
        }

        if !provided_values.is_empty() {
            let unconsumed = provided_values
                .values()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(materialization_error(
                object_key,
                format!("leaves provided fields [{unconsumed}] unconsumed"),
            ));
        }

        match materialization.kind {
            ObjectMaterializationKind::Record { construct_target } => Ok(ExprIr::Construct {
                type_ref: construct_target.ir,
                fields: lowered_fields,
            }),
            ObjectMaterializationKind::DiscriminatedUnionBranch { branch } => {
                Ok(ExprIr::Construct {
                    type_ref: match materialization.resolved_target.ir {
                        package_target @ TypeRefIr::PackageSymbol { .. } => package_target,
                        _ => branch.ir,
                    },
                    fields: lowered_fields,
                })
            }
            ObjectMaterializationKind::Map => Ok(ExprIr::MapLiteral {
                entries: lowered_fields,
            }),
        }
    }
}

fn materialization_error(object_key: &ExpressionKey, detail: impl Into<String>) -> CompileError {
    CompileError::Semantic(format!(
        "target-typed object literal materialization fact for ExpressionKey {object_key:?} {}",
        detail.into()
    ))
}
