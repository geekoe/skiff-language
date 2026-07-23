use skiff_artifact_model::CallableProvenanceUnknownReason;

use crate::shared::ast::{
    BinaryOp, DbBody, DbChangeOp, DbOperation, DbQueryBlock, DbSelector, DbWhereClause, Expr,
    PatchOperation,
};

use super::{
    super::provenance::{all_effects, join_effects, AbstractValue, EscapeLane},
    join_environments, Environment, Evaluator,
};

impl Evaluator<'_, '_> {
    pub(super) fn eval_expr(&mut self, expr: &Expr, env: &mut Environment) -> AbstractValue {
        let key = self.current_key();
        self.next_index = self.next_index.saturating_add(1);
        let reference = self.expression_may_be_reference(&key);
        let value = match expr {
            Expr::Literal(_) => AbstractValue::constant(reference),
            Expr::Identifier(name) => env
                .get(name)
                .cloned()
                .unwrap_or_else(|| AbstractValue::constant(reference)),
            Expr::DependencySourceAddress(_) => {
                self.state.effects.requires_same_heap_identity = true;
                self.state.effects.invokes_unknown_target = true;
                self.state.effects.may_suspend = true;
                self.state
                    .mark_unknown(CallableProvenanceUnknownReason::UnknownCallTarget);
                AbstractValue::unknown(true)
            }
            Expr::Binary { op, left, right } => {
                let mut value = self.eval_expr(left, env);
                let right = self.eval_expr(right, env);
                if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
                    && (value.contains_caller_reference() || right.contains_caller_reference())
                {
                    self.state.effects.requires_same_heap_identity = true;
                }
                value.join(&right);
                value.reference = reference;
                if !reference {
                    value.caller_references.clear();
                }
                value
            }
            Expr::Unary { expr, .. } => {
                let mut value = self.eval_expr(expr, env);
                value.reference = reference;
                if !reference {
                    value.caller_references.clear();
                }
                value
            }
            Expr::Call { callee, args } => self.eval_call(&key, callee, args, env),
            Expr::Generic { callee, .. } => self.eval_expr(callee, env),
            Expr::InterfaceBox { value, .. } => {
                let mut value = self.eval_expr(value, env);
                value.reference = true;
                self.state.effects.requires_same_heap_identity = true;
                self.state.record_escape(&value, EscapeLane::Callback);
                value
            }
            Expr::Field { object, .. } => {
                let mut value = self.eval_expr(object, env);
                value.reference = reference;
                if !reference {
                    value.caller_references.clear();
                }
                value
            }
            Expr::Record { fields, .. } => {
                let mut value = AbstractValue::default();
                for (_, field) in fields {
                    value.join(&self.eval_expr(field, env));
                }
                value.with_fresh_container(true)
            }
            Expr::ObjectLiteral { entries } => {
                let mut value = AbstractValue::default();
                for entry in entries {
                    value.join(&self.eval_expr(&entry.value, env));
                }
                value.with_fresh_container(true)
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
                value.with_fresh_container(true)
            }
            Expr::Throw { value } => {
                let value = self.eval_expr(value, env);
                self.state.record_throw(&value);
                AbstractValue::constant(false)
            }
            Expr::Rethrow { exception } => {
                self.eval_expr(exception, env);
                self.mark_unsupported_control_flow();
                AbstractValue::unknown(reference)
            }
            Expr::Catch { try_expr, .. } => {
                let value = self.eval_expr(try_expr, env);
                // A typed catch materializes an owner-local tagged result.
                // Effects of evaluating the try expression remain exact, but
                // the catch construct itself neither invokes an unknown target
                // nor requires caller heap identity.
                value.with_fresh_container(reference)
            }
            Expr::DbOperation(operation) => {
                let inputs = self.eval_db_operation(operation, env);
                self.state.record_escape(&inputs, EscapeLane::Database);
                self.state.effects.may_suspend = true;
                AbstractValue::fresh(reference)
            }
            Expr::DbQuery(query) => {
                let inputs = self.eval_db_query(&query.query, env);
                self.state.record_escape(&inputs, EscapeLane::Database);
                self.state.effects.may_suspend = true;
                AbstractValue::fresh(reference)
            }
            Expr::DbTransaction(transaction) => {
                let mut body_env = env.clone();
                self.eval_block(&transaction.body, &mut body_env);
                join_environments(env, &body_env);
                self.state.effects.may_suspend = true;
                self.state
                    .mark_unknown(CallableProvenanceUnknownReason::UnsupportedControlFlow);
                AbstractValue::unknown(reference)
            }
            Expr::DbLeaseClaim(claim) => {
                let key_value = self.eval_expr(&claim.key, env);
                self.state.record_escape(&key_value, EscapeLane::Database);
                let mut body_env = env.clone();
                if let Some(binding) = &claim.binding {
                    body_env.insert(binding.clone(), AbstractValue::fresh(true));
                }
                self.eval_block(&claim.body, &mut body_env);
                join_environments(env, &body_env);
                self.state.effects.may_suspend = true;
                AbstractValue::fresh(reference)
            }
            Expr::DbLeaseRead(read) => {
                let key_value = self.eval_expr(&read.key, env);
                self.state.record_escape(&key_value, EscapeLane::Database);
                self.state.effects.may_suspend = true;
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
        let mut inputs = AbstractValue::default();
        if let Some(selector) = &operation.selector {
            inputs.join(&self.eval_db_selector(selector, env));
        }
        if let Some(query) = operation.independent_query() {
            inputs.join(&self.eval_db_query(query, env));
        }
        for body in [&operation.body, &operation.insert_body]
            .into_iter()
            .flatten()
        {
            match body {
                DbBody::ObjectFields { fields } => {
                    for field in fields {
                        inputs.join(&self.eval_expr(&field.value, env));
                    }
                }
                DbBody::Values { value } => inputs.join(&self.eval_expr(value, env)),
            }
        }
        if let Some(change) = &operation.change {
            for operation in &change.ops {
                match operation {
                    DbChangeOp::Set { value, .. }
                    | DbChangeOp::Inc { value, .. }
                    | DbChangeOp::AddToSet { value, .. }
                    | DbChangeOp::Remove { value, .. } => {
                        inputs.join(&self.eval_expr(value, env));
                    }
                    DbChangeOp::Unset { .. } => {}
                }
            }
        }
        inputs
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

    fn mark_unsupported_control_flow(&mut self) {
        join_effects(&mut self.state.effects, &all_effects());
        self.state
            .mark_unknown(CallableProvenanceUnknownReason::UnsupportedControlFlow);
    }
}
