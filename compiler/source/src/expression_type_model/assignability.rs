use super::*;

impl<'a> OwnerChecker<'a> {
    pub(super) fn binary_type(
        &self,
        op: BinaryOp,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
    ) -> Option<ResolvedTypeRef> {
        match op {
            BinaryOp::Add if self.operands_string_concat(left, right) => {
                self.resolve_builtin(BuiltinShape::String.name())
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::And
            | BinaryOp::Or => self.resolve_builtin(BuiltinShape::Bool.name()),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.resolve_builtin(BuiltinShape::Number.name())
            }
        }
    }

    pub(super) fn check_binary_operands(
        &mut self,
        key: &ExpressionKey,
        op: BinaryOp,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
        db_field_relational: bool,
    ) {
        match op {
            BinaryOp::Add if self.operands_string_concat(left, right) => {}
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.check_operand_assignable(
                    key,
                    "binary arithmetic operand",
                    left,
                    BuiltinShape::Number.name(),
                );
                self.check_operand_assignable(
                    key,
                    "binary arithmetic operand",
                    right,
                    BuiltinShape::Number.name(),
                );
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                if db_field_relational
                    && self.operands_both_assignable_to(left, right, BuiltinShape::String.name())
                {
                    return;
                }
                self.check_operand_assignable(
                    key,
                    "binary comparison operand",
                    left,
                    BuiltinShape::Number.name(),
                );
                self.check_operand_assignable(
                    key,
                    "binary comparison operand",
                    right,
                    BuiltinShape::Number.name(),
                );
            }
            BinaryOp::And | BinaryOp::Or => {
                self.check_operand_assignable(
                    key,
                    "binary boolean operand",
                    left,
                    BuiltinShape::Bool.name(),
                );
                self.check_operand_assignable(
                    key,
                    "binary boolean operand",
                    right,
                    BuiltinShape::Bool.name(),
                );
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let (Some(left), Some(right)) = (left, right) else {
                    return;
                };
                if !self
                    .type_resolution
                    .assignable_in_context(left, right, &self.type_context)
                    && !self
                        .type_resolution
                        .assignable_in_context(right, left, &self.type_context)
                {
                    self.outputs.diagnostics.push(format!(
                        "{}: equality operand type mismatch at {}: left {}, right {}",
                        self.module_path,
                        self.expression_span_label(key),
                        left,
                        right
                    ));
                }
            }
        }
    }

    pub(super) fn operands_string_concat(
        &self,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
    ) -> bool {
        let Some(expected) = self.resolve_builtin(BuiltinShape::String.name()) else {
            return false;
        };
        left.is_some_and(|left| {
            self.type_resolution
                .assignable_in_context(left, &expected, &self.type_context)
        }) || right.is_some_and(|right| {
            self.type_resolution
                .assignable_in_context(right, &expected, &self.type_context)
        })
    }

    pub(super) fn operands_both_assignable_to(
        &self,
        left: Option<&ResolvedTypeRef>,
        right: Option<&ResolvedTypeRef>,
        expected_builtin: &str,
    ) -> bool {
        let (Some(left), Some(right), Some(expected)) =
            (left, right, self.resolve_builtin(expected_builtin))
        else {
            return false;
        };
        self.type_resolution
            .assignable_in_context(left, &expected, &self.type_context)
            && self
                .type_resolution
                .assignable_in_context(right, &expected, &self.type_context)
    }

    pub(super) fn check_unary_operand(
        &mut self,
        key: &ExpressionKey,
        op: UnaryOp,
        actual: Option<&ResolvedTypeRef>,
    ) {
        match op {
            UnaryOp::Not => self.check_operand_assignable(
                key,
                "unary boolean operand",
                actual,
                BuiltinShape::Bool.name(),
            ),
        }
    }

    pub(super) fn check_operand_assignable(
        &mut self,
        key: &ExpressionKey,
        context: &str,
        actual: Option<&ResolvedTypeRef>,
        expected_builtin: &str,
    ) {
        let Some(actual) = actual else {
            return;
        };
        let Some(expected) = self.resolve_builtin(expected_builtin) else {
            return;
        };
        if !self
            .type_resolution
            .assignable_in_context(actual, &expected, &self.type_context)
        {
            self.outputs.diagnostics.push(format!(
                "{}: {context} type mismatch at {}: expected {}, found {}",
                self.module_path,
                self.expression_span_label(key),
                expected,
                actual
            ));
        }
    }

    pub(super) fn unary_type(&self, op: UnaryOp) -> Option<ResolvedTypeRef> {
        match op {
            UnaryOp::Not => self.resolve_builtin(BuiltinShape::Bool.name()),
        }
    }

    pub(super) fn check_assignable(
        &mut self,
        annotation: &TypeRef,
        actual: &ResolvedTypeRef,
        exact_expected: Option<&PackageTypeRef>,
        span: SourceSpan,
        context: &str,
        value: Option<(&Expr, &ExpressionKey)>,
    ) {
        match self
            .type_resolution
            .resolve_type_ref(annotation, &self.type_context)
        {
            Ok(expected) => {
                if let Some((value, key)) = value {
                    self.check_value_assignable_to_expected(
                        value,
                        key,
                        actual,
                        &expected,
                        ValueAssignmentContext {
                            annotation: Some(annotation),
                            exact_expected,
                            diagnostic_context: context,
                            fallback_span: span,
                        },
                    );
                } else if !self.type_resolution.assignable_in_context(
                    actual,
                    &expected,
                    &self.type_context,
                ) {
                    self.push_type_mismatch(context, span, &expected, actual);
                }
            }
            Err(error) => self.outputs.diagnostics.push(format!(
                "{}: failed to resolve {context} annotation at {}: {error}",
                self.module_path,
                span_label(span)
            )),
        }
    }

    pub(super) fn check_value_assignable_to_expected(
        &mut self,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        assignment: ValueAssignmentContext<'_>,
    ) -> bool {
        let ValueAssignmentContext {
            annotation,
            exact_expected,
            diagnostic_context: context,
            fallback_span,
        } = assignment;
        let target_key = self.transparent_value_target_key(value_key);
        let target_value = transparent_value_target(value);
        if matches!(target_value, Expr::ArrayLiteral { .. }) {
            return self.materialize_target_typed_array_literal(
                annotation,
                target_value,
                &target_key,
                actual,
                expected,
                context,
            );
        }
        if matches!(
            target_value,
            Expr::ObjectLiteral { .. } | Expr::MapLiteral { .. }
        ) {
            let target_actual = self
                .outputs
                .facts
                .get(&target_key)
                .and_then(|fact| fact.ty.clone())
                .unwrap_or_else(|| actual.clone());
            return self.materialize_target_typed_object_literal(
                annotation,
                target_value,
                &target_key,
                &target_actual,
                expected,
                context,
            );
        }
        let assignability = ExpressionAssignability::new(
            self.module_path,
            self.expression_sources,
            self.type_resolution,
            &self.type_context,
            self.dependency_analysis,
        );
        let package_json_context = context.starts_with("call `std.json.encode` argument ");
        let assignability = if package_json_context {
            assignability.with_package_json_context()
        } else {
            assignability
        };
        let expected_projected = match self.dependency_analysis {
            Some(dependency_analysis) => Some(match exact_expected {
                Some(expected) => Ok(expected.clone()),
                None => match annotation {
                    Some(annotation) => ContractProjectionState::project_source_type_ref(
                        annotation,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    ),
                    None => ContractProjectionState::project_resolved_type(
                        expected,
                        self.type_resolution,
                        dependency_analysis,
                        &self.type_context,
                    ),
                },
            }),
            None => None,
        };
        let expected_projected = match expected_projected.transpose() {
            Ok(expected) => expected,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: {context} exact source type projection failed at {}: {error}",
                    self.module_path,
                    span_label(fallback_span)
                ));
                return false;
            }
        };
        let contract_assignable = match contract_source_assignability_with_projections(
            actual,
            self.contract_projection.expression_type(value_key),
            expected,
            expected_projected.as_ref(),
            self.type_resolution,
            self.dependency_analysis,
            &self.type_context,
        ) {
            Ok(assignable) => assignable,
            Err(error) => {
                self.outputs.diagnostics.push(format!(
                    "{}: {context} exact source type projection failed at {}: {error}",
                    self.module_path,
                    span_label(fallback_span)
                ));
                return false;
            }
        };
        let assignable = match contract_assignable {
            Some(true) => true,
            Some(false) if !package_json_context => false,
            Some(false) | None => match assignability.value_assignable_to_expected(
                annotation,
                value,
                actual,
                expected,
                self.contract_projection.expression_type(value_key),
            ) {
                Ok(assignable) => assignable,
                Err(error) => {
                    self.outputs.diagnostics.push(format!(
                        "{}: {context} exact source type projection failed at {}: {error}",
                        self.module_path,
                        span_label(fallback_span)
                    ));
                    return false;
                }
            },
        };
        if assignable {
            self.record_union_target_constructor(value, value_key, actual, expected);
            return true;
        }
        let object_source = self
            .outputs
            .object_materialization
            .sources
            .get(value_key);
        if let Some(diagnostics) = assignability.object_literal_assignability_diagnostics(
            ObjectLiteralAssignabilityContext {
                annotation,
                value,
                value_key,
                actual,
                expected,
                diagnostic_context: context,
                source: object_source,
            },
        ) {
            if !diagnostics.is_empty() {
                self.outputs.diagnostics.extend(diagnostics);
                return false;
            }
        }
        if let (Some(expected), Some(actual)) = (
            expected_projected.as_ref(),
            self.contract_projection.expression_type(value_key),
        ) {
            self.outputs.diagnostics.push(format!(
                "{}: {context} canonical type identity mismatch at {}: expected {expected:?}, found {actual:?}",
                self.module_path,
                span_label(fallback_span),
            ));
        }
        self.push_type_mismatch(context, fallback_span, expected, actual);
        false
    }

    pub(super) fn push_type_mismatch(
        &mut self,
        context: &str,
        span: SourceSpan,
        expected: &ResolvedTypeRef,
        actual: &ResolvedTypeRef,
    ) {
        self.outputs.diagnostics.push(format!(
            "{}: {context} type mismatch at {}: expected {}, found {}",
            self.module_path,
            span_label(span),
            expected,
            actual
        ));
    }

    /// Records that a nominal record constructor enters an enclosing
    /// anonymous-union target context.
    ///
    /// `final leaf: LeafA | LeafB = LeafA {...}` and union-typed call
    /// parameters widen the constructor branch into the union for the rest of
    /// the bytecode lane: the runtime value still carries the concrete leaf
    /// identity, but the static expression type becomes the union so slot and
    /// call-argument facts line up with the declared union type. The rewrite
    /// is deliberately limited to named record constructors whose resolved
    /// target is an exact member of an anonymous union; no other construct
    /// gains union behavior here.
    fn record_union_target_constructor(
        &mut self,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
    ) {
        if !matches!(value, Expr::Record { .. }) {
            return;
        }
        let TypeRefIr::Union { items } = &expected.ir else {
            return;
        };
        if !items.iter().any(|item| item == &actual.ir) {
            return;
        }
        if let Some(fact) = self.outputs.facts.get_mut(value_key) {
            fact.ty = Some(expected.clone());
        }
    }
}
