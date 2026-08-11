//! Inout static semantics (R-198, design §3.1).
//!
//! Runs on the execution-semantics pass where exact call targets and callee
//! effect facts are available:
//!
//! - an actual at an inout parameter position must be written `inout <place>`
//!   and the place must derive from a local `var` binding (exact path);
//! - the callee must be an exact package-local or package-direct target;
//! - the callee must be `NoPending` (the loan must not cross a Pending);
//! - loans are exclusive: overlapping places are rejected, and a loaned place
//!   cannot be read/written/captured elsewhere in the same argument list;
//! - sibling lanes never issue inout calls.

use crate::{
    shared::ast::{CallArg, Expr},
    writable_places::{place_from_expr, selectors_from_expr, WritablePlace, WritableRoot},
    ResolvedCallTarget,
};

use super::{
    mutation::{BindingKind, Scope},
    owner::{OwnerAnalyzer, ValidationContext},
};

impl OwnerAnalyzer<'_> {
    /// Validates one call whose arguments may include `inout <place>`.
    pub(super) fn validate_inout_call(
        &mut self,
        call: &Expr,
        args: &[CallArg],
        scope: &Scope,
        context: ValidationContext,
    ) {
        let Expr::Call { callee, .. } = call else {
            return;
        };
        let inout_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| matches!(arg, CallArg::InOutPlace { .. }))
            .map(|(index, _)| index)
            .collect();
        if inout_positions.is_empty() {
            return;
        }
        if context.in_lane {
            self.diagnostic("sibling lane must not issue inout calls");
            return;
        }
        let target = self.resolved_target(call).cloned();
        let callee_path =
            crate::shared::ast_utils::expr_path(callee).unwrap_or_else(|| "<callee>".to_string());
        let (declared_inout, may_pending) = match target.as_ref() {
            Some(ResolvedCallTarget::LocalFunction {
                source_callable, ..
            })
            | Some(ResolvedCallTarget::LocalImplMethod {
                source_callable, ..
            }) => {
                let declared = self
                    .inout_param_indices
                    .get(source_callable)
                    .cloned()
                    .unwrap_or_default();
                let may_pending = match self.callable_effects.operations().get(source_callable) {
                    Some(skiff_artifact_model::CallableEffectSummary::Analyzed { effects }) => {
                        effects.may_pending
                    }
                    Some(skiff_artifact_model::CallableEffectSummary::Unknown { .. }) | None => {
                        true
                    }
                };
                (declared, may_pending)
            }
            Some(ResolvedCallTarget::DependencyPackageFunction {
                inout_parameters,
                exact_signature,
                ..
            }) => (inout_parameters.clone(), {
                // The exact signature carries the File IR suspension channel
                // (mirrors callable_effects transfer).
                exact_signature
                    .as_ref()
                    .map(|signature| signature.may_suspend)
                    .unwrap_or(true)
            }),
            _ => {
                for index in &inout_positions {
                    self.diagnostic(format!(
                        "inout only allowed on exact package-local or package-direct targets; argument {index} of `{callee_path}` targets a dynamic or non-exact callee"
                    ));
                }
                return;
            }
        };

        // Argument placement: every inout-position actual must be `inout
        // <place>`; every `inout <place>` actual must sit on a declared inout
        // parameter.
        let mut loans: Vec<WritablePlace> = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let is_inout_arg = matches!(arg, CallArg::InOutPlace { .. });
            let declared = declared_inout.contains_key(&index);
            match (is_inout_arg, declared) {
                (true, false) => {
                    self.diagnostic(format!(
                        "inout argument at position {index} does not match an inout parameter of `{callee_path}`"
                    ));
                }
                (false, true) => {
                    let name = declared_inout
                        .get(&index)
                        .map(String::as_str)
                        .unwrap_or("<unnamed>");
                    self.diagnostic(format!(
                        "inout argument required for parameter `{name}` of `{callee_path}`"
                    ));
                }
                _ => {}
            }
        }
        if may_pending {
            self.diagnostic(format!(
                "inout call requires a NoPending callee; `{callee_path}` may suspend"
            ));
        }
        let mut rejected_loan = false;
        for index in &inout_positions {
            let CallArg::InOutPlace { expr } = &args[*index] else {
                continue;
            };
            let Some(place) = place_from_expr(expr) else {
                self.diagnostic(
                    "inout argument must be a writable place derived from a local `var`",
                );
                rejected_loan = true;
                continue;
            };
            match &place.root {
                WritableRoot::VarBinding(name) => {
                    let writable = scope
                        .get(name)
                        .is_some_and(|entry| entry.kind == BindingKind::Var);
                    if !writable {
                        self.diagnostic(
                            "inout argument must be a writable place derived from a local `var`",
                        );
                        rejected_loan = true;
                        continue;
                    }
                }
                WritableRoot::ActorSelfField(_) | WritableRoot::InOutParam(_) => {
                    self.diagnostic(
                        "inout argument must be a writable place derived from a local `var`",
                    );
                    rejected_loan = true;
                    continue;
                }
            }
            if loans
                .iter()
                .any(|existing| existing.overlaps(&place) || place.overlaps(existing))
            {
                self.diagnostic("overlapping inout arguments");
                rejected_loan = true;
                continue;
            }
            loans.push(place);
        }
        if rejected_loan {
            // Keep reporting the remaining structural violations without
            // cascading loan-use diagnostics on invalid loans.
            return;
        }
        // Loan exclusivity: while the loans are active, no other argument may
        // read, write or capture a loaned place (use-after-move).
        for (index, arg) in args.iter().enumerate() {
            if matches!(arg, CallArg::InOutPlace { .. }) {
                continue;
            }
            if let Some(reason) = Self::loaned_place_use(arg.expr(), &loans) {
                self.diagnostic(format!(
                    "inout place is exclusively loaned; argument {index} of `{callee_path}` {reason}"
                ));
            }
        }
    }

    /// Returns a description when `expr` references any loaned place.
    fn loaned_place_use(expr: &Expr, loans: &[WritablePlace]) -> Option<&'static str> {
        let mut found: Option<&'static str> = None;
        Self::walk_expr_for_loans(expr, loans, &mut found);
        found
    }

    fn walk_expr_for_loans(expr: &Expr, loans: &[WritablePlace], found: &mut Option<&'static str>) {
        if found.is_some() {
            return;
        }
        if let Some(selectors) = selectors_from_expr(expr) {
            let root_name = expr_root_name(expr);
            if let Some(root_name) = root_name {
                let read = WritablePlace {
                    root: WritableRoot::VarBinding(root_name.to_string()),
                    path: selectors,
                };
                for loan in loans {
                    if loan.overlaps(&read) {
                        *found = Some("reads or writes the loaned place");
                        return;
                    }
                }
            }
        }
        match expr {
            Expr::Identifier(_)
            | Expr::Literal(_)
            | Expr::DependencySourceAddress(_)
            | Expr::Field { .. }
            | Expr::Generic { .. } => {}
            Expr::Index { index, .. } => {
                // The object is already represented by the complete place
                // comparison above. The selector is an independently
                // evaluated expression and may itself use another loan.
                Self::walk_expr_for_loans(index, loans, found);
            }
            Expr::Binary { left, right, .. } => {
                Self::walk_expr_for_loans(left, loans, found);
                Self::walk_expr_for_loans(right, loans, found);
            }
            Expr::Unary { expr, .. } | Expr::InterfaceBox { value: expr, .. } => {
                Self::walk_expr_for_loans(expr, loans, found);
            }
            Expr::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::walk_expr_for_loans(condition, loans, found);
                Self::walk_expr_for_loans(then_expr, loans, found);
                Self::walk_expr_for_loans(else_expr, loans, found);
            }
            Expr::Call { callee, args } => {
                Self::walk_expr_for_loans(callee, loans, found);
                for arg in args {
                    Self::walk_expr_for_loans(arg.expr(), loans, found);
                }
            }
            Expr::Record { fields, .. } => {
                for (_, value) in fields {
                    Self::walk_expr_for_loans(value, loans, found);
                }
            }
            Expr::ObjectLiteral { entries } => {
                for entry in entries {
                    Self::walk_expr_for_loans(&entry.value, loans, found);
                }
            }
            Expr::MapLiteral { entries } => {
                for entry in entries {
                    Self::walk_expr_for_loans(&entry.value, loans, found);
                }
            }
            Expr::ArrayLiteral { items } => {
                for item in items {
                    Self::walk_expr_for_loans(item, loans, found);
                }
            }
            Expr::Patch { operations, .. } => {
                for operation in operations {
                    match operation {
                        crate::shared::ast::PatchOperation::Set { value, .. }
                        | crate::shared::ast::PatchOperation::Inc { value, .. } => {
                            Self::walk_expr_for_loans(value, loans, found);
                        }
                    }
                }
            }
            Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
                Self::walk_expr_for_loans(&value.tail, loans, found);
            }
            Expr::Timeout { value, .. }
            | Expr::Throw { value }
            | Expr::Rethrow { exception: value }
            | Expr::Catch {
                try_expr: value, ..
            } => {
                Self::walk_expr_for_loans(value, loans, found);
            }
            Expr::DbOperation(_)
            | Expr::DbQuery(_)
            | Expr::DbTransaction(_)
            | Expr::DbLeaseClaim(_)
            | Expr::DbLeaseRead(_)
            | Expr::Dispatch { .. } => {}
        }
    }
}

fn expr_root_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Identifier(name) => Some(name),
        Expr::Field { object, .. }
        | Expr::Index { object, .. }
        | Expr::Generic { callee: object, .. } => expr_root_name(object),
        Expr::Literal(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Ternary { .. }
        | Expr::Call { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::MapLiteral { .. }
        | Expr::ArrayLiteral { .. }
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
        | Expr::Dispatch { .. } => None,
    }
}
