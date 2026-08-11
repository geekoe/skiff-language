use std::collections::BTreeMap;

use crate::{
    shared::ast::Expr,
    writable_places::{place_from_expr, WritableRoot},
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

/// Binding writability class (writable-place model, R-195).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingKind {
    /// Local `var` binding — a writable root.
    Var,
    /// `let` bindings, ordinary parameters, loop/pattern/with bindings and
    /// top-level consts — never a writable root.
    Immutable,
    /// Actor `self` — `self.field` is a writable root.
    SelfValue,
    /// A currently valid `inout` loan parameter (callee side).
    InOutParam,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BindingEntry {
    pub(super) root: BindingRoot,
    pub(super) kind: BindingKind,
}

pub(super) type Scope = BTreeMap<String, BindingEntry>;

impl OwnerAnalyzer<'_> {
    pub(super) fn validate_mutation_target(
        &mut self,
        target: &Expr,
        scope: &Scope,
        context: ValidationContext,
    ) {
        let Some(place) = place_from_expr(target) else {
            self.diagnostic("assignment target is not an exact writable place");
            return;
        };
        let root_name = match &place.root {
            WritableRoot::InOutParam(_) => {
                if context.in_lane {
                    self.diagnostic(
                        "concurrent lane writes through an outer inout loan; inout-derived writes are forbidden",
                    );
                    return;
                }
                return;
            }
            WritableRoot::ActorSelfField(_) => {
                if context.in_lane {
                    self.diagnostic("concurrent lane writes an actor self field");
                }
                return;
            }
            WritableRoot::VarBinding(name) => name,
        };
        let entry = scope.get(root_name).copied();
        if !context.in_lane {
            match entry.map(|entry| entry.kind) {
                Some(BindingKind::Var | BindingKind::InOutParam) => return,
                Some(BindingKind::Immutable | BindingKind::SelfValue) => {
                    self.diagnostic(format!(
                        "assignment target derives from immutable binding `{root_name}`"
                    ));
                }
                None => self.diagnostic(format!(
                    "assignment target `{root_name}` has no writable local root"
                )),
            }
            return;
        }
        match entry.map(|entry| (entry.root, entry.kind)) {
            Some((BindingRoot::LaneLocalFresh, BindingKind::Var)) => {}
            Some((BindingRoot::Scalar, _)) => {
                self.diagnostic(format!(
                    "concurrent mutation target `{root_name}` is not a mutable lane-local fresh root"
                ));
            }
            Some((BindingRoot::Outer | BindingRoot::LaneLocalOpaque, BindingKind::Var))
            | Some((BindingRoot::Outer, BindingKind::InOutParam))
            | None => {
                self.diagnostic(format!(
                    "concurrent lane writes outer mutable root `{root_name}`; outer mutable root writes are forbidden"
                ))
            }
            _ => {
                self.diagnostic(format!(
                    "concurrent lane writes immutable binding `{root_name}`"
                ));
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
                self.taint_lane_local_root_from_payloads(
                    object,
                    args.iter().map(|arg| arg.expr()),
                    scope,
                    context,
                );
            }
        } else if receiver_field
            .is_some_and(|(_, field)| matches!(field, "push" | "pop" | "set" | "delete"))
        {
            // If target resolution could not prove the exact builtin, the
            // mutating spelling remains fail closed.
            if let Some((object, _)) = receiver_field {
                self.validate_mutation_target(object, scope, context);
                self.taint_lane_local_root_from_payloads(
                    object,
                    args.iter().map(|arg| arg.expr()),
                    scope,
                    context,
                );
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
        // Restored concurrent local-call gate (R-084/R-198): with the
        // aggregate alias flags retired, the only caller-visible write channel
        // is an explicit inout loan; sibling lanes may never issue inout
        // calls, so any local call in a lane that would pass an outer
        // var-derived (or inout-derived) place to an inout-taking callee is
        // rejected through the writable-place model. Callees without inout
        // parameters cannot write caller places under the new semantics.
        let callee_has_inout = self
            .inout_param_indices
            .get(source_callable)
            .is_some_and(|indices| !indices.is_empty());
        if !callee_has_inout {
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
        actuals.extend(args.iter().map(|arg| arg.expr()));
        for actual in actuals {
            let Some(place) = place_from_expr(actual) else {
                continue;
            };
            let root_name = match &place.root {
                WritableRoot::VarBinding(name) => name.as_str(),
                WritableRoot::InOutParam(_) | WritableRoot::ActorSelfField(_) => {
                    self.diagnostic(format!(
                        "concurrent local call `{}` may write outer mutable root through a loan; outer mutable root writes are forbidden",
                        source_callable.symbol()
                    ));
                    continue;
                }
            };
            let is_lane_local_fresh = scope
                .get(root_name)
                .is_some_and(|entry| entry.root == BindingRoot::LaneLocalFresh);
            if !is_lane_local_fresh {
                self.diagnostic(format!(
                    "concurrent local call `{}` may write outer mutable root `{root_name}`; outer mutable root writes are forbidden",
                    source_callable.symbol()
                ));
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
        let Some(root) = place_from_expr(target).map(|place| place.root_name().to_string()) else {
            return;
        };
        if scope.get(&root)
            != Some(&BindingEntry {
                root: BindingRoot::LaneLocalFresh,
                kind: BindingKind::Var,
            })
        {
            return;
        }
        if payloads
            .into_iter()
            .any(|payload| !lane_local_payload_is_safe(payload, scope))
        {
            scope.insert(
                root,
                BindingEntry {
                    root: BindingRoot::LaneLocalOpaque,
                    kind: BindingKind::Var,
                },
            );
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
    } else if let Some(root) = place_from_expr(expression)
        .map(|place| place.root_name().to_string())
        .and_then(|root| scope.get(&root).map(|entry| entry.root))
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
        Expr::ArrayLiteral { items } => items
            .iter()
            .all(|item| lane_local_payload_is_safe(item, scope)),
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
        | Expr::Index { .. }
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
    if let Some(root) = place_from_expr(expression)
        .map(|place| place.root_name().to_string())
        .and_then(|root| scope.get(&root).copied())
    {
        return matches!(root.root, BindingRoot::LaneLocalFresh | BindingRoot::Scalar);
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
        Expr::ArrayLiteral { items } => items
            .iter()
            .all(|item| lane_local_payload_is_safe(item, scope)),
        Expr::ValueBlock(value) | Expr::ConcurrentValue(value) => {
            lane_local_payload_is_safe(&value.tail, scope)
        }
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::Field { .. }
        | Expr::Index { .. }
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use skiff_artifact_model::{BuiltinReceiverMethod, BuiltinReceiverOp, BuiltinReceiverRoot};

    use crate::{
        expression_model::ExpressionSourceMap,
        shared::{
            ast::{Expr, FunctionDecl, Stmt},
            ast_utils::expr_path,
            parser::parse_source,
        },
        ExpressionOwnerKey, ResolvedCallTarget, ResolvedCallTargetFacts, SourceCallableEffectFacts,
        SourceSymbolKey,
    };

    use super::super::{
        collectors::expr_address,
        model::SourceExecutionSemantics,
        mutation::{BindingEntry, BindingKind, BindingRoot, Scope},
        owner::{OwnerAnalyzer, ValidationContext},
    };

    const MODULE_PATH: &str = "internal.binding_inout";

    fn fixture_function(source: &str, name: &str) -> FunctionDecl {
        let file = parse_source(source).expect("fixture should parse");
        file.functions
            .into_iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| panic!("fixture function {name} should be present"))
    }

    struct AnalyzerFacts<'a> {
        expression_sources: &'a ExpressionSourceMap,
        expression_keys: &'a BTreeMap<usize, crate::ExpressionKey>,
        resolved_targets: &'a ResolvedCallTargetFacts,
        callable_effects: &'a SourceCallableEffectFacts,
        inout_param_indices: &'a BTreeMap<SourceSymbolKey, BTreeMap<usize, String>>,
    }

    fn analyzer<'a>(
        function: &'a FunctionDecl,
        facts: AnalyzerFacts<'a>,
        semantics: &'a mut SourceExecutionSemantics,
        diagnostics: &'a mut Vec<String>,
    ) -> OwnerAnalyzer<'a> {
        OwnerAnalyzer::new(
            MODULE_PATH,
            ExpressionOwnerKey::Function("run".to_string()),
            function,
            facts.expression_sources,
            facts.expression_keys,
            facts.resolved_targets,
            facts.callable_effects,
            facts.inout_param_indices,
            BTreeSet::new(),
            semantics,
            diagnostics,
        )
    }

    fn lane_scope() -> Scope {
        BTreeMap::from([
            (
                "outer".to_string(),
                BindingEntry {
                    root: BindingRoot::Outer,
                    kind: BindingKind::Var,
                },
            ),
            (
                "loan".to_string(),
                BindingEntry {
                    root: BindingRoot::Outer,
                    kind: BindingKind::InOutParam,
                },
            ),
            (
                "laneFresh".to_string(),
                BindingEntry {
                    root: BindingRoot::LaneLocalFresh,
                    kind: BindingKind::Var,
                },
            ),
            (
                "local".to_string(),
                BindingEntry {
                    root: BindingRoot::LaneLocalFresh,
                    kind: BindingKind::Var,
                },
            ),
            (
                "locked".to_string(),
                BindingEntry {
                    root: BindingRoot::LaneLocalOpaque,
                    kind: BindingKind::Immutable,
                },
            ),
        ])
    }

    fn assign_targets(function: &FunctionDecl) -> Vec<&Expr> {
        function
            .body
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Stmt::Assign { target, .. } => Some(target),
                _ => None,
            })
            .collect()
    }

    fn call_exprs(function: &FunctionDecl) -> Vec<&Expr> {
        function
            .body
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Stmt::Expr(expr) => Some(expr),
                _ => None,
            })
            .collect()
    }

    fn call_named<'a>(function: &'a FunctionDecl, method: &str) -> &'a Expr {
        call_exprs(function)
            .into_iter()
            .find(|expr| match expr {
                Expr::Call { callee, .. } => {
                    expr_path(callee).is_some_and(|path| path.ends_with(method))
                }
                _ => false,
            })
            .unwrap_or_else(|| panic!("fixture should contain a call to {method}"))
    }

    /// R-198: a sibling lane must not write an outer captured var path, an
    /// inout-derived path or any immutable binding; only a lane-local fresh
    /// var root may be written.
    #[test]
    fn sibling_lane_writes_are_gated_on_writable_lane_local_roots() {
        let function = fixture_function(
            r#"
                function run() -> void {
                  var outer = 0
                  var loan = 0
                  var laneFresh = 0
                  let locked = 0
                  outer = 1
                  loan = 2
                  laneFresh = 3
                  locked = 4
                }
            "#,
            "run",
        );
        let targets = assign_targets(&function);
        assert_eq!(targets.len(), 4);
        let expression_sources = ExpressionSourceMap::default();
        let expression_keys: BTreeMap<usize, crate::ExpressionKey> = BTreeMap::new();
        let resolved_targets = ResolvedCallTargetFacts::empty();
        let callable_effects = SourceCallableEffectFacts::default();
        let inout_param_indices = BTreeMap::new();
        let mut semantics = SourceExecutionSemantics::default();
        let mut diagnostics = Vec::new();
        let mut analyzer = analyzer(
            &function,
            AnalyzerFacts {
                expression_sources: &expression_sources,
                expression_keys: &expression_keys,
                resolved_targets: &resolved_targets,
                callable_effects: &callable_effects,
                inout_param_indices: &inout_param_indices,
            },
            &mut semantics,
            &mut diagnostics,
        );
        let scope = lane_scope();
        let context = ValidationContext {
            in_lane: true,
            ..ValidationContext::default()
        };
        analyzer.validate_mutation_target(targets[0], &scope, context);
        analyzer.validate_mutation_target(targets[1], &scope, context);
        analyzer.validate_mutation_target(targets[2], &scope, context);
        analyzer.validate_mutation_target(targets[3], &scope, context);
        drop(analyzer);
        assert_eq!(
            diagnostics.len(),
            3,
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert!(
            diagnostics.iter().any(|d| d.contains("outer mutable root")),
            "outer var write must be rejected: {diagnostics:?}"
        );
        assert!(
            diagnostics.iter().any(|d| d.contains("`loan`")),
            "inout-derived write must be rejected: {diagnostics:?}"
        );
        assert!(
            diagnostics.iter().any(|d| d.contains("immutable binding")),
            "immutable write must be rejected: {diagnostics:?}"
        );
    }

    /// R-198: a sibling lane must never issue an inout call.
    #[test]
    fn sibling_lanes_must_not_issue_inout_calls() {
        let function = fixture_function(
            r#"
                function run() -> void {
                  var x = 1
                  inc(inout x)
                }
            "#,
            "run",
        );
        let call = call_named(&function, "inc");
        let expression_sources = ExpressionSourceMap::default();
        let expression_keys: BTreeMap<usize, crate::ExpressionKey> = BTreeMap::new();
        let resolved_targets = ResolvedCallTargetFacts::empty();
        let callable_effects = SourceCallableEffectFacts::default();
        let inout_param_indices = BTreeMap::new();
        let mut semantics = SourceExecutionSemantics::default();
        let mut diagnostics = Vec::new();
        let mut analyzer = analyzer(
            &function,
            AnalyzerFacts {
                expression_sources: &expression_sources,
                expression_keys: &expression_keys,
                resolved_targets: &resolved_targets,
                callable_effects: &callable_effects,
                inout_param_indices: &inout_param_indices,
            },
            &mut semantics,
            &mut diagnostics,
        );
        analyzer.validate_inout_call(
            call,
            args_of(call),
            &lane_scope(),
            ValidationContext {
                in_lane: true,
                ..ValidationContext::default()
            },
        );
        drop(analyzer);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.contains("sibling lane must not issue inout calls")),
            "inout call inside a sibling lane must be rejected: {diagnostics:?}"
        );
    }

    fn args_of(call: &Expr) -> &[crate::shared::ast::CallArg] {
        let Expr::Call { args, .. } = call else {
            panic!("expected a call expression");
        };
        args
    }

    /// R-198: a sibling lane may not mutate through a receiver mutator rooted
    /// at an outer var, while a lane-local fresh var receiver stays allowed.
    #[test]
    fn sibling_lane_receiver_mutators_are_gated_on_lane_local_roots() {
        let function = fixture_function(
            r#"
                function run() -> void {
                  var items = Array.empty<number>()
                  var local = Array.empty<number>()
                  items.push(1)
                  local.push(2)
                }
            "#,
            "run",
        );
        let push_op = BuiltinReceiverOp::new(
            BuiltinReceiverRoot::Array,
            BuiltinReceiverMethod::Push,
            skiff_artifact_model::RECEIVER_BUILTIN_CAPABILITY_VERSION,
        )
        .expect("Array.push op exists");
        let calls = call_exprs(&function);
        let expression_keys = calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                (
                    expr_address(call),
                    crate::ExpressionKey::new(
                        MODULE_PATH,
                        ExpressionOwnerKey::Function("run".to_string()),
                        index as u32,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let target_facts = calls
            .iter()
            .map(|call| {
                let key = expression_keys[&expr_address(call)].clone();
                (key, ResolvedCallTarget::ReceiverBuiltin { op: push_op })
            })
            .collect::<BTreeMap<_, _>>();
        let expression_sources = ExpressionSourceMap::default();
        let resolved_targets = ResolvedCallTargetFacts::from_targets(target_facts);
        let callable_effects = SourceCallableEffectFacts::default();
        let inout_param_indices = BTreeMap::new();
        let mut semantics = SourceExecutionSemantics::default();
        let mut diagnostics = Vec::new();
        let mut analyzer = analyzer(
            &function,
            AnalyzerFacts {
                expression_sources: &expression_sources,
                expression_keys: &expression_keys,
                resolved_targets: &resolved_targets,
                callable_effects: &callable_effects,
                inout_param_indices: &inout_param_indices,
            },
            &mut semantics,
            &mut diagnostics,
        );
        let scope = lane_scope();
        let context = ValidationContext {
            in_lane: true,
            ..ValidationContext::default()
        };
        for call in &calls {
            analyzer.validate_mutating_call(call, &mut scope.clone(), context);
        }
        drop(analyzer);
        assert_eq!(
            diagnostics.len(),
            1,
            "unexpected diagnostics: {diagnostics:?}"
        );
        assert!(
            diagnostics.iter().any(|d| d.contains("outer mutable root")),
            "outer-var receiver mutation must be rejected: {diagnostics:?}"
        );
    }
}
