use std::collections::BTreeSet;

use skiff_artifact_model::{CallableProvenanceUnknownReason, ValueProjectionPath};

use crate::shared::ast::{ForBinding, Stmt};

use super::{
    super::provenance::{AbstractValue, CallableState, EscapeLane},
    join_environments, pattern_bindings, Environment, Evaluator,
};

impl Evaluator<'_, '_> {
    pub(super) fn eval_block(&mut self, block: &crate::shared::ast::Block, env: &mut Environment) {
        for statement in &block.statements {
            self.eval_stmt(statement, env);
        }
    }

    pub(super) fn eval_stmt(&mut self, statement: &Stmt, env: &mut Environment) {
        match statement {
            Stmt::CompilerTestEffectRegister { .. } => {
                // The dependency-call probe owns two expression keys but is
                // link metadata, not an executed call.
                self.next_index += 2;
                for expression in
                    crate::shared::ast_utils::compiler_test_effect_expressions(statement)
                        .expect("matched compiler test effect")
                {
                    self.eval_expr(expression, env);
                }
            }
            Stmt::Assert { condition, .. } => {
                self.eval_expr(condition, env);
            }
            Stmt::Let { name, value, .. } => {
                let value = self.eval_expr(value, env);
                env.insert(name.clone(), value);
            }
            Stmt::Assign { target, value } => {
                let _target_value = self.eval_expr(target, env);
                let assigned = self.eval_expr(value, env);
                if let crate::shared::ast::Expr::Identifier(name) = target {
                    env.insert(name.clone(), assigned);
                } else if let crate::shared::ast::Expr::Field { object, .. } = target {
                    let base = self.eval_store_base(object, env);
                    self.transfer_field_store(&base, &assigned);
                } else {
                    // The current abstract environment has no heap/points-to
                    // store transfer. Updating only the syntactic owner would
                    // be unsound through aliases (`alias = holder`) and later
                    // nested loads. Until a complete heap model exists, every
                    // post-construction container/field store poisons the whole
                    // callable instead of emitting false safe facts.
                    self.mark_unsupported_heap_store();
                }
            }
            Stmt::Timeout { body, .. } => {
                self.eval_scoped_block(body, env);
            }
            Stmt::Concurrent { body } => {
                self.eval_concurrent_block(body, None, env);
            }
            Stmt::Serial { body } => {
                self.eval_scoped_block(body, env);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                self.eval_expr(condition, env);
                let mut then_env = env.clone();
                self.eval_block(then_block, &mut then_env);
                let mut else_env = env.clone();
                if let Some(else_block) = else_block {
                    self.eval_block(else_block, &mut else_env);
                }
                join_environments(&mut then_env, &else_env);
                *env = then_env;
            }
            Stmt::For {
                binding,
                iterable,
                body,
            } => {
                let iterable = self.eval_expr(iterable, env);
                let element_value = self.project_value(
                    &iterable,
                    &ValueProjectionPath::container_element(),
                    true,
                    true,
                );
                let mut body_env = env.clone();
                match binding {
                    ForBinding::Item { item } => {
                        body_env.insert(item.clone(), element_value);
                    }
                    ForBinding::Entry { key, value } => {
                        body_env.insert(key.clone(), AbstractValue::unknown(true));
                        body_env.insert(value.clone(), element_value);
                    }
                }
                self.eval_block(body, &mut body_env);
                join_environments(env, &body_env);
                // A single AST traversal cannot prove cross-iteration mutation
                // ordering. Reject the whole provenance path instead of using a
                // one-pass false negative.
                self.mark_unsupported_loop();
            }
            Stmt::Match { value, arms } => {
                let value = self.eval_expr(value, env);
                let mut joined = env.clone();
                for arm in arms {
                    let mut arm_env = env.clone();
                    let mut names = BTreeSet::new();
                    pattern_bindings(&arm.pattern, &mut names);
                    for name in names {
                        arm_env.insert(name, value.clone());
                    }
                    self.eval_block(&arm.body, &mut arm_env);
                    join_environments(&mut joined, &arm_env);
                }
                *env = joined;
            }
            Stmt::DbTransaction { body } => {
                let mut body_env = env.clone();
                self.eval_block(body, &mut body_env);
                join_environments(env, &body_env);
                self.state.effects.may_suspend = true;
            }
            Stmt::Throw { value } => {
                let value = self.eval_expr(value, env);
                let value = self.materialize_heap_value(&value);
                self.state.record_wire_detached_throw(&value);
            }
            Stmt::Rethrow { exception } => {
                let exception = self.eval_expr(exception, env);
                let exception = self.materialize_heap_value(&exception);
                self.state.record_wire_detached_throw(&exception);
            }
            Stmt::Emit(value) => {
                let value = self.eval_expr(value, env);
                let value = self.materialize_heap_value(&value);
                self.state.record_escape(&value, EscapeLane::Stream);
                self.state.effects.may_suspend = true;
            }
            Stmt::Spawn { call } => {
                let start = self.next_index;
                self.eval_expr(call, env);
                let captured = self.values_in_range(start, self.next_index);
                self.state.record_escape(&captured, EscapeLane::Spawn);
                self.state.effects.may_suspend = true;
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    let value = self.eval_expr(value, env);
                    let value = self.materialize_heap_value(&value);
                    self.state.record_return(&value);
                }
            }
            Stmt::Expr(value) => {
                self.eval_expr(value, env);
            }
            Stmt::Break | Stmt::Continue => {
                self.mark_unsupported_loop();
            }
        }
    }

    fn mark_unsupported_loop(&mut self) {
        // Loop shape makes the value provenance incomplete, but it does not
        // invent an unknown call target or caller mutation. Effects observed
        // in the body and every explicit return/throw/escape lane were already
        // transferred above. The join environment is the loop fixed point for
        // this finite provenance lattice, so retain those exact facts.
    }

    pub(super) fn eval_scoped_block(
        &mut self,
        block: &crate::shared::ast::Block,
        env: &mut Environment,
    ) {
        let visible = env.keys().cloned().collect::<Vec<_>>();
        let mut nested = env.clone();
        self.eval_block(block, &mut nested);
        for name in visible {
            if let Some(value) = nested.get(&name) {
                env.insert(name, value.clone());
            }
        }
    }

    pub(super) fn eval_concurrent_block(
        &mut self,
        body: &crate::shared::ast::Block,
        tail: Option<&crate::shared::ast::Expr>,
        env: &mut Environment,
    ) -> Option<AbstractValue> {
        let mut sibling_env = env.clone();
        for statement in &body.statements {
            match statement {
                Stmt::Let {
                    mutable: false,
                    name,
                    value,
                    ..
                } => {
                    let value = self.eval_expr(value, &mut sibling_env);
                    sibling_env.insert(name.clone(), value);
                }
                Stmt::Serial { body } => {
                    let mut lane_env = sibling_env.clone();
                    self.eval_block(body, &mut lane_env);
                }
                _ => {
                    let mut lane_env = sibling_env.clone();
                    self.eval_stmt(statement, &mut lane_env);
                }
            }
        }
        tail.map(|tail| {
            let mut tail_env = sibling_env;
            self.eval_expr(tail, &mut tail_env)
        })
    }

    fn mark_unsupported_heap_store(&mut self) {
        self.state.join(&CallableState::fail_closed(
            CallableProvenanceUnknownReason::UnsupportedHeapStore,
        ));
    }

    fn eval_store_base(
        &self,
        object: &crate::shared::ast::Expr,
        env: &Environment,
    ) -> AbstractValue {
        match object {
            crate::shared::ast::Expr::Identifier(name) => env
                .get(name)
                .cloned()
                .unwrap_or_else(|| AbstractValue::unknown(true)),
            _ => AbstractValue::unknown(true),
        }
    }

    fn transfer_field_store(&mut self, base: &AbstractValue, assigned: &AbstractValue) {
        if base.unknown || assigned.unknown || self.store_would_create_cycle(base, assigned) {
            self.mark_unsupported_heap_store();
            return;
        }
        let mut transferred = false;
        if !base.fresh_roots.is_empty() {
            self.store_into_fresh_roots(&base.fresh_roots, assigned);
            transferred = true;
        }
        if base.contains_direct_caller_reference() {
            self.state.effects.writes_caller_reachable = true;
            self.state.write_parameters.extend(
                base.direct_caller_references
                    .iter()
                    .map(|reference| reference.parameter),
            );
            for reference in &base.direct_caller_references {
                self.state
                    .parameter_stores
                    .entry(reference.clone())
                    .and_modify(|value| value.join(assigned))
                    .or_insert_with(|| assigned.clone());
            }
            transferred = true;
        }
        if transferred {
            return;
        }
        // Local SCC seeding uses lattice bottom for a not-yet-transferred
        // callee. Do not turn that temporary absence into a sticky
        // fail-closed fact; the next fixed-point iteration will supply the
        // Fresh return root or a real unknown value.
        if base.origins.is_empty() && base.fresh_roots.is_empty() {
            return;
        }
        self.mark_unsupported_heap_store();
    }
}
