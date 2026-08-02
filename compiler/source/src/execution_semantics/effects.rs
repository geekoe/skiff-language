use std::collections::BTreeMap;

use crate::{
    shared::{
        ast::{Expr, Stmt},
        ast_utils::{walk_expr, walk_stmt, AstVisitor},
    },
    ExpressionKey, ResolvedCallTarget, ResolvedCallTargetFacts, SourceSymbolKey,
};

use super::collectors::{expr_address, CallableDefinition};

/// Owner-local execution effects needed by source validation. v1 only tracks
/// the `db:transaction` access tag; ordinary single-db operations and every
/// other access are intentionally not modeled here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CallableEffectProfile {
    pub(super) uses_db_transaction: bool,
}

/// Fixed-point profiles over the local call graph. Local callee bodies are
/// joined into callers, so an actor method profile contains every transaction
/// reachable through same-package helpers. Detached `spawn` targets and
/// non-local targets (actor methods, dependency packages, interfaces,
/// contracts, natives, builtins) never contribute: spawn bodies are not
/// actor-method code and the other bodies are the documented v1 boundary.
pub(super) fn callable_effect_profiles(
    definitions: &[CallableDefinition<'_>],
    expression_keys: &BTreeMap<usize, ExpressionKey>,
    resolved_targets: &ResolvedCallTargetFacts,
) -> BTreeMap<SourceSymbolKey, CallableEffectProfile> {
    let mut profiles = definitions
        .iter()
        .map(|definition| (definition.key.clone(), CallableEffectProfile::default()))
        .collect::<BTreeMap<_, _>>();
    for _ in 0..=definitions.len() {
        let next = definitions
            .iter()
            .map(|definition| {
                let mut collector = EffectCollector {
                    expression_keys,
                    resolved_targets,
                    callable_profiles: &profiles,
                    profile: CallableEffectProfile::default(),
                    spawn_target_call: None,
                };
                collector.visit_block(&definition.function.body);
                (definition.key.clone(), collector.profile)
            })
            .collect::<BTreeMap<_, _>>();
        if next == profiles {
            return profiles;
        }
        profiles = next;
    }
    // The join lattice is monotone, so the extra iteration above always
    // converges. Reaching here is a hard invariant failure; fail closed by
    // treating every profile as transaction-tainted.
    profiles
        .values_mut()
        .for_each(|profile| profile.uses_db_transaction = true);
    profiles
}

struct EffectCollector<'a> {
    expression_keys: &'a BTreeMap<usize, ExpressionKey>,
    resolved_targets: &'a ResolvedCallTargetFacts,
    callable_profiles: &'a BTreeMap<SourceSymbolKey, CallableEffectProfile>,
    profile: CallableEffectProfile,
    /// The call expression of the innermost `spawn` statement currently being
    /// walked. Its target body is detached; nested calls in its arguments or
    /// callee expression still execute in the caller context and are joined.
    spawn_target_call: Option<usize>,
}

impl AstVisitor for EffectCollector<'_> {
    fn visit_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::DbTransaction { .. } => {
                self.profile.uses_db_transaction = true;
                walk_stmt(self, statement);
            }
            Stmt::Spawn { call } => {
                self.spawn_target_call = Some(expr_address(call));
                walk_expr(self, call);
                self.spawn_target_call = None;
            }
            _ => walk_stmt(self, statement),
        }
    }

    fn visit_expr(&mut self, expression: &Expr) {
        match expression {
            Expr::DbTransaction(_) => self.profile.uses_db_transaction = true,
            Expr::Call { .. } if self.spawn_target_call != Some(expr_address(expression)) => {
                self.collect_call(expression);
            }
            _ => {}
        }
        walk_expr(self, expression);
    }
}

impl EffectCollector<'_> {
    fn collect_call(&mut self, expression: &Expr) {
        let Some(key) = self.expression_keys.get(&expr_address(expression)) else {
            return;
        };
        let Some(target) = self.resolved_targets.target(key) else {
            return;
        };
        match target {
            ResolvedCallTarget::LocalFunction {
                source_callable, ..
            }
            | ResolvedCallTarget::LocalImplMethod {
                source_callable, ..
            } => {
                if let Some(profile) = self.callable_profiles.get(source_callable) {
                    self.profile.uses_db_transaction |= profile.uses_db_transaction;
                }
            }
            ResolvedCallTarget::ConfigIntrinsic { .. }
            | ResolvedCallTarget::ActorMethod { .. }
            | ResolvedCallTarget::NativeFunction { .. }
            | ResolvedCallTarget::ReceiverBuiltin { .. }
            | ResolvedCallTarget::DependencyPackageFunction { .. }
            | ResolvedCallTarget::InterfaceMethod { .. }
            | ResolvedCallTarget::ContractOperation { .. }
            | ResolvedCallTarget::Unknown { .. } => {}
        }
    }
}
