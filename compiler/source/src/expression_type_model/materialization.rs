use super::*;

impl<'a> OwnerChecker<'a> {
    pub(super) fn check_record_expr(
        &mut self,
        key: &ExpressionKey,
        type_name: &str,
        type_args: &[TypeRef],
        fields: &[(String, Expr)],
    ) -> Option<ResolvedTypeRef> {
        let mut field_types = Vec::new();
        let mut provided_field_keys = Vec::new();
        for (name, value) in fields {
            let value_key = self.peek_key();
            provided_field_keys.push((name.clone(), value_key));
            let value_ty = self.check_expr(value);
            field_types.push(value_ty);
        }
        self.validate_constructor(
            key,
            type_name,
            type_args,
            fields,
            &field_types,
            &provided_field_keys,
        )
    }

    pub(super) fn check_object_literal_expr(
        &mut self,
        key: &ExpressionKey,
        entries: &[crate::shared::ast::ObjectLiteralEntry],
    ) -> Option<ResolvedTypeRef> {
        let source_fact = self.expression_sources.fact(key);
        let mut fields = BTreeMap::new();
        let mut source_fields = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let value_key = self.peek_key();
            let actual = self.check_expr(&entry.value);
            let Some(name) = object_literal_key_text(&entry.key) else {
                continue;
            };
            if let Some(actual) = &actual {
                fields.insert(name.clone(), actual.clone());
            }
            source_fields.push(ObjectLiteralSourceField {
                name,
                expression: value_key,
                actual,
                value_span: record_field_value_source_span(source_fact, index),
            });
        }
        self.outputs.object_materialization.sources.insert(
            key.clone(),
            ObjectLiteralSource {
                span: source_fact
                    .map(|fact| fact.span)
                    .unwrap_or_else(SourceSpan::synthetic),
                fields: source_fields,
            },
        );
        Some(ResolvedTypeRef::with_text(
            TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.ir.clone()))
                    .collect(),
            },
            "{}".to_string(),
        ))
    }

    pub(super) fn validate_constructor(
        &mut self,
        key: &ExpressionKey,
        type_name: &str,
        type_args: &[TypeRef],
        fields: &[(String, Expr)],
        field_types: &[Option<ResolvedTypeRef>],
        provided_field_keys: &[(String, ExpressionKey)],
    ) -> Option<ResolvedTypeRef> {
        let source_fact = self.expression_sources.fact(key);
        let target = match self.type_resolution.resolve_constructor_target(
            type_name,
            type_args,
            &self.type_context,
        ) {
            Ok(target) => target,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: constructor target `{type_name}` failed to resolve at {}: {error}",
                    self.module_path,
                    source_fact
                        .map(|fact| span_label(fact.span))
                        .unwrap_or_else(|| "unknown span".to_string())
                ));
                return None;
            }
        };

        let mut provided = BTreeMap::<String, ExpressionKey>::new();
        let mut duplicates = BTreeSet::new();
        let mut provided_fields = Vec::new();
        let mut duplicate_fields = Vec::new();
        let mut unknown_fields = Vec::new();
        let mut type_mismatches = Vec::new();
        let exact_field_types = self
            .dependency_analysis
            .and_then(|dependency_analysis| {
                let direct = type_name.split_once('.').and_then(|(alias, stable_key)| {
                    dependency_analysis.direct_package_type(alias, stable_key)
                });
                let projected = || {
                    let PackageTypeRef::PackageSchema {
                        package_id,
                        stable_schema_key,
                        package_schema_type_id,
                    } = ContractProjectionState::project_resolved_type(
                        &target.ty,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    )
                    .ok()?
                    else {
                        return None;
                    };
                    dependency_analysis.exact_package_type(
                        &package_id,
                        &stable_schema_key,
                        &package_schema_type_id,
                    )
                };
                direct.or_else(projected).and_then(|record| {
                    let skiff_artifact_model::ContractTypeDescriptor::Record { fields } =
                        &record.canonical_descriptor.descriptor
                    else {
                        return None;
                    };
                    Some(
                        fields
                            .iter()
                            .map(|(name, ty)| {
                                (
                                    name.clone(),
                                    contract_call_typing::package_type_ref_from_contract_type(ty),
                                )
                            })
                            .collect::<BTreeMap<_, _>>(),
                    )
                })
            })
            .unwrap_or_default();
        for (index, (field_name, value_key)) in provided_field_keys.iter().enumerate() {
            let name_span = record_field_name_source_span(source_fact, index);
            let value_span = record_field_value_source_span(source_fact, index);
            provided_fields.push(ConstructorProvidedField {
                name: field_name.clone(),
                expression: value_key.clone(),
                actual: field_types.get(index).cloned().flatten(),
                name_span,
                value_span,
            });
            if let Some(first_expression) = provided.insert(field_name.clone(), value_key.clone()) {
                duplicates.insert(field_name.clone());
                duplicate_fields.push(DuplicateConstructorField {
                    name: field_name.clone(),
                    first_expression,
                    duplicate_expression: value_key.clone(),
                    name_span,
                });
                self.outputs.diagnostics.push(format!(
                    "{}: duplicate constructor field `{field_name}` at {}",
                    self.module_path,
                    span_label(name_span)
                ));
            }
        }

        for (index, (field_name, value)) in fields.iter().enumerate() {
            if duplicates.contains(field_name) {
                continue;
            }
            let value_key = provided_field_keys
                .get(index)
                .map(|(_, key)| key)
                .expect("constructor field keys must align with AST fields");
            let actual = field_types
                .get(index)
                .and_then(|field_type| field_type.as_ref());
            let Some(expected) = target.fields.get(field_name) else {
                unknown_fields.push(UnknownConstructorField {
                    name: field_name.clone(),
                    expression: value_key.clone(),
                    actual: actual.cloned(),
                    name_span: record_field_name_source_span(source_fact, index),
                    value_span: record_field_value_source_span(source_fact, index),
                });
                self.outputs.diagnostics.push(format!(
                    "{}: unknown constructor field `{field_name}` for `{type_name}` at {}",
                    self.module_path,
                    span_label(record_field_name_source_span(source_fact, index))
                ));
                continue;
            };
            if let Some(actual) = actual {
                let context = format!("constructor field `{field_name}`");
                if !self.check_value_assignable_to_expected(
                    value,
                    value_key,
                    actual,
                    expected,
                    ValueAssignmentContext {
                        annotation: None,
                        exact_expected: exact_field_types.get(field_name),
                        diagnostic_context: &context,
                        fallback_span: record_field_value_source_span(source_fact, index),
                    },
                ) {
                    type_mismatches.push(ConstructorFieldTypeMismatch {
                        name: field_name.clone(),
                        expression: value_key.clone(),
                        expected: expected.clone(),
                        actual: actual.clone(),
                        value_span: record_field_value_source_span(source_fact, index),
                    });
                }
            }
        }

        let mut materialized_fields = Vec::new();
        let mut missing_required_fields = Vec::new();
        for (field_name, expected) in &target.fields {
            if let Some(expression) = provided.get(field_name) {
                materialized_fields.push(MaterializedConstructorField {
                    name: field_name.clone(),
                    ty: expected.clone(),
                    source: ConstructorFieldValueSource::Provided {
                        field_name: field_name.clone(),
                        expression: expression.clone(),
                    },
                });
            } else if self.type_resolution.is_nullable(expected) {
                materialized_fields.push(MaterializedConstructorField {
                    name: field_name.clone(),
                    ty: expected.clone(),
                    source: ConstructorFieldValueSource::SyntheticNull,
                });
            } else {
                let span = source_fact
                    .map(|fact| fact.span)
                    .unwrap_or_else(SourceSpan::synthetic);
                missing_required_fields.push(MissingConstructorField {
                    name: field_name.clone(),
                    expected: expected.clone(),
                    span,
                });
                self.outputs.diagnostics.push(format!(
                    "{}: missing required constructor field `{field_name}` for `{type_name}` at {}",
                    self.module_path,
                    span_label(span)
                ));
            }
        }

        self.outputs.constructor_validations.insert(
            key.clone(),
            ConstructorValidation {
                target: target.ty.clone(),
                provided_fields,
                materialized_fields,
                duplicate_fields,
                missing_required_fields,
                unknown_fields,
                type_mismatches,
            },
        );
        Some(target.ty)
    }

    pub(super) fn transparent_value_target_key(&self, key: &ExpressionKey) -> ExpressionKey {
        let mut target = key.clone();
        let mut visited = BTreeSet::new();
        while visited.insert(target.clone()) {
            let Some(next) = self.transparent_value_targets.get(&target) else {
                break;
            };
            target = next.clone();
        }
        target
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn materialize_target_typed_object_literal(
        &mut self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &str,
    ) -> bool {
        self.outputs
            .object_materialization
            .targeted
            .insert(value_key.clone());
        let assignability = ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            self.dependency_analysis,
        );
        let assignability = if context.starts_with("call `std.json.encode` argument ") {
            assignability.with_package_json_context()
        } else {
            assignability
        };
        let plan = match assignability.object_literal_materialization_plan(
            annotation, value, value_key, actual, expected, context,
        ) {
            Ok(plan) => plan,
            Err(diagnostics) => {
                self.outputs.diagnostics.extend(diagnostics);
                return false;
            }
        };
        let Some(source) = self
            .outputs
            .object_materialization
            .sources
            .get(value_key)
            .cloned()
        else {
            self.outputs.diagnostics.push(format!(
                "{}: {context} target-typed object literal is missing source facts at {}",
                self.module_path,
                self.expression_span_label(value_key)
            ));
            return false;
        };
        let provided = source
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect::<BTreeMap<_, _>>();
        let mut fields = Vec::with_capacity(plan.fields.len());
        let mut valid = true;
        for (name, ty) in &plan.fields {
            let source = if let Some(provided) = provided.get(name.as_str()) {
                if let Some(actual) = &provided.actual {
                    valid &= self.check_value_assignable_to_expected(
                        object_literal_field_value(value, name)
                            .expect("materialization plan field must exist in object literal"),
                        &provided.expression,
                        actual,
                        ty,
                        ValueAssignmentContext {
                            annotation: None,
                            exact_expected: None,
                            diagnostic_context: &format!("{context} object literal field `{name}`"),
                            fallback_span: provided.value_span,
                        },
                    );
                } else {
                    // Exact contract-derived and flow-assigned bindings can be
                    // name-resolved without carrying a structural
                    // ResolvedTypeRef. Once the enclosing object has selected
                    // a unique target, retain that exact field target for the
                    // identifier instead of abandoning recursive
                    // materialization.
                    if !expression_accepts_contextual_target(
                        object_literal_field_value(value, name)
                            .expect("materialization plan field must exist in object literal"),
                    ) {
                        self.outputs.diagnostics.push(format!(
                            "{}: {context} object literal field `{name}` has no resolved expression type at {}",
                            self.module_path,
                            span_label(provided.value_span)
                        ));
                        valid = false;
                    }
                }
                ObjectFieldValueSource::Provided {
                    expression: provided.expression.clone(),
                }
            } else if self.type_resolution.is_nullable(ty) {
                ObjectFieldValueSource::SyntheticNull
            } else {
                self.outputs.diagnostics.push(format!(
                    "{}: {context} materialization plan omitted required object literal field `{name}` at {}",
                    self.module_path,
                    span_label(source.span)
                ));
                valid = false;
                continue;
            };
            fields.push(MaterializedObjectField {
                name: name.clone(),
                ty: ty.clone(),
                source,
            });
        }
        if valid {
            self.outputs.object_materialization.facts.insert(
                value_key.clone(),
                TargetTypedObjectMaterialization {
                    resolved_target: plan.resolved_target,
                    kind: plan.kind,
                    fields,
                },
            );
        }
        valid
    }

    pub(super) fn materialize_target_typed_array_literal(
        &mut self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &str,
    ) -> bool {
        let Expr::ArrayLiteral { items } = value else {
            return false;
        };
        let Some(element_ty) =
            exact_array_element_type(self.type_resolution, &expected.ir, &self.type_context)
        else {
            return self
                .generic_array_assignable(annotation, value, value_key, actual, expected, context);
        };
        let materialized = array_type_from_ir(element_ty.clone());
        let element_ty = ResolvedTypeRef::new(element_ty);
        let mut valid = true;
        for (index, item) in items.iter().enumerate() {
            let item_key = ExpressionKey::new(
                value_key.module_path().to_string(),
                value_key.owner().clone(),
                value_key
                    .preorder_index()
                    .checked_add((index as u32).saturating_add(1))
                    .expect("array item expression key should fit in u32"),
            );
            let Some(item_actual) = self
                .outputs
                .facts
                .get(&item_key)
                .and_then(|fact| fact.ty.clone())
            else {
                self.outputs.diagnostics.push(format!(
                    "{}: {context} array literal item at {} has no resolved expression type",
                    self.module_path,
                    self.expression_span_label(value_key)
                ));
                valid = false;
                continue;
            };
            valid &= self.check_value_assignable_to_expected(
                item,
                &item_key,
                &item_actual,
                &element_ty,
                ValueAssignmentContext {
                    annotation: None,
                    exact_expected: None,
                    diagnostic_context: &format!("{context} array element {index}"),
                    fallback_span: self.expression_span(&item_key),
                },
            );
        }
        if valid {
            if let Some(fact) = self.outputs.facts.get_mut(value_key) {
                fact.ty = Some(materialized.clone());
            }
            if let Some(dependency_analysis) = self.dependency_analysis {
                match ContractProjectionState::project_resolved_type(
                    &materialized,
                    self.type_resolution,
                    dependency_analysis,
                    &self.type_context,
                ) {
                    Ok(projected) => {
                        self.contract_projection
                            .record_expression_type(value_key.clone(), projected);
                    }
                    Err(error) => self.outputs.diagnostics.push(format!(
                        "{}: {context} array literal exact type projection failed at {}: {error}",
                        self.module_path,
                        self.expression_span_label(value_key)
                    )),
                }
            }
        }
        valid
    }

    fn generic_array_assignable(
        &mut self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &str,
    ) -> bool {
        let assignability = ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            self.dependency_analysis,
        );
        let assignability = if context.starts_with("call `std.json.encode` argument ") {
            assignability.with_package_json_context()
        } else {
            assignability
        };
        let expected_projected = self.dependency_analysis.and_then(|dependency_analysis| {
            annotation.and_then(|annotation| {
                ContractProjectionState::project_source_type_ref(
                    annotation,
                    self.type_resolution,
                    dependency_analysis,
                    &self.type_context,
                )
                .ok()
            })
        });
        match assignability.value_assignable_to_expected(
            annotation,
            value,
            actual,
            expected,
            expected_projected.as_ref(),
        ) {
            Ok(assignable) => assignable,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: {context} array assignability failed at {}: {error}",
                    self.module_path,
                    self.expression_span_label(value_key)
                ));
                false
            }
        }
    }
}

fn exact_array_element_type(
    type_resolution: &TypeResolutionModel,
    ty: &TypeRefIr,
    context: &TypeResolutionContext<'_>,
) -> Option<TypeRefIr> {
    let ty = type_resolution.transparent_alias_ir(ty, context);
    match ty {
        TypeRefIr::Builtin { name, args }
            if name == BuiltinShape::Array.name() && args.len() == 1 =>
        {
            Some(args[0].clone())
        }
        TypeRefIr::Nullable { inner } => exact_array_element_type(type_resolution, &inner, context),
        _ => None,
    }
}

fn expression_accepts_contextual_target(value: &Expr) -> bool {
    matches!(value, Expr::Identifier(_))
}

fn record_field_name_source_span(
    fact: Option<&crate::ExpressionSourceFact>,
    index: usize,
) -> SourceSpan {
    fact.and_then(|fact| fact.record_fields.get(index))
        .map(|field| field.name_span)
        .unwrap_or_else(SourceSpan::synthetic)
}

fn record_field_value_source_span(
    fact: Option<&crate::ExpressionSourceFact>,
    index: usize,
) -> SourceSpan {
    fact.and_then(|fact| fact.record_fields.get(index))
        .map(|field| field.value_span)
        .unwrap_or_else(SourceSpan::synthetic)
}
