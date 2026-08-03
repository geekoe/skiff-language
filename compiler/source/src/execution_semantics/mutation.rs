use std::collections::BTreeMap;

use skiff_artifact_model::CallableEffectSummary;

use crate::{
    shared::{ast::Expr, ast_utils::expr_path},
    ResolvedCallTarget,
};

use super::{
    collectors::{receiver_builtin_spec, without_generic},
    owner::{OwnerAnalyzer, ValidationContext},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingRoot {
    Outer,
    LaneLocalFresh,
    LaneLocalOpaque,
    Scalar,
}

pub(super) type Scope = BTreeMap<String, BindingRoot>;

impl OwnerAnalyzer<'_> {
    pub(super) fn validate_mutation_target(
        &mut self,
        target: &Expr,
        scope: &Scope,
        context: ValidationContext,
    ) {
        if !context.in_lane {
            return;
        }
        let Some(root) =
            expr_path(target).and_then(|path| path.split('.').next().map(str::to_string))
        else {
            self.diagnostic("concurrent mutation target has opaque root provenance");
            return;
        };
        match scope.get(&root) {
            Some(BindingRoot::LaneLocalFresh) => {}
            Some(BindingRoot::Scalar) => {
                self.diagnostic(format!(
                    "concurrent mutation target `{root}` is not a mutable lane-local fresh root"
                ));
            }
            Some(BindingRoot::Outer | BindingRoot::LaneLocalOpaque) | None => {
                self.diagnostic(format!(
                    "concurrent lane writes outer mutable root `{root}`; outer mutable root writes are forbidden"
                ))
            }
        }
    }

    pub(super) fn validate_mutating_call(
        &mut self,
        call: &Expr,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        if !context.in_lane {
            return;
        }
        let Expr::Call { callee, args } = call else {
            return;
        };
        let resolved_target = self.resolved_target(call).cloned();
        let receiver_field = match without_generic(callee) {
            Expr::Field { object, field } => Some((object.as_ref(), field.as_str())),
            _ => None,
        };
        let mutates = resolved_target
            .as_ref()
            .and_then(|target| match target {
                ResolvedCallTarget::ReceiverBuiltin { op } => Some(*op),
                _ => None,
            })
            .and_then(receiver_builtin_spec)
            .is_some_and(|spec| spec.mutates_receiver);
        if mutates {
            if let Some((object, _)) = receiver_field {
                self.validate_mutation_target(object, scope, context);
                self.taint_lane_local_root_from_payloads(object, args.iter(), scope, context);
            }
        } else if receiver_field
            .is_some_and(|(_, field)| matches!(field, "push" | "pop" | "set" | "delete"))
        {
            // If target resolution could not prove the exact builtin, the
            // mutating spelling remains fail closed.
            if let Some((object, _)) = receiver_field {
                self.validate_mutation_target(object, scope, context);
                self.taint_lane_local_root_from_payloads(object, args.iter(), scope, context);
            }
        }

        let source_callable = match resolved_target.as_ref() {
            Some(ResolvedCallTarget::LocalFunction {
                source_callable, ..
            })
            | Some(ResolvedCallTarget::LocalImplMethod {
                source_callable, ..
            }) => source_callable,
            _ => return,
        };
        let writes_caller_reachable = match self.callable_effects.operations().get(source_callable)
        {
            Some(CallableEffectSummary::Analyzed { effects }) => effects.writes_caller_reachable,
            Some(CallableEffectSummary::Unknown { .. }) | None => {
                self.diagnostic(format!(
                    "concurrent local call `{}` has unknown caller-reachable mutation facts",
                    source_callable.symbol()
                ));
                return;
            }
        };
        if !writes_caller_reachable {
            return;
        }

        let mut actuals = Vec::with_capacity(args.len() + usize::from(receiver_field.is_some()));
        if matches!(
            resolved_target,
            Some(ResolvedCallTarget::LocalImplMethod { .. })
        ) {
            if let Some((receiver, _)) = receiver_field {
                actuals.push(receiver);
            }
        }
        actuals.extend(args);
        for actual in actuals {
            match binding_root_for_value(actual, scope, true, true) {
                BindingRoot::LaneLocalFresh | BindingRoot::Scalar => {}
                BindingRoot::Outer => {
                    let root = expr_path(actual)
                        .and_then(|path| path.split('.').next().map(str::to_string))
                        .unwrap_or_else(|| "<opaque>".to_string());
                    self.diagnostic(format!(
                        "concurrent local call `{}` may write outer mutable root `{root}`; outer mutable root writes are forbidden",
                        source_callable.symbol()
                    ));
                }
                BindingRoot::LaneLocalOpaque => {
                    self.diagnostic(format!(
                        "concurrent local call `{}` has caller-reachable mutation through a non-fresh lane value",
                        source_callable.symbol()
                    ));
                }
            }
        }
    }

    pub(super) fn taint_lane_local_root_from_payloads<'expr>(
        &mut self,
        target: &Expr,
        payloads: impl IntoIterator<Item = &'expr Expr>,
        scope: &mut Scope,
        context: ValidationContext,
    ) {
        if !context.in_lane {
            return;
        }
        let Some(root) =
            expr_path(target).and_then(|path| path.split('.').next().map(str::to_string))
        else {
            return;
        };
        if scope.get(&root) != Some(&BindingRoot::LaneLocalFresh) {
            return;
        }
        if payloads
            .into_iter()
            .any(|payload| !lane_local_payload_is_safe(payload, scope))
        {
            scope.insert(root, BindingRoot::LaneLocalOpaque);
        }
    }
}

pub(super) fn binding_root_for_value(
    expression: &Expr,
    scope: &Scope,
    in_lane: bool,
    mutable: bool,
) -> BindingRoot {
    if !in_lane {
        return BindingRoot::Outer;
    }
    if definitely_lane_local_fresh(expression, scope) {
        BindingRoot::LaneLocalFresh
    } else if let Some(root) = expr_path(expression)
        .and_then(|path| path.split('.').next().map(str::to_string))
        .and_then(|root| scope.get(&root).copied())
    {
        root
    } else if !mutable && matches!(expression, Expr::Literal(_)) {
        BindingRoot::Scalar
    } else {
        BindingRoot::LaneLocalOpaque
    }
}

fn definitely_lane_local_fresh(expression: &Expr, scope: &Scope) -> bool {
    match expression {
        Expr::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            lane_local_payload_is_safe(condition, scope)
                && lane_local_payload_is_safe(then_expr, scope)
                && lane_local_payload_is_safe(else_expr, scope)
        }
        Expr::Record { fields, .. } => fields
            .iter()
            .all(|(_, value)| lane_local_payload_is_safe(value, scope)),
        Expr::ObjectLiteral { entries } => entries
            .iter()
            .all(|entry| lane_local_payload_is_safe(&entry.value, scope)),
        Expr::Patch { operations, .. } => operations.iter().all(|operation| match operation {
            crate::shared::ast::PatchOperation::Set { value, .. }
            | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                lane_local_payload_is_safe(value, scope)
            }
        }),
        Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
            definitely_lane_local_fresh(&value.tail, scope)
        }
        Expr::Timeout { value, .. } => definitely_lane_local_fresh(value, scope),
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Field { .. }
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

fn lane_local_payload_is_safe(expression: &Expr, scope: &Scope) -> bool {
    if definitely_lane_local_fresh(expression, scope) {
        return true;
    }
    if matches!(expression, Expr::Literal(_)) {
        return true;
    }
    if let Some(root) = expr_path(expression)
        .and_then(|path| path.split('.').next().map(str::to_string))
        .and_then(|root| scope.get(&root).copied())
    {
        return matches!(root, BindingRoot::LaneLocalFresh | BindingRoot::Scalar);
    }
    match expression {
        Expr::Binary { left, right, .. } => {
            lane_local_payload_is_safe(left, scope) && lane_local_payload_is_safe(right, scope)
        }
        Expr::Unary { expr, .. } | Expr::InterfaceBox { value: expr, .. } => {
            lane_local_payload_is_safe(expr, scope)
        }
        Expr::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            lane_local_payload_is_safe(condition, scope)
                && lane_local_payload_is_safe(then_expr, scope)
                && lane_local_payload_is_safe(else_expr, scope)
        }
        Expr::Timeout { value, .. } => lane_local_payload_is_safe(value, scope),
        Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
            lane_local_payload_is_safe(&value.tail, scope)
        }
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::Field { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::Patch { .. }
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
