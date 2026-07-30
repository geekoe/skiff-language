use std::collections::{BTreeMap, BTreeSet};

use crate::shared::{
    ast::{Block, Expr, Stmt},
    ast_utils::AstVisitor,
};

use super::{
    collectors::{
        referenced_names_in_expr, referenced_names_in_stmt, statement_kind,
        ForbiddenConcurrentExpression, LocalNameCollector,
    },
    effects::{effect_profile_for_expr, effect_profile_for_stmt, validate_lane_effects},
    model::{ConcurrentLaneKind, ConcurrentLanePlan, ConcurrentSourcePlan},
    mutation::{BindingRoot, Scope},
    owner::{OwnerAnalyzer, ValidationContext},
};

impl OwnerAnalyzer<'_> {
    pub(super) fn validate_concurrent(
        &mut self,
        body: &Block,
        tail: Option<&Expr>,
        scope: &Scope,
        context: ValidationContext,
        produces_value: bool,
    ) {
        if context.in_lane {
            self.diagnostic("illegal concurrent surface: nested concurrent");
        }
        let mut direct_consts = BTreeMap::<String, Vec<u32>>::new();
        for (index, statement) in body.statements.iter().enumerate() {
            if let Stmt::Let {
                mutable: false,
                name,
                ..
            } = statement
            {
                direct_consts
                    .entry(name.clone())
                    .or_default()
                    .push(index as u32);
            }
        }
        let mut nested_names = BTreeSet::new();
        for statement in &body.statements {
            let mut collector = LocalNameCollector::default();
            collector.visit_stmt(statement);
            if let Stmt::Let { name, .. } = statement {
                collector.names.remove(name);
            }
            nested_names.extend(collector.names);
        }
        let plan_site = tail
            .map(|tail| self.expr_site(tail))
            .or_else(|| {
                body.statements
                    .first()
                    .map(|statement| self.stmt_site(statement))
            })
            .unwrap_or_else(|| self.owner_site());
        let mut lanes = Vec::with_capacity(body.statements.len() + usize::from(tail.is_some()));
        let mut sibling_scope = scope.clone();
        let mut lane_profiles = Vec::new();

        for (index, statement) in body.statements.iter().enumerate() {
            let source_order = index as u32;
            let references = referenced_names_in_stmt(statement);
            let dependencies = references
                .iter()
                .filter_map(|name| {
                    direct_consts.get(name).and_then(|declarations| {
                        declarations
                            .iter()
                            .rev()
                            .find(|declaration| **declaration < source_order)
                            .copied()
                    })
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            self.validate_concurrent_references(
                &references,
                source_order,
                &direct_consts,
                &nested_names,
                &sibling_scope,
            );
            let kind = if matches!(statement, Stmt::Serial { .. }) {
                ConcurrentLaneKind::Serial
            } else {
                ConcurrentLaneKind::Statement
            };
            let lane_context = ValidationContext {
                value_boundary: context.value_boundary || produces_value,
                in_lane: true,
            };
            self.validate_lane_stmt(statement, &mut sibling_scope.clone(), lane_context, kind);
            lane_profiles.push(effect_profile_for_stmt(
                statement,
                self.expression_keys,
                self.resolved_targets,
                self.callable_profiles,
            ));
            lanes.push(ConcurrentLanePlan {
                source_order,
                kind,
                dependencies,
                source_site: self.stmt_site(statement),
            });
            if let Stmt::Let {
                mutable: false,
                name,
                value,
                ..
            } = statement
            {
                let _ = value;
                sibling_scope.insert(name.clone(), BindingRoot::ConcurrentSibling);
            }
        }

        if let Some(tail) = tail {
            let source_order = lanes.len() as u32;
            let references = referenced_names_in_expr(tail);
            self.validate_concurrent_references(
                &references,
                source_order,
                &direct_consts,
                &nested_names,
                &sibling_scope,
            );
            let mut dependencies = (0..source_order).collect::<Vec<_>>();
            dependencies.sort_unstable();
            self.validate_lane_expr(
                tail,
                &mut sibling_scope.clone(),
                ValidationContext {
                    value_boundary: true,
                    in_lane: true,
                },
            );
            lane_profiles.push(effect_profile_for_expr(
                tail,
                self.expression_keys,
                self.resolved_targets,
                self.callable_profiles,
            ));
            lanes.push(ConcurrentLanePlan {
                source_order,
                kind: ConcurrentLaneKind::Tail,
                dependencies,
                source_site: self.expr_site(tail),
            });
        }

        for diagnostic in validate_lane_effects(&lane_profiles) {
            self.diagnostic(diagnostic);
        }
        self.semantics.concurrent_plans.push(ConcurrentSourcePlan {
            produces_value,
            lanes,
            source_site: plan_site,
        });
    }

    fn validate_concurrent_references(
        &mut self,
        references: &BTreeSet<String>,
        source_order: u32,
        direct_consts: &BTreeMap<String, Vec<u32>>,
        nested_names: &BTreeSet<String>,
        scope: &Scope,
    ) {
        for name in references {
            if let Some(declarations) = direct_consts.get(name) {
                if !declarations
                    .iter()
                    .any(|declaration| *declaration < source_order)
                {
                    let declaration = declarations
                        .iter()
                        .copied()
                        .find(|declaration| *declaration >= source_order)
                        .expect("direct const declarations are source ordered");
                    self.diagnostic(format!(
                        "concurrent forward reference `{name}` from lane {source_order} to lane {declaration}"
                    ));
                }
            } else if nested_names.contains(name) && !scope.contains_key(name) {
                self.diagnostic(format!(
                    "concurrent binding `{name}` is not sibling-visible outside its serial lane"
                ));
            }
        }
    }

    pub(super) fn validate_lane_stmt(
        &mut self,
        statement: &Stmt,
        scope: &mut Scope,
        context: ValidationContext,
        kind: ConcurrentLaneKind,
    ) {
        let mut forbidden_expression = ForbiddenConcurrentExpression::default();
        forbidden_expression.visit_stmt(statement);
        if let Some(kind) = forbidden_expression.first {
            self.diagnostic(format!("illegal concurrent surface: {kind}"));
        }
        match statement {
            Stmt::Let { mutable: true, .. } => {
                self.diagnostic("concurrent surface rejects mutable `let` bindings");
                self.validate_stmt(statement, scope, context);
            }
            Stmt::Let { .. } | Stmt::Assign { .. } | Stmt::Assert { .. } | Stmt::Expr(_) => {
                self.validate_stmt(statement, scope, context)
            }
            Stmt::DbTransaction { body } => {
                let mut transaction_scope = scope.clone();
                for nested in &body.statements {
                    self.validate_lane_stmt(
                        nested,
                        &mut transaction_scope,
                        context,
                        ConcurrentLaneKind::Statement,
                    );
                }
            }
            Stmt::Serial { body } if kind == ConcurrentLaneKind::Serial => {
                let mut serial_scope = scope.clone();
                for nested in &body.statements {
                    if matches!(nested, Stmt::Serial { .. } | Stmt::Concurrent { .. }) {
                        self.diagnostic("illegal concurrent surface: nested serial/concurrent");
                    }
                    self.validate_lane_stmt(
                        nested,
                        &mut serial_scope,
                        context,
                        ConcurrentLaneKind::Statement,
                    );
                }
            }
            Stmt::CompilerTestEffectRegister { .. }
            | Stmt::Timeout { .. }
            | Stmt::Concurrent { .. }
            | Stmt::Serial { .. }
            | Stmt::If { .. }
            | Stmt::For { .. }
            | Stmt::Match { .. }
            | Stmt::Throw { .. }
            | Stmt::Rethrow { .. }
            | Stmt::Emit(_)
            | Stmt::Return(_)
            | Stmt::Spawn { .. }
            | Stmt::Break
            | Stmt::Continue => {
                self.diagnostic(format!(
                    "illegal concurrent surface: {}",
                    statement_kind(statement)
                ));
                self.validate_stmt(statement, scope, context);
            }
        }
    }

    fn validate_lane_expr(
        &mut self,
        expression: &Expr,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        let mut forbidden = ForbiddenConcurrentExpression::default();
        forbidden.visit_expr(expression);
        if let Some(kind) = forbidden.first {
            self.diagnostic(format!("illegal concurrent surface: {kind}"));
        }
        self.validate_expr(expression, scope, context);
    }
}
