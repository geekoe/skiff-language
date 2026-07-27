use std::collections::{BTreeMap, BTreeSet};

use crate::{
    shared::{
        ast::{DbOperationKind, Expr, Stmt},
        ast_utils::{walk_expr, walk_stmt, AstVisitor},
    },
    ExpressionKey, ResolvedCallTarget, ResolvedCallTargetFacts, SourceSymbolKey,
};

use super::collectors::{expr_address, CallableDefinition};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CallableEffectProfile {
    accesses: BTreeSet<ExternalAccess>,
    opaque: bool,
}

impl CallableEffectProfile {
    fn join(&mut self, other: &Self) {
        self.accesses.extend(other.accesses.iter().cloned());
        self.opaque |= other.opaque;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ExternalAccessMode {
    Read,
    Write,
    Exclusive,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ExternalAccess {
    target: String,
    conflict_key: String,
    mode: ExternalAccessMode,
    cancel_safe: bool,
}

impl ExternalAccess {
    fn database(target: &str, mode: ExternalAccessMode) -> Self {
        Self {
            target: format!("db:{target}"),
            conflict_key: target.to_string(),
            mode,
            // Database operations are compiler-owned and have a defined
            // response-discard/transaction terminal. Unknown host/package
            // operations never reach this constructor.
            cancel_safe: true,
        }
    }
}

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
    profiles
        .values_mut()
        .for_each(|profile| profile.opaque = true);
    profiles
}

pub(super) fn effect_profile_for_stmt(
    statement: &Stmt,
    expression_keys: &BTreeMap<usize, ExpressionKey>,
    resolved_targets: &ResolvedCallTargetFacts,
    callable_profiles: &BTreeMap<SourceSymbolKey, CallableEffectProfile>,
) -> CallableEffectProfile {
    let mut collector = EffectCollector {
        expression_keys,
        resolved_targets,
        callable_profiles,
        profile: CallableEffectProfile::default(),
    };
    collector.visit_stmt(statement);
    collector.profile
}

pub(super) fn effect_profile_for_expr(
    expression: &Expr,
    expression_keys: &BTreeMap<usize, ExpressionKey>,
    resolved_targets: &ResolvedCallTargetFacts,
    callable_profiles: &BTreeMap<SourceSymbolKey, CallableEffectProfile>,
) -> CallableEffectProfile {
    let mut collector = EffectCollector {
        expression_keys,
        resolved_targets,
        callable_profiles,
        profile: CallableEffectProfile::default(),
    };
    collector.visit_expr(expression);
    collector.profile
}

pub(super) fn validate_lane_effects(lanes: &[CallableEffectProfile]) -> Vec<String> {
    let mut diagnostics = Vec::new();
    for (lane, profile) in lanes.iter().enumerate() {
        if profile.opaque {
            diagnostics.push(format!(
                "concurrent lane {lane} has unknown target/conflict-key/cancel-safety metadata"
            ));
        }
        for access in &profile.accesses {
            if !access.cancel_safe {
                diagnostics.push(format!(
                    "concurrent lane {lane} effect {} has insufficient cancel-safety",
                    access.target
                ));
            }
        }
    }
    for left in 0..lanes.len() {
        for right in (left + 1)..lanes.len() {
            for left_access in &lanes[left].accesses {
                for right_access in &lanes[right].accesses {
                    let conflict = match (left_access.mode, right_access.mode) {
                        (ExternalAccessMode::Read, ExternalAccessMode::Read) => None,
                        (ExternalAccessMode::Exclusive, _) | (_, ExternalAccessMode::Exclusive) => {
                            Some("exclusive")
                        }
                        (ExternalAccessMode::Write, ExternalAccessMode::Write) => {
                            Some("write/write")
                        }
                        (ExternalAccessMode::Read, ExternalAccessMode::Write)
                        | (ExternalAccessMode::Write, ExternalAccessMode::Read) => {
                            Some("read/write")
                        }
                    };
                    if let Some(conflict) = conflict {
                        diagnostics.push(format!(
                            "concurrent effect conflict {conflict} between lanes {left} and {right} (targets {} / {}, conflict-keys {} / {})",
                            left_access.target,
                            right_access.target,
                            left_access.conflict_key,
                            right_access.conflict_key
                        ));
                    }
                }
            }
        }
    }
    diagnostics
}

struct EffectCollector<'a> {
    expression_keys: &'a BTreeMap<usize, ExpressionKey>,
    resolved_targets: &'a ResolvedCallTargetFacts,
    callable_profiles: &'a BTreeMap<SourceSymbolKey, CallableEffectProfile>,
    profile: CallableEffectProfile,
}

impl AstVisitor for EffectCollector<'_> {
    fn visit_stmt(&mut self, statement: &Stmt) {
        if matches!(statement, Stmt::DbTransaction { .. }) {
            self.profile.accesses.insert(ExternalAccess::database(
                "transaction",
                ExternalAccessMode::Exclusive,
            ));
        }
        walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        match expression {
            Expr::DbOperation(operation) => {
                let mode = match operation.op {
                    DbOperationKind::Find
                    | DbOperationKind::Optional
                    | DbOperationKind::Require
                    | DbOperationKind::Count
                    | DbOperationKind::Exists => ExternalAccessMode::Read,
                    DbOperationKind::Insert
                    | DbOperationKind::Update
                    | DbOperationKind::Upsert
                    | DbOperationKind::Replace
                    | DbOperationKind::Delete => ExternalAccessMode::Write,
                };
                self.profile
                    .accesses
                    .insert(ExternalAccess::database(&operation.target.name, mode));
            }
            Expr::DbQuery(query) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    &query.target.name,
                    ExternalAccessMode::Read,
                ));
            }
            Expr::DbTransaction(_) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    "transaction",
                    ExternalAccessMode::Exclusive,
                ));
            }
            Expr::DbLeaseClaim(claim) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    &claim.target.name,
                    ExternalAccessMode::Exclusive,
                ));
            }
            Expr::DbLeaseRead(read) => {
                self.profile.accesses.insert(ExternalAccess::database(
                    &read.target.name,
                    ExternalAccessMode::Read,
                ));
            }
            Expr::Call { .. } => self.collect_call(expression),
            Expr::Literal(_)
            | Expr::Identifier(_)
            | Expr::DependencySourceAddress(_)
            | Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Generic { .. }
            | Expr::InterfaceBox { .. }
            | Expr::Field { .. }
            | Expr::Record { .. }
            | Expr::ObjectLiteral { .. }
            | Expr::Patch { .. }
            | Expr::ValueBlock(_)
            | Expr::ConcurrentValue(_)
            | Expr::Timeout { .. }
            | Expr::Throw { .. }
            | Expr::Rethrow { .. }
            | Expr::Catch { .. } => {}
        }
        walk_expr(self, expression);
    }
}

impl EffectCollector<'_> {
    fn collect_call(&mut self, expression: &Expr) {
        let Some(key) = self.expression_keys.get(&expr_address(expression)) else {
            self.profile.opaque = true;
            return;
        };
        let Some(target) = self.resolved_targets.target(key) else {
            self.profile.opaque = true;
            return;
        };
        match target {
            ResolvedCallTarget::ConfigIntrinsic { .. }
            | ResolvedCallTarget::ReceiverBuiltin { .. } => {}
            ResolvedCallTarget::LocalFunction { source_callable }
            | ResolvedCallTarget::LocalImplMethod { source_callable } => {
                if let Some(profile) = self.callable_profiles.get(source_callable) {
                    self.profile.join(profile);
                } else {
                    self.profile.opaque = true;
                }
            }
            ResolvedCallTarget::ActorMethod { .. }
            | ResolvedCallTarget::NativeFunction { .. }
            | ResolvedCallTarget::DependencyPackageFunction { .. }
            | ResolvedCallTarget::InterfaceMethod { .. }
            | ResolvedCallTarget::ContractOperation { .. }
            | ResolvedCallTarget::Unknown { .. } => {
                // Current artifacts do not publish the target/conflict/cancel
                // tuple required to prove these calls concurrent-safe. The
                // source boundary therefore rejects rather than inferring from
                // a target name or a maySuspend bit.
                self.profile.opaque = true;
            }
        }
    }
}
