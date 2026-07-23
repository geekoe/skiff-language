use std::collections::BTreeMap;

use skiff_compiler_source::{ExpressionKey, ObjectFieldValueSource, ObjectMaterializationKind};
use skiff_syntax::{
    ast::ObjectLiteralEntry,
    error::{CompileError, Result},
};

use crate::file_ir::{ExprIr, ExprRefIr, LiteralIr};

use super::FunctionLowerer;

impl FunctionLowerer<'_> {
    pub(super) fn lower_target_typed_object_literal(
        &mut self,
        expression_key: Option<&ExpressionKey>,
        entries: &[ObjectLiteralEntry],
    ) -> Result<ExprIr> {
        let object_key = expression_key.ok_or_else(|| {
            CompileError::Semantic(
                "target-typed object literal lowering requires an ExpressionKey".to_string(),
            )
        })?;
        let materialization = self
            .expression_types
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "target-typed object literal lowering requires source-owned materialization facts; missing expression type model for ExpressionKey {object_key:?}"
                ))
            })?
            .object_materialization(object_key)
            .cloned()
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "target-typed object literal lowering requires source-owned materialization fact for ExpressionKey {object_key:?}"
                ))
            })?;

        let mut provided_values = BTreeMap::<ExpressionKey, (String, ExprRefIr)>::new();
        for entry in entries {
            let field_name = self.lower_object_literal_key(&entry.key)?;
            let value_key = self.peek_expression_key().ok_or_else(|| {
                CompileError::Semantic(format!(
                    "target-typed object literal materialization for ExpressionKey {object_key:?} cannot identify provided field `{field_name}`"
                ))
            })?;
            let value = self.lower_expr(&entry.value)?;
            if provided_values
                .insert(value_key.clone(), (field_name.clone(), value))
                .is_some()
            {
                return Err(CompileError::Semantic(format!(
                    "target-typed object literal materialization for ExpressionKey {object_key:?} contains duplicate provided expression fact {value_key:?}"
                )));
            }
        }

        let mut lowered_fields = BTreeMap::new();
        for field in materialization.fields {
            let value = match field.source {
                ObjectFieldValueSource::Provided { expression } => {
                    let Some((provided_name, value)) = provided_values.remove(&expression) else {
                        return Err(CompileError::Semantic(format!(
                            "target-typed object literal materialization for ExpressionKey {object_key:?} field `{}` references missing provided expression {expression:?}",
                            field.name
                        )));
                    };
                    if provided_name != field.name {
                        return Err(CompileError::Semantic(format!(
                            "target-typed object literal materialization for ExpressionKey {object_key:?} field `{}` references source field `{provided_name}` at {expression:?}",
                            field.name
                        )));
                    }
                    value
                }
                ObjectFieldValueSource::SyntheticNull => self.push_expr(ExprIr::Literal {
                    value: LiteralIr::Null,
                }),
            };
            if lowered_fields.insert(field.name.clone(), value).is_some() {
                return Err(CompileError::Semantic(format!(
                    "target-typed object literal materialization for ExpressionKey {object_key:?} contains duplicate canonical field `{}`",
                    field.name
                )));
            }
        }

        if !provided_values.is_empty() {
            let unconsumed = provided_values
                .values()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(CompileError::Semantic(format!(
                "target-typed object literal materialization for ExpressionKey {object_key:?} leaves provided fields [{unconsumed}] unconsumed"
            )));
        }

        match materialization.kind {
            ObjectMaterializationKind::Record { construct_target } => Ok(ExprIr::Construct {
                type_ref: construct_target.ir,
                fields: lowered_fields,
            }),
            ObjectMaterializationKind::DiscriminatedUnionBranch { branch } => {
                Ok(ExprIr::Construct {
                    type_ref: branch.ir,
                    fields: lowered_fields,
                })
            }
            ObjectMaterializationKind::Map => Ok(ExprIr::MapLiteral {
                entries: lowered_fields,
            }),
        }
    }
}
