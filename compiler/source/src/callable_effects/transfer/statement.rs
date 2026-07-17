use std::collections::BTreeSet;

use skiff_artifact_model::CallableProvenanceUnknownReason;

use crate::shared::ast::{ForBinding, Stmt};

use super::{
    super::provenance::{all_effects, join_effects, AbstractValue, CallableState, EscapeLane},
    join_environments, pattern_bindings, Environment, Evaluator,
};

impl Evaluator<'_, '_> {
    pub(super) fn eval_block(&mut self, block: &crate::shared::ast::Block, env: &mut Environment) {
        for statement in &block.statements {
            self.eval_stmt(statement, env);
        }
    }

    fn eval_stmt(&mut self, statement: &Stmt, env: &mut Environment) {
        match statement {
            Stmt::Assert { condition, .. } => {
                self.eval_expr(condition, env);
            }
            Stmt::Let { name, value, .. } => {
                let value = self.eval_expr(value, env);
                env.insert(name.clone(), value);
            }
            Stmt::Assign { target, value } => {
                self.eval_expr(target, env);
                let assigned = self.eval_expr(value, env);
                if let crate::shared::ast::Expr::Identifier(name) = target {
                    env.insert(name.clone(), assigned);
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
                let mut body_env = env.clone();
                match binding {
                    ForBinding::Item { item } => {
                        body_env.insert(item.clone(), iterable.clone());
                    }
                    ForBinding::Entry { key, value } => {
                        body_env.insert(key.clone(), AbstractValue::unknown(true));
                        body_env.insert(value.clone(), iterable.clone());
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
                self.state.record_throw(&value);
            }
            Stmt::Rethrow { exception } => {
                self.eval_expr(exception, env);
                join_effects(&mut self.state.effects, &all_effects());
                self.state
                    .mark_unknown(CallableProvenanceUnknownReason::UnsupportedControlFlow);
            }
            Stmt::Emit(value) => {
                let value = self.eval_expr(value, env);
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
        join_effects(&mut self.state.effects, &all_effects());
        self.state
            .mark_unknown(CallableProvenanceUnknownReason::UnsupportedControlFlow);
    }

    fn mark_unsupported_heap_store(&mut self) {
        self.state.join(&CallableState::fail_closed(
            CallableProvenanceUnknownReason::UnsupportedControlFlow,
        ));
    }
}
