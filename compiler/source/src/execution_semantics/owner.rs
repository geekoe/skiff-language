use std::collections::{BTreeMap, BTreeSet};

use crate::{
    shared::{
        ast::{Block, Expr, ForBinding, FunctionDecl, Stmt, ValueBlock},
        ast_utils::{walk_expr, AstVisitor},
        error::SourceSpan,
    },
    ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap, ResolvedCallTarget,
    ResolvedCallTargetFacts, SourceCallableEffectFacts, SourceSymbolKey,
};

use super::{
    collectors::{expr_address, pattern_bindings, LocalNameCollector},
    model::{ExecutionSourceSite, SourceExecutionSemantics, TimeoutSourcePlan},
    mutation::{binding_root_for_value, BindingRoot, Scope},
};

#[allow(clippy::too_many_arguments)]
pub(super) struct OwnerAnalyzer<'a> {
    module_path: &'a str,
    owner: ExpressionOwnerKey,
    source_key: SourceSymbolKey,
    function: &'a FunctionDecl,
    expression_sources: &'a ExpressionSourceMap,
    pub(super) expression_keys: &'a BTreeMap<usize, ExpressionKey>,
    pub(super) resolved_targets: &'a ResolvedCallTargetFacts,
    pub(super) callable_effects: &'a SourceCallableEffectFacts,
    pub(super) semantics: &'a mut SourceExecutionSemantics,
    diagnostics: &'a mut Vec<String>,
    all_local_names: BTreeSet<String>,
    top_level_value_names: BTreeSet<String>,
}

impl<'a> OwnerAnalyzer<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        module_path: &'a str,
        owner: ExpressionOwnerKey,
        source_key: SourceSymbolKey,
        function: &'a FunctionDecl,
        expression_sources: &'a ExpressionSourceMap,
        expression_keys: &'a BTreeMap<usize, ExpressionKey>,
        resolved_targets: &'a ResolvedCallTargetFacts,
        callable_effects: &'a SourceCallableEffectFacts,
        top_level_value_names: BTreeSet<String>,
        semantics: &'a mut SourceExecutionSemantics,
        diagnostics: &'a mut Vec<String>,
    ) -> Self {
        let mut local_names = LocalNameCollector::default();
        local_names.visit_block(&function.body);
        Self {
            module_path,
            owner,
            source_key,
            function,
            expression_sources,
            expression_keys,
            resolved_targets,
            callable_effects,
            semantics,
            diagnostics,
            all_local_names: local_names.names,
            top_level_value_names,
        }
    }

    pub(super) fn analyze(&mut self) {
        let mut scope = self
            .function
            .params
            .iter()
            .map(|parameter| (parameter.name.clone(), BindingRoot::Outer))
            .collect::<Scope>();
        if self.function.implicit_self.is_some() {
            scope.insert("self".to_string(), BindingRoot::Outer);
        }
        self.validate_block(
            &self.function.body,
            &mut scope,
            ValidationContext::default(),
        );
    }

    pub(super) fn validate_block(
        &mut self,
        block: &Block,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        for statement in &block.statements {
            self.validate_stmt(statement, scope, context);
        }
    }

    pub(super) fn validate_stmt(
        &mut self,
        statement: &Stmt,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        match statement {
            Stmt::CompilerTestEffectRegister {
                target_probe,
                expect,
                step_expect,
                outcome,
                ..
            } => {
                self.validate_expr(target_probe, scope, context);
                if let Some(expect) = expect {
                    self.validate_expr(expect, scope, context);
                }
                if let Some(step_expect) = step_expect {
                    self.validate_expr(step_expect, scope, context);
                }
                match outcome {
                    crate::shared::ast::TestEffectStepOutcome::Respond { value }
                    | crate::shared::ast::TestEffectStepOutcome::Throw { value } => {
                        self.validate_expr(value, scope, context);
                    }
                    crate::shared::ast::TestEffectStepOutcome::Stream { events } => {
                        for event in events {
                            self.validate_expr(event, scope, context);
                        }
                    }
                }
            }
            Stmt::Assert { condition, .. } => self.validate_expr(condition, scope, context),
            Stmt::Let {
                mutable,
                name,
                value,
                ..
            } => {
                self.validate_expr(value, scope, context);
                scope.insert(
                    name.clone(),
                    binding_root_for_value(value, scope, context.in_lane, *mutable),
                );
            }
            Stmt::Assign { target, value } => {
                self.validate_expr(target, scope, context);
                self.validate_expr(value, scope, context);
                self.validate_mutation_target(target, scope, context);
                self.taint_lane_local_root_from_payloads(target, [value], scope, context);
            }
            Stmt::Timeout { duration, body } => {
                self.record_timeout(duration, false, duration.span);
                let mut nested = scope.clone();
                self.validate_block(body, &mut nested, context);
            }
            Stmt::Concurrent { .. } => {
                self.diagnostic("concurrent is not supported in v1");
            }
            Stmt::Serial { .. } => {
                self.diagnostic("serial is not supported in v1");
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.validate_expr(condition, scope, context);
                let mut then_scope = scope.clone();
                self.validate_block(then_block, &mut then_scope, context);
                if let Some(else_block) = else_block {
                    let mut else_scope = scope.clone();
                    self.validate_block(else_block, &mut else_scope, context);
                }
            }
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
                self.validate_expr(iterable, scope, context);
                let mut nested = scope.clone();
                match binding {
                    ForBinding::Item { item } => {
                        nested.insert(item.clone(), BindingRoot::LaneLocalOpaque);
                    }
                    ForBinding::Entry { key, value } => {
                        nested.insert(key.clone(), BindingRoot::LaneLocalOpaque);
                        nested.insert(value.clone(), BindingRoot::LaneLocalOpaque);
                    }
                }
                self.validate_block(body, &mut nested, context);
            }
            Stmt::While { condition, body } => {
                self.validate_expr(condition, scope, context);
                let mut nested = scope.clone();
                self.validate_block(body, &mut nested, context);
            }
            Stmt::Match { value, arms } => {
                self.validate_expr(value, scope, context);
                for arm in arms {
                    let mut nested = scope.clone();
                    let mut bindings = BTreeSet::new();
                    pattern_bindings(&arm.pattern, &mut bindings);
                    nested.extend(
                        bindings
                            .into_iter()
                            .map(|name| (name, BindingRoot::LaneLocalOpaque)),
                    );
                    self.validate_block(&arm.body, &mut nested, context);
                }
            }
            Stmt::DbTransaction { body } => {
                let mut nested = scope.clone();
                self.validate_block(body, &mut nested, context);
            }
            Stmt::Throw { value } => self.validate_expr(value, scope, context),
            Stmt::Rethrow { exception } => self.validate_expr(exception, scope, context),
            Stmt::Emit(value) | Stmt::Expr(value) => {
                self.validate_expr(value, scope, context);
            }
            Stmt::Return(value) => {
                if context.value_boundary {
                    self.diagnostic("value block control flow cannot cross the value boundary");
                }
                if let Some(value) = value {
                    self.validate_expr(value, scope, context);
                }
            }
            Stmt::Spawn { call } => self.validate_expr(call, scope, context),
            Stmt::Break | Stmt::Continue => {
                if context.value_boundary {
                    self.diagnostic("value block control flow cannot cross the value boundary");
                }
            }
        }
    }

    pub(super) fn validate_expr(
        &mut self,
        expression: &Expr,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        match expression {
            Expr::Literal(_) | Expr::DependencySourceAddress(_) => {}
            Expr::Identifier(name) => {
                if !scope.contains_key(name)
                    && self.all_local_names.contains(name)
                    && !self.top_level_value_names.contains(name)
                {
                    self.diagnostic(format!(
                        "unresolved local name `{name}` outside lexical scope"
                    ));
                }
            }
            Expr::Binary { left, right, .. } => {
                self.validate_expr(left, scope, context);
                self.validate_expr(right, scope, context);
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.validate_expr(condition, scope, context);
                self.validate_expr(then_expr, scope, context);
                self.validate_expr(else_expr, scope, context);
            }
            Expr::Unary { expr, .. } | Expr::Generic { callee: expr, .. } => {
                self.validate_expr(expr, scope, context);
            }
            Expr::Call { callee, args } => {
                self.validate_expr(callee, scope, context);
                for argument in args {
                    self.validate_expr(argument, scope, context);
                }
                self.validate_mutating_call(expression, scope, context);
            }
            Expr::InterfaceBox { value, .. } => self.validate_expr(value, scope, context),
            Expr::Field { object, .. } => self.validate_expr(object, scope, context),
            Expr::Record { fields, .. } => {
                for (_, value) in fields {
                    self.validate_expr(value, scope, context);
                }
            }
            Expr::ObjectLiteral { entries } => {
                for entry in entries {
                    self.validate_expr(&entry.value, scope, context);
                }
            }
            Expr::Patch { operations, .. } => {
                for operation in operations {
                    match operation {
                        crate::shared::ast::PatchOperation::Set { value, .. }
                        | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                            self.validate_expr(value, scope, context);
                        }
                    }
                }
            }
            Expr::ValueBlock(value) => self.validate_value_block(value, scope, context),
            Expr::ConcurrentValue(_) => {
                self.diagnostic("concurrent value is not supported in v1");
            }
            Expr::Timeout { duration, value } => {
                self.record_timeout(duration, true, self.expr_span(expression));
                self.validate_expr(value, scope, context);
            }
            Expr::Throw { value } => self.validate_expr(value, scope, context),
            Expr::Rethrow { exception } => self.validate_expr(exception, scope, context),
            Expr::Catch { try_expr, .. } => self.validate_expr(try_expr, scope, context),
            Expr::DbOperation(operation) => {
                let mut visitor = NestedExpressionValidator {
                    analyzer: self,
                    scope,
                    context,
                };
                walk_expr(&mut visitor, expression);
                let _ = operation;
            }
            Expr::DbQuery(_) | Expr::DbLeaseRead(_) => {
                let mut visitor = NestedExpressionValidator {
                    analyzer: self,
                    scope,
                    context,
                };
                walk_expr(&mut visitor, expression);
            }
            Expr::DbTransaction(transaction) => {
                let mut nested = scope.clone();
                self.validate_block(&transaction.body, &mut nested, context);
            }
            Expr::DbLeaseClaim(claim) => {
                self.validate_expr(&claim.key, scope, context);
                let mut nested = scope.clone();
                if let Some(binding) = &claim.binding {
                    nested.insert(binding.clone(), BindingRoot::LaneLocalOpaque);
                }
                self.validate_block(&claim.body, &mut nested, context);
            }
        }
    }

    fn validate_value_block(
        &mut self,
        value: &ValueBlock,
        scope: &Scope,
        context: ValidationContext,
    ) {
        let mut nested = scope.clone();
        let value_context = ValidationContext {
            value_boundary: true,
            ..context
        };
        self.validate_block(&value.body, &mut nested, value_context);
        self.validate_expr(&value.tail, &mut nested, value_context);
    }

    fn record_timeout(
        &mut self,
        duration: &crate::shared::ast::DurationLiteral,
        produces_value: bool,
        span: SourceSpan,
    ) {
        match duration.checked_milliseconds() {
            Ok(duration_milliseconds) => {
                self.semantics.timeout_plans.push(TimeoutSourcePlan {
                    duration_milliseconds,
                    produces_value,
                    source_site: ExecutionSourceSite {
                        module_path: self.module_path.to_string(),
                        owner: self.owner.clone(),
                        span,
                    },
                });
            }
            Err(error) => self.diagnostic(format!(
                "invalid checked timeout duration {}{}: {error}",
                duration.digits,
                duration.unit.suffix()
            )),
        }
    }

    pub(super) fn resolved_target(&self, expression: &Expr) -> Option<&ResolvedCallTarget> {
        self.expression_keys
            .get(&expr_address(expression))
            .and_then(|key| self.resolved_targets.target(key))
    }

    fn expr_span(&self, expression: &Expr) -> SourceSpan {
        self.expression_keys
            .get(&expr_address(expression))
            .and_then(|key| self.expression_sources.fact(key))
            .map(|fact| fact.span)
            .unwrap_or(self.function.span)
    }

    pub(super) fn diagnostic(&mut self, message: impl Into<String>) {
        self.diagnostics.push(format!(
            "{} {:?}: {}",
            self.module_path,
            self.owner,
            message.into()
        ));
    }
}

struct NestedExpressionValidator<'a, 'b> {
    analyzer: &'a mut OwnerAnalyzer<'b>,
    scope: &'a mut Scope,
    context: ValidationContext,
}

impl AstVisitor for NestedExpressionValidator<'_, '_> {
    fn visit_expr(&mut self, expression: &Expr) {
        self.analyzer
            .validate_expr(expression, self.scope, self.context);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ValidationContext {
    pub(super) value_boundary: bool,
    pub(super) in_lane: bool,
}
