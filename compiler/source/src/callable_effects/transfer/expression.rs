use skiff_artifact_model::{CallableProvenanceUnknownReason, ValueProjectionPath};

use crate::shared::ast::{
    BinaryOp, DbBlockMode, DbBody, DbChangeOp, DbOperation, DbQueryBlock, DbSelector,
    DbWhereClause, DispatchTiming, Expr, PatchOperation, Stmt,
};

use skiff_artifact_model::PendingEffectCategory;

use super::{
    super::analysis::ModuleConstantFact,
    super::provenance::{record_pending_category, AbstractValue, CallableState, EscapeLane},
    join_environments, Environment, Evaluator,
};

impl Evaluator<'_, '_> {
    pub(super) fn eval_expr(&mut self, expr: &Expr, env: &mut Environment) -> AbstractValue {
        let key = self.current_key();
        self.next_index = self.next_index.saturating_add(1);
        let reference = self.expression_may_be_reference(&key);
        let value = match expr {
            Expr::Literal(_) => AbstractValue::constant(reference),
            Expr::Identifier(name) => {
                if let Some(value) = env.get(name) {
                    value.clone()
                } else {
                    let key = crate::SourceSymbolKey::new(self.definition.module_path, name);
                    match self.module_constants.get(&key) {
                        Some(ModuleConstantFact::Exact) => AbstractValue::constant(reference),
                        Some(ModuleConstantFact::Unsupported) => {
                            self.state.mark_unknown(
                                CallableProvenanceUnknownReason::UnsupportedControlFlow,
                            );
                            AbstractValue::unknown(reference)
                        }
                        None => AbstractValue::unknown(reference),
                    }
                }
            }
            Expr::DependencySourceAddress(_) => {
                self.state.effects.invokes_unknown_target = true;
                record_pending_category(&mut self.state.effects, PendingEffectCategory::Unknown);
                self.state
                    .mark_unknown(CallableProvenanceUnknownReason::UnknownCallTarget);
                AbstractValue::unknown(true)
            }
            Expr::Binary { op, left, right } => {
                let mut value = self.eval_expr(left, env);
                let right = self.eval_expr(right, env);
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
                    && value.reference
                    && right.reference
                    && (value.contains_direct_caller_reference()
                        || right.contains_direct_caller_reference())
                {
                    let mut compared = value.clone();
                    compared.join(&right);
                    self.state.record_same_heap_identity(&compared);
                }
                value.join(&right);
                value.reference = reference;
                if !reference {
                    value.caller_references.clear();
                    value.direct_caller_references.clear();
                    value.fresh_roots.clear();
                    value.fresh_references.clear();
                    value.needs_fresh_root = false;
                }
                value
            }
            Expr::Unary { expr, .. } => {
                let mut value = self.eval_expr(expr, env);
                value.reference = reference;
                if !reference {
                    value.caller_references.clear();
                    value.direct_caller_references.clear();
                    value.fresh_roots.clear();
                    value.fresh_references.clear();
                    value.needs_fresh_root = false;
                }
                value
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                let mut value = self.eval_expr(condition, env);
                value.join(&self.eval_expr(then_expr, env));
                value.join(&self.eval_expr(else_expr, env));
                value.reference = reference;
                if !reference {
                    value.caller_references.clear();
                    value.direct_caller_references.clear();
                    value.fresh_roots.clear();
                    value.fresh_references.clear();
                    value.needs_fresh_root = false;
                }
                value
            }
            Expr::Call { callee, args } => self.eval_call(&key, callee, args, env),
            Expr::Dispatch { call, timing } => {
                let start = self.next_index;
                self.eval_expr(call, env);
                if let Some(DispatchTiming::After(expr) | DispatchTiming::At(expr)) = timing {
                    self.eval_expr(expr, env);
                }
                let captured = self.values_in_range(start, self.next_index);
                self.state.record_escape(&captured, EscapeLane::Dispatch);
                record_pending_category(&mut self.state.effects, PendingEffectCategory::Unknown);
                AbstractValue::unknown(true)
            }
            Expr::Generic { callee, .. } => self.eval_expr(callee, env),
            Expr::InterfaceBox { value, .. } => {
                let mut value = self.eval_expr(value, env);
                value.reference = true;
                self.state.record_escape(&value, EscapeLane::Callback);
                value
            }
            Expr::Field { object, field } => {
                let value = self.eval_expr(object, env);
                if let Some(field_value) = value.catch_field(field, reference) {
                    field_value
                } else {
                    let Ok(path) = ValueProjectionPath::field(field.clone()) else {
                        self.state.join(&CallableState::fail_closed(
                            CallableProvenanceUnknownReason::UnsupportedHeapStore,
                        ));
                        return AbstractValue::unknown(reference);
                    };
                    let mut value = self.project_value(&value, &path, reference, true);
                    if reference
                        && value
                            .origins
                            .contains(&super::super::provenance::Origin::Fresh)
                        && value.fresh_roots.is_empty()
                    {
                        let local_candidate = self.allocate_fresh_container(
                            key.preorder_index(),
                            AbstractValue::default(),
                        );
                        value.join(&local_candidate);
                    }
                    value
                }
            }
            Expr::Index { object, index } => {
                let value = self.eval_expr(object, env);
                let _selector = self.eval_expr(index, env);
                self.project_value(
                    &value,
                    &ValueProjectionPath::container_element(),
                    reference,
                    true,
                )
            }
            Expr::Record { fields, .. } => {
                let mut value = AbstractValue::default();
                for (_, field) in fields {
                    value.join(&self.eval_expr(field, env));
                }
                self.allocate_fresh_container(key.preorder_index(), value)
            }
            Expr::ObjectLiteral { entries } => {
                let mut value = AbstractValue::default();
                for entry in entries {
                    value.join(&self.eval_expr(&entry.value, env));
                }
                self.allocate_fresh_container(key.preorder_index(), value)
            }
            Expr::Patch { operations, .. } => {
                let mut value = AbstractValue::default();
                for operation in operations {
                    let expression = match operation {
                        PatchOperation::Set { value, .. } | PatchOperation::Inc { value, .. } => {
                            value
                        }
                    };
                    value.join(&self.eval_expr(expression, env));
                }
                self.allocate_fresh_container(key.preorder_index(), value)
            }
            Expr::ValueBlock(value) => {
                let mut nested = env.clone();
                self.eval_block(&value.body, &mut nested);
                let result = self.eval_expr(&value.tail, &mut nested);
                let visible = env.keys().cloned().collect::<Vec<_>>();
                for name in visible {
                    if let Some(value) = nested.get(&name) {
                        env.insert(name, value.clone());
                    }
                }
                result
            }
            Expr::ConcurrentValue(value) => self
                .eval_concurrent_block(&value.body, Some(&value.tail), env)
                .unwrap_or_else(|| AbstractValue::unknown(reference)),
            Expr::Timeout { value, .. } => self.eval_expr(value, env),
            Expr::Throw { value } => {
                let value = self.eval_expr(value, env);
                self.state.record_wire_detached_throw(&value);
                AbstractValue::constant(false)
            }
            Expr::Rethrow { exception } => {
                let exception = self.eval_expr(exception, env);
                self.state.record_wire_detached_throw(&exception);
                AbstractValue::constant(false)
            }
            Expr::Catch { try_expr, .. } => {
                let value = self.eval_expr(try_expr, env);
                // A typed catch materializes an owner-local tagged result.
                // Effects of evaluating the try expression remain exact, but
                // the catch construct itself neither invokes an unknown target
                // nor requires caller heap identity.
                AbstractValue::catch_result(value, reference)
            }
            Expr::DbOperation(operation) => {
                let persisted = self.eval_db_operation(operation, env);
                self.state.record_persistent_escape(&persisted);
                record_pending_category(&mut self.state.effects, PendingEffectCategory::HostEffect);
                AbstractValue::fresh(reference)
            }
            Expr::DbQuery(query) => {
                self.eval_db_query(&query.query, env);
                record_pending_category(&mut self.state.effects, PendingEffectCategory::HostEffect);
                AbstractValue::fresh(reference)
            }
            Expr::DbTransaction(transaction) => {
                let mut body_env = env.clone();
                let result = match transaction.mode {
                    DbBlockMode::Effect => {
                        self.eval_block(&transaction.body, &mut body_env);
                        AbstractValue::constant(false)
                    }
                    DbBlockMode::Value => {
                        let Some((last, prefix)) = transaction.body.statements.split_last() else {
                            self.state.mark_unknown(
                                CallableProvenanceUnknownReason::UnsupportedControlFlow,
                            );
                            return AbstractValue::unknown(reference);
                        };
                        for statement in prefix {
                            self.eval_stmt(statement, &mut body_env);
                        }
                        let Stmt::Expr(result) = last else {
                            self.eval_stmt(last, &mut body_env);
                            self.state.mark_unknown(
                                CallableProvenanceUnknownReason::UnsupportedControlFlow,
                            );
                            return AbstractValue::unknown(reference);
                        };
                        self.eval_expr(result, &mut body_env)
                    }
                };
                join_environments(env, &body_env);
                record_pending_category(&mut self.state.effects, PendingEffectCategory::HostEffect);
                result
            }
            Expr::DbLeaseClaim(claim) => {
                self.eval_expr(&claim.key, env);
                let mut body_env = env.clone();
                if let Some(binding) = &claim.binding {
                    body_env.insert(binding.clone(), AbstractValue::fresh(true));
                }
                self.eval_block(&claim.body, &mut body_env);
                join_environments(env, &body_env);
                record_pending_category(&mut self.state.effects, PendingEffectCategory::HostEffect);
                AbstractValue::fresh(reference)
            }
            Expr::DbLeaseRead(read) => {
                self.eval_expr(&read.key, env);
                record_pending_category(&mut self.state.effects, PendingEffectCategory::HostEffect);
                AbstractValue::fresh(reference)
            }
        };
        self.values.insert(key.preorder_index(), value.clone());
        value
    }

    fn eval_db_operation(
        &mut self,
        operation: &DbOperation,
        env: &mut Environment,
    ) -> AbstractValue {
        let mut persisted = AbstractValue::default();
        if let Some(selector) = &operation.selector {
            self.eval_db_selector(selector, env);
        }
        if let Some(query) = operation.independent_query() {
            self.eval_db_query(query, env);
        }
        for body in [&operation.body, &operation.insert_body]
            .into_iter()
            .flatten()
        {
            match body {
                DbBody::ObjectFields { fields } => {
                    for field in fields {
                        persisted.join(&self.eval_db_write_value(&field.value, env));
                    }
                }
                DbBody::Values { value } => {
                    persisted.join(&self.eval_db_write_value(value, env));
                }
            }
        }
        if let Some(change) = &operation.change {
            for operation in &change.ops {
                match operation {
                    DbChangeOp::Set { value, .. }
                    | DbChangeOp::Inc { value, .. }
                    | DbChangeOp::AddToSet { value, .. }
                    | DbChangeOp::Remove { value, .. } => {
                        persisted.join(&self.eval_db_write_value(value, env));
                    }
                    DbChangeOp::Unset { .. } => {}
                }
            }
        }
        persisted
    }

    fn eval_db_write_value(&mut self, expression: &Expr, env: &mut Environment) -> AbstractValue {
        let mut value = self.eval_expr(expression, env);
        if self.contains_mutated_fresh_root(&value) {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnsupportedHeapStore,
            ));
            value.unknown = true;
        }
        // A statically resolved field projection is encoded into the database
        // write payload, so the stored value is detached from the source
        // object's heap graph. Keep direct caller-owned values conservative:
        // `payload = input` is still an observable database escape.
        if is_static_field_projection(expression) && !value.unknown {
            value.caller_references.clear();
            value.direct_caller_references.clear();
        }
        value
    }

    fn eval_db_selector(&mut self, selector: &DbSelector, env: &mut Environment) -> AbstractValue {
        match selector {
            DbSelector::Key { value } => self.eval_expr(value, env),
            DbSelector::Query { query } => self.eval_db_query(query, env),
        }
    }

    fn eval_db_query(&mut self, query: &DbQueryBlock, env: &mut Environment) -> AbstractValue {
        let mut inputs = AbstractValue::default();
        for clause in &query.where_clauses {
            match clause {
                DbWhereClause::Predicate { predicate } => {
                    inputs.join(&self.eval_expr(predicate, env));
                }
                DbWhereClause::Conditional {
                    condition,
                    predicate,
                } => {
                    inputs.join(&self.eval_expr(condition, env));
                    inputs.join(&self.eval_expr(predicate, env));
                }
            }
        }
        for value in [&query.limit, &query.offset, &query.after]
            .into_iter()
            .flatten()
        {
            inputs.join(&self.eval_expr(value, env));
        }
        inputs
    }
}

fn is_static_field_projection(expression: &Expr) -> bool {
    match expression {
        Expr::Field { object, .. } => is_static_projection_root(object),
        Expr::Index { object, .. } => is_static_projection_root(object),
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Ternary { .. }
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::Patch { .. }
        | Expr::ValueBlock(_)
        | Expr::ConcurrentValue(_)
        | Expr::Timeout { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Dispatch { .. } => false,
    }
}

fn is_static_projection_root(expression: &Expr) -> bool {
    match expression {
        Expr::Identifier(_) => true,
        Expr::Field { object, .. } | Expr::Index { object, .. } => {
            is_static_projection_root(object)
        }
        Expr::Literal(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Ternary { .. }
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::Patch { .. }
        | Expr::ValueBlock(_)
        | Expr::ConcurrentValue(_)
        | Expr::Timeout { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Dispatch { .. } => false,
    }
}
