use super::*;

impl<'a> OwnerChecker<'a> {
    pub(super) fn check_block_scoped(&mut self, block: &Block, narrowing: &TypeNarrowing) -> bool {
        let saved_env = self.env.clone();
        let saved_projected_env = self.contract_projection.binding_snapshot();
        let saved_path_refinements = self.path_refinements.clone();
        self.apply_narrowing(narrowing);
        let exits = self.check_block(block);
        self.env = saved_env;
        self.contract_projection
            .restore_bindings(saved_projected_env);
        self.path_refinements = saved_path_refinements;
        exits
    }

    pub(super) fn check_expr_scoped(
        &mut self,
        expr: &Expr,
        narrowing: &TypeNarrowing,
    ) -> Option<ResolvedTypeRef> {
        let saved_env = self.env.clone();
        let saved_projected_env = self.contract_projection.binding_snapshot();
        let saved_path_refinements = self.path_refinements.clone();
        self.apply_narrowing(narrowing);
        let ty = self.check_expr(expr);
        self.env = saved_env;
        self.contract_projection
            .restore_bindings(saved_projected_env);
        self.path_refinements = saved_path_refinements;
        ty
    }

    pub(super) fn apply_narrowing(&mut self, narrowing: &TypeNarrowing) {
        let projected_bindings = self.contract_projection.binding_snapshot();
        for (name, ty) in &narrowing.env {
            self.env.insert(name.clone(), ty.clone());
            let projected = match projected_bindings.get(name) {
                Some(PackageTypeRef::Nullable { inner })
                    if !matches!(ty.ir, TypeRefIr::Nullable { .. }) =>
                {
                    Some((**inner).clone())
                }
                _ => self
                    .dependency_analysis
                    .and_then(|dependency_analysis| {
                        ContractProjectionState::project_resolved_type(
                            ty,
                            self.type_resolution,
                            dependency_analysis,
                            &self.type_context,
                        )
                        .ok()
                    })
                    .or_else(|| projected_bindings.get(name).cloned()),
            };
            self.contract_projection.bind(name, projected);
        }
        for (path, ty) in &narrowing.paths {
            self.path_refinements.insert(path.clone(), ty.clone());
        }
    }

    pub(super) fn condition_narrowings(&self, condition: &Expr) -> ConditionNarrowings {
        match condition {
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
            } => {
                let nested = self.condition_narrowings(expr);
                ConditionNarrowings {
                    when_true: nested.when_false,
                    when_false: nested.when_true,
                }
            }
            Expr::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => ConditionNarrowings {
                when_true: self
                    .condition_narrowings(left)
                    .when_true
                    .combined(self.condition_narrowings(right).when_true),
                when_false: TypeNarrowing::default(),
            },
            Expr::Binary {
                op: BinaryOp::Or,
                left,
                right,
            } => ConditionNarrowings {
                when_true: TypeNarrowing::default(),
                when_false: self
                    .condition_narrowings(left)
                    .when_false
                    .combined(self.condition_narrowings(right).when_false),
            },
            Expr::Binary { op, left, right } if matches!(op, BinaryOp::Eq | BinaryOp::Ne) => {
                self.equality_narrowings(*op, left, right)
            }
            _ => ConditionNarrowings::default(),
        }
    }

    pub(super) fn equality_narrowings(
        &self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> ConditionNarrowings {
        if expr_is_null_literal(right) {
            return self.null_comparison_narrowings(op, left);
        }
        if expr_is_null_literal(left) {
            return self.null_comparison_narrowings(op, right);
        }
        if let Some(value) = expr_string_literal(right) {
            return self.discriminant_narrowings(op, left, value);
        }
        if let Some(value) = expr_string_literal(left) {
            return self.discriminant_narrowings(op, right, value);
        }
        ConditionNarrowings::default()
    }

    pub(super) fn null_comparison_narrowings(
        &self,
        op: BinaryOp,
        value: &Expr,
    ) -> ConditionNarrowings {
        let Some(non_null) = self
            .type_for_path_expr(value)
            .and_then(|ty| non_nullable_type(&ty))
        else {
            return ConditionNarrowings::default();
        };
        let narrowing = self.narrowing_for_path_expr(value, non_null);
        match op {
            BinaryOp::Eq => ConditionNarrowings {
                when_true: TypeNarrowing::default(),
                when_false: narrowing,
            },
            BinaryOp::Ne => ConditionNarrowings {
                when_true: narrowing,
                when_false: TypeNarrowing::default(),
            },
            _ => ConditionNarrowings::default(),
        }
    }

    pub(super) fn discriminant_narrowings(
        &self,
        op: BinaryOp,
        tag_expr: &Expr,
        tag_value: &str,
    ) -> ConditionNarrowings {
        let Expr::Field { object, field } = tag_expr else {
            return ConditionNarrowings::default();
        };
        if field != "tag" {
            return ConditionNarrowings::default();
        }
        let Some(object_ty) = self.type_for_path_expr(object) else {
            return ConditionNarrowings::default();
        };
        let Some(matching) = self.narrow_type_by_tag(&object_ty, tag_value, true) else {
            return ConditionNarrowings::default();
        };
        let non_matching = self.narrow_type_by_tag(&object_ty, tag_value, false);
        let matching = self.narrowing_for_path_expr(object, matching);
        let non_matching = non_matching
            .map(|ty| self.narrowing_for_path_expr(object, ty))
            .unwrap_or_default();
        match op {
            BinaryOp::Eq => ConditionNarrowings {
                when_true: matching,
                when_false: non_matching,
            },
            BinaryOp::Ne => ConditionNarrowings {
                when_true: non_matching,
                when_false: matching,
            },
            _ => ConditionNarrowings::default(),
        }
    }

    pub(super) fn null_guard_assignment_narrowing(
        &self,
        condition: &Expr,
        then_block: &Block,
    ) -> Option<TypeNarrowing> {
        let Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } = condition
        else {
            return None;
        };
        let value = if expr_is_null_literal(right) {
            left.as_ref()
        } else if expr_is_null_literal(left) {
            right.as_ref()
        } else {
            return None;
        };
        let path = expr_path(value)?;
        if !block_assigns_obviously_non_null(then_block, &path) {
            return None;
        }
        self.type_for_path_expr(value)
            .and_then(|ty| non_nullable_type(&ty))
            .map(|ty| self.narrowing_for_path_expr(value, ty))
    }

    pub(super) fn type_for_path_expr(&self, expr: &Expr) -> Option<ResolvedTypeRef> {
        let path = expr_path(expr)?;
        if let Some(ty) = self.path_refinements.get(&path) {
            return Some(ty.clone());
        }
        match expr {
            Expr::Identifier(name) => self.env.get(name).cloned(),
            Expr::Field { object, field } => self
                .type_for_path_expr(object)
                .and_then(|object_ty| self.record_field_type(&object_ty, field)),
            _ => None,
        }
    }

    pub(super) fn narrow_type_by_tag(
        &self,
        ty: &ResolvedTypeRef,
        tag_value: &str,
        include_matching: bool,
    ) -> Option<ResolvedTypeRef> {
        narrow_type_by_tag(ty, tag_value, include_matching).or_else(|| {
            let shape = self.type_resolution.type_shape_ir(ty, &self.type_context)?;
            narrow_type_by_tag(&resolved_type_from_ir(&shape), tag_value, include_matching)
        })
    }

    pub(super) fn narrowing_for_path_expr(
        &self,
        expr: &Expr,
        ty: ResolvedTypeRef,
    ) -> TypeNarrowing {
        match expr {
            Expr::Identifier(name) => TypeNarrowing {
                env: BTreeMap::from([(name.clone(), ty)]),
                paths: BTreeMap::new(),
            },
            _ => expr_path(expr)
                .map(|path| TypeNarrowing {
                    env: BTreeMap::new(),
                    paths: BTreeMap::from([(path, ty)]),
                })
                .unwrap_or_default(),
        }
    }

    pub(super) fn check_condition(&mut self, condition: &Expr, context: &str) {
        let actual = self.check_expr(condition);
        let Some(actual) = actual else {
            return;
        };
        let Some(expected) = self.resolve_builtin(BuiltinShape::Bool.name()) else {
            return;
        };
        if !self
            .type_resolution
            .assignable_in_context(&actual, &expected, &self.type_context)
        {
            self.outputs.diagnostics.push(format!(
                "{}: {context} type mismatch at {}: expected bool, found {}",
                self.module_path,
                self.current_expression_span_label(),
                actual
            ));
        }
    }
}

fn non_nullable_type(ty: &ResolvedTypeRef) -> Option<ResolvedTypeRef> {
    match &ty.ir {
        TypeRefIr::Nullable { inner } => {
            let source_text = ty
                .to_string()
                .trim()
                .strip_suffix('?')
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| debug_text(inner));
            Some(ResolvedTypeRef::with_text(
                inner.as_ref().clone(),
                source_text,
            ))
        }
        TypeRefIr::Union { items } => {
            let remaining = items
                .iter()
                .filter(|item| !is_null_type(item))
                .cloned()
                .collect::<Vec<_>>();
            (remaining.len() != items.len()).then(|| {
                resolved_type_from_ir(&normalize_union(TypeRefIr::Union { items: remaining }))
            })
        }
        _ => None,
    }
}

fn narrow_type_by_tag(
    ty: &ResolvedTypeRef,
    tag_value: &str,
    include_matching: bool,
) -> Option<ResolvedTypeRef> {
    let branches = catch_result_branches(&ty.ir)?;
    let selected = branches
        .into_iter()
        .filter(|branch| {
            record_tag_literal(branch).is_some_and(|value| (value == tag_value) == include_matching)
        })
        .collect::<Vec<_>>();
    (!selected.is_empty())
        .then(|| resolved_type_from_ir(&normalize_union(TypeRefIr::Union { items: selected })))
}

fn record_tag_literal(ty: &TypeRefIr) -> Option<&str> {
    let TypeRefIr::Record { fields } = ty else {
        return None;
    };
    let TypeRefIr::Literal {
        value: LiteralIr::String { value },
    } = fields.get("tag")?
    else {
        return None;
    };
    Some(value.as_str())
}
