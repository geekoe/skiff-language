use super::*;

impl<'a> OwnerChecker<'a> {
    pub(super) fn check_test_effect_request_subset(
        &mut self,
        value: &Expr,
        expected: &PackageTypeRef,
    ) {
        let Expr::ObjectLiteral { entries } = value else {
            self.check_test_effect_value(value, expected, "expect");
            return;
        };
        let resolved = resolved_package_type_ref(expected);
        let Some(TypeRefIr::Record { fields }) = self
            .type_resolution
            .type_shape_ir(&resolved, &self.type_context)
        else {
            self.check_test_effect_value(value, expected, "expect");
            return;
        };
        let mut selected = BTreeMap::new();
        for entry in entries {
            let Some(name) = object_literal_key_text(&entry.key) else {
                self.outputs.diagnostics.push(format!(
                    "{}: test effect expect subset keys must name static request fields",
                    self.module_path
                ));
                continue;
            };
            let Some(ty) = fields.get(&name) else {
                self.outputs.diagnostics.push(format!(
                    "{}: test effect expect subset contains unknown request field `{name}`",
                    self.module_path
                ));
                continue;
            };
            selected.insert(name, ty.clone());
        }
        let partial = ResolvedTypeRef::with_text(
            TypeRefIr::Record { fields: selected },
            format!("subset<{resolved}>"),
        );
        let key = self.peek_key();
        let actual = self.check_expr(value);
        if let Some(actual) = actual {
            self.check_value_assignable_to_expected(
                value,
                &key,
                &actual,
                &partial,
                ValueAssignmentContext {
                    annotation: None,
                    exact_expected: None,
                    diagnostic_context: "test effect expect subset",
                    fallback_span: self.expression_span(&key),
                },
            );
        }
    }

    pub(super) fn check_test_effect_value(
        &mut self,
        value: &Expr,
        expected: &PackageTypeRef,
        context: &str,
    ) {
        let key = self.peek_key();
        let actual = self.check_expr(value);
        let Some(actual) = actual else {
            return;
        };
        if let (Some(projected), Some(dependencies)) = (
            self.contract_projection.expression_type(&key),
            self.dependency_analysis,
        ) {
            if package_type_target_assignable(projected, expected, dependencies) {
                return;
            }
            self.outputs.diagnostics.push(format!(
                "{}: test effect {context} package type is not assignable to the declared target at {}",
                self.module_path,
                self.expression_span_label(&key)
            ));
            return;
        }
        let resolved_expected = resolved_package_type_ref(expected);
        self.check_value_assignable_to_expected(
            value,
            &key,
            &actual,
            &resolved_expected,
            ValueAssignmentContext {
                annotation: None,
                exact_expected: Some(expected),
                diagnostic_context: &format!("test effect {context}"),
                fallback_span: self.expression_span(&key),
            },
        );
    }

    pub(super) fn check_test_effect_throw(&mut self, value: &Expr, target: &str) {
        let key = self.peek_key();
        let Some(actual) = self.check_expr(value) else {
            return;
        };
        if let Err(error) = self
            .type_resolution
            .catch_leaves(&actual, &self.type_context)
        {
            self.outputs.diagnostics.push(format!(
                "{}: test effect `{target}` throw has invalid catch payload at {}: {error}",
                self.module_path,
                self.expression_span_label(&key),
            ));
            return;
        }
        if let Some(fact) = self.outputs.facts.get_mut(&key) {
            fact.test_effect_throw_payload_type = Some(actual.ir);
        }
    }

    pub(super) fn validate_throw_payload(
        &mut self,
        key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        construct: &str,
    ) {
        if let Err(error) = self
            .type_resolution
            .catch_leaves(actual, &self.type_context)
        {
            self.outputs.diagnostics.push(format!(
                "{}: {construct} payload `{}` has no valid nominal catch identity at {}: {error}",
                self.module_path,
                actual,
                self.expression_span_label(key)
            ));
        }
    }

    pub(super) fn validate_rethrow_operand(
        &mut self,
        key: &ExpressionKey,
        actual: &ResolvedTypeRef,
    ) {
        if let Err(error) = self
            .type_resolution
            .exception_catch_leaves(actual, &self.type_context)
        {
            self.outputs.diagnostics.push(format!(
                "{}: invalid rethrow operand `{}` at {}: {error}",
                self.module_path,
                actual,
                self.expression_span_label(key)
            ));
        }
    }
}
