use std::collections::BTreeSet;

use skiff_artifact_model::{
    builtin_receiver_callable_semantics, native_callable_semantics, BoundaryCallbackContract,
    BoundaryErrorContract, BoundaryOperationDescriptor, BoundaryStreamContract, BuiltinReceiverOp,
    CallableProvenanceUnknownReason,
};

use crate::{shared::ast::Expr, ExpressionKey, ResolvedCallTarget};

use super::{
    super::provenance::{AbstractValue, CallableState, EscapeLane, Origin},
    Environment, Evaluator,
};

impl Evaluator<'_, '_> {
    pub(super) fn eval_call(
        &mut self,
        call_key: &ExpressionKey,
        callee: &Expr,
        args: &[Expr],
        env: &mut Environment,
    ) -> AbstractValue {
        let target = self.resolved_call_targets.target(call_key).cloned();
        let callee_start = self.next_index;
        let callee_value = if matches!(
            target,
            Some(ResolvedCallTarget::LocalFunction { .. })
                | Some(ResolvedCallTarget::ConfigIntrinsic { .. })
                | Some(ResolvedCallTarget::NativeFunction { .. })
                | Some(ResolvedCallTarget::DependencyPackageFunction { .. })
                | Some(ResolvedCallTarget::ContractOperation { .. })
        ) {
            self.eval_exact_non_receiver_callee(callee, env)
        } else {
            self.eval_expr(callee, env)
        };
        let receiver = receiver_object_index(callee_start, callee)
            .and_then(|index| self.value_at(index).cloned());
        let mut actuals = args
            .iter()
            .map(|arg| self.eval_expr(arg, env))
            .collect::<Vec<_>>();
        let return_reference = self.expression_may_be_reference(call_key);
        let result = match target {
            Some(ResolvedCallTarget::ConfigIntrinsic { .. }) => {
                AbstractValue::fresh(return_reference)
            }
            Some(ResolvedCallTarget::LocalFunction { .. })
            | Some(ResolvedCallTarget::LocalImplMethod { .. })
            | Some(ResolvedCallTarget::ActorMethod { .. }) => {
                let target = target.expect("matched local target");
                let Some(callee_key) = target.source_callable_key() else {
                    return self.apply_unknown_call_with_callee(
                        &callee_value,
                        &actuals,
                        return_reference,
                        EscapeLane::External,
                    );
                };
                if self
                    .definitions
                    .get(&callee_key)
                    .is_some_and(|definition| definition.has_receiver())
                {
                    actuals.insert(0, receiver.unwrap_or_else(|| AbstractValue::unknown(true)));
                }
                let callee = self.summaries.get(&callee_key).cloned().unwrap_or_else(|| {
                    if self.definitions.contains_key(&callee_key) {
                        return CallableState::bottom();
                    }
                    CallableState::fail_closed(CallableProvenanceUnknownReason::UnknownCallTarget)
                });
                self.apply_callee(&callee, &actuals, return_reference, None)
            }
            Some(ResolvedCallTarget::NativeFunction { binding_key }) => self
                .apply_exact_native_call(&binding_key, &callee_value, &actuals, return_reference),
            Some(ResolvedCallTarget::ReceiverBuiltin { op }) => {
                let Some(receiver) = receiver else {
                    return self.apply_unknown_call_with_callee(
                        &callee_value,
                        &actuals,
                        return_reference,
                        EscapeLane::Native,
                    );
                };
                self.apply_exact_receiver_call(
                    call_key,
                    op,
                    &callee_value,
                    receiver,
                    &actuals,
                    return_reference,
                )
            }
            Some(ResolvedCallTarget::DependencyPackageFunction {
                package_requirement_alias,
                package_callable_id,
                expected_local_abi,
                ..
            }) => {
                let Some(callable) = self.dependency_analysis.package_callable(
                    &package_requirement_alias,
                    &expected_local_abi,
                    &package_callable_id,
                ) else {
                    return self.apply_unknown_call_with_callee(
                        &callee_value,
                        &actuals,
                        return_reference,
                        EscapeLane::External,
                    );
                };
                let callee = CallableState::from_semantic_facts(
                    &callable.semantic_facts().effects,
                    &callable.semantic_facts().provenance,
                );
                self.apply_callee(
                    &callee,
                    &actuals,
                    return_reference,
                    Some(package_callable_id.to_string()),
                )
            }
            Some(ResolvedCallTarget::ContractOperation {
                contract_requirement,
                contract_operation_id,
                ..
            }) => {
                let Some(callee) = self
                    .dependency_analysis
                    .exact_contract_operation(&contract_requirement, &contract_operation_id)
                    .and_then(detached_contract_callee)
                else {
                    return self.apply_unknown_call_with_callee(
                        &callee_value,
                        &actuals,
                        return_reference,
                        EscapeLane::External,
                    );
                };
                self.apply_callee(&callee, &actuals, return_reference, None)
            }
            Some(ResolvedCallTarget::Unknown { .. }) | None => self.apply_unknown_call_with_callee(
                &callee_value,
                &actuals,
                return_reference,
                EscapeLane::External,
            ),
        };
        if return_reference
            && !result.unknown
            && !result.contains_caller_reference()
            && result.origins.contains(&Origin::Fresh)
            && result.fresh_roots.is_empty()
        {
            self.allocate_fresh_container(call_key.preorder_index(), result)
        } else {
            result
        }
    }

    fn eval_exact_non_receiver_callee(
        &mut self,
        callee: &Expr,
        env: &mut Environment,
    ) -> AbstractValue {
        // A non-receiver callable address is safe only as the syntactic callee
        // of the exact target attached to this call key. Every first-class
        // evaluation path still goes through `eval_expr` and remains
        // fail-closed.
        match callee {
            Expr::Identifier(_) => {
                let key = self.current_key();
                self.next_index = self.next_index.saturating_add(1);
                let value = AbstractValue::constant(self.expression_may_be_reference(&key));
                self.values.insert(key.preorder_index(), value.clone());
                value
            }
            Expr::DependencySourceAddress(_) => {
                let key = self.current_key();
                self.next_index = self.next_index.saturating_add(1);
                let value = AbstractValue::constant(self.expression_may_be_reference(&key));
                self.values.insert(key.preorder_index(), value.clone());
                value
            }
            Expr::Generic { callee, .. } => {
                let key = self.current_key();
                self.next_index = self.next_index.saturating_add(1);
                let value = self.eval_exact_non_receiver_callee(callee, env);
                self.values.insert(key.preorder_index(), value.clone());
                value
            }
            Expr::Field { object, .. } => {
                let key = self.current_key();
                self.next_index = self.next_index.saturating_add(1);
                let reference = self.expression_may_be_reference(&key);
                let mut value = self.eval_exact_non_receiver_callee(object, env);
                value.reference = reference;
                if !reference {
                    value.caller_references.clear();
                }
                self.values.insert(key.preorder_index(), value.clone());
                value
            }
            _ => self.eval_expr(callee, env),
        }
    }

    fn apply_exact_native_call(
        &mut self,
        binding_key: &str,
        callee_value: &AbstractValue,
        actuals: &[AbstractValue],
        return_reference: bool,
    ) -> AbstractValue {
        let Some(callee) = native_callable_callee(binding_key) else {
            return self.apply_unknown_call_with_callee(
                callee_value,
                actuals,
                return_reference,
                EscapeLane::Native,
            );
        };
        self.apply_callee(&callee, actuals, return_reference, None)
    }

    fn apply_exact_receiver_call(
        &mut self,
        call_key: &ExpressionKey,
        op: BuiltinReceiverOp,
        callee_value: &AbstractValue,
        receiver: AbstractValue,
        args: &[AbstractValue],
        return_reference: bool,
    ) -> AbstractValue {
        let is_map_set = op.canonical_key == "receiver:Map.set@1";
        if !is_map_set
            && matches!(
                op.canonical_key,
                "receiver:Array.push@1" | "receiver:Array.set@1" | "receiver:Map.set@1"
            )
            && args
                .iter()
                .any(|value| self.contains_mutated_fresh_root(value))
        {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnsupportedHeapStore,
            ));
            return AbstractValue::unknown(return_reference);
        }
        if is_map_set
            && (receiver.unknown
                || args.iter().any(|value| value.unknown)
                || args
                    .iter()
                    .any(|value| self.store_would_create_cycle(&receiver.fresh_roots, value)))
        {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnsupportedHeapStore,
            ));
            return AbstractValue::unknown(return_reference);
        }
        let mut actuals = Vec::with_capacity(args.len().saturating_add(1));
        actuals.push(receiver.clone());
        actuals.extend_from_slice(args);
        let Some(mut callee) = receiver_callable_callee(op) else {
            return self.apply_unknown_call_with_callee(
                callee_value,
                &actuals,
                return_reference,
                EscapeLane::Native,
            );
        };
        // Receiver mutation and heap-identity requirements are contextual to
        // the receiver graph, not to values merely embedded into that graph.
        // A fresh local JsonObject therefore discharges set's W+I facts even
        // when the inserted value originated at the caller boundary.
        if !actuals[0].contains_caller_reference() && !actuals[0].unknown {
            callee.effects.writes_caller_reachable = false;
            callee.effects.requires_same_heap_identity = false;
            callee.same_heap_identity_parameters.clear();
        }
        let mut result = self.apply_callee(&callee, &actuals, return_reference, None);
        if is_map_set && !receiver.fresh_roots.is_empty() {
            // Map entries are heap edges owned by the Map allocation, not
            // aliases to the Map object itself. Reinserting a mutated local
            // record is safe so long as the edge does not close a cycle.
            if let Some(value) = args.get(1) {
                self.store_into_fresh_roots(&receiver.fresh_roots, value);
            }
        }
        if op.canonical_key == "receiver:Map.get@1" && !receiver.fresh_roots.is_empty() {
            // A lookup may return an entry stored in the receiver, but the
            // entry is a distinct object. Keep caller-owned candidates mapped
            // through the receiver while assigning local candidates their own
            // allocation site.
            for root in &receiver.fresh_roots {
                result.fresh_roots.remove(root);
            }
            let local_candidate =
                self.allocate_fresh_container(call_key.preorder_index(), AbstractValue::default());
            result.join(&local_candidate);
        }
        result
    }

    fn apply_unknown_call_with_callee(
        &mut self,
        callee_value: &AbstractValue,
        actuals: &[AbstractValue],
        return_reference: bool,
        lane: EscapeLane,
    ) -> AbstractValue {
        let mut reachable = Vec::with_capacity(actuals.len().saturating_add(1));
        reachable.push(callee_value.clone());
        reachable.extend_from_slice(actuals);
        self.apply_unknown_call(&reachable, return_reference, lane)
    }

    fn apply_unknown_call(
        &mut self,
        actuals: &[AbstractValue],
        return_reference: bool,
        lane: EscapeLane,
    ) -> AbstractValue {
        let callee = CallableState::fail_closed(CallableProvenanceUnknownReason::UnknownCallTarget);
        let mut returned = self.apply_callee(&callee, actuals, return_reference, None);
        returned.unknown = true;
        for actual in actuals {
            self.state.record_escape(actual, lane);
        }
        returned
    }

    fn apply_callee(
        &mut self,
        callee: &CallableState,
        actuals: &[AbstractValue],
        return_reference: bool,
        dependency_return: Option<String>,
    ) -> AbstractValue {
        let any_caller_reference = actuals
            .iter()
            .any(|value| value.contains_caller_reference() || value.unknown);
        if callee.write_parameters.is_empty() && callee.parameter_stores.is_empty() {
            self.state.effects.writes_caller_reachable |=
                callee.effects.writes_caller_reachable && any_caller_reference;
        } else if callee.effects.writes_caller_reachable {
            for actual in indexed_actuals(&callee.write_parameters, actuals) {
                if actual.contains_caller_reference() || actual.unknown {
                    self.state.effects.writes_caller_reachable = true;
                    self.state
                        .write_parameters
                        .extend(actual.caller_references.iter().copied());
                }
            }
        }
        for (formal, stored) in &callee.parameter_stores {
            let Some(actual) = usize::try_from(*formal)
                .ok()
                .and_then(|index| actuals.get(index))
            else {
                self.state.join(&CallableState::fail_closed(
                    CallableProvenanceUnknownReason::UnsupportedHeapStore,
                ));
                continue;
            };
            let mapped = map_value(stored, actuals, true);
            if actual.unknown
                || mapped.unknown
                || self.store_would_create_cycle(&actual.fresh_roots, &mapped)
            {
                self.state.join(&CallableState::fail_closed(
                    CallableProvenanceUnknownReason::UnsupportedHeapStore,
                ));
            } else {
                let mut transferred = false;
                if !actual.fresh_roots.is_empty() {
                    self.store_into_fresh_roots(&actual.fresh_roots, &mapped);
                    transferred = true;
                }
                if actual.contains_caller_reference() {
                    self.state.effects.writes_caller_reachable = true;
                    self.state.effects.requires_same_heap_identity = true;
                    self.state
                        .write_parameters
                        .extend(actual.caller_references.iter().copied());
                    self.state
                        .same_heap_identity_parameters
                        .extend(actual.caller_references.iter().copied());
                    for parameter in &actual.caller_references {
                        self.state
                            .parameter_stores
                            .entry(*parameter)
                            .and_modify(|value| value.join(&mapped))
                            .or_insert_with(|| mapped.clone());
                    }
                    transferred = true;
                }
                if !transferred && (!actual.origins.is_empty() || !actual.fresh_roots.is_empty()) {
                    self.state.join(&CallableState::fail_closed(
                        CallableProvenanceUnknownReason::UnsupportedHeapStore,
                    ));
                }
            }
        }
        if callee.effects.requires_same_heap_identity {
            let identity_actuals = indexed_actuals(&callee.same_heap_identity_parameters, actuals);
            let identity_is_observable = if callee.same_heap_identity_parameters.is_empty() {
                any_caller_reference || callee.effects.invokes_unknown_target
            } else {
                identity_actuals
                    .iter()
                    .any(|actual| actual.contains_caller_reference() || actual.unknown)
            };
            self.state.effects.requires_same_heap_identity |= identity_is_observable;
            if identity_is_observable {
                let relevant_actuals = if callee.same_heap_identity_parameters.is_empty() {
                    actuals.iter().collect::<Vec<_>>()
                } else {
                    identity_actuals
                };
                for actual in relevant_actuals {
                    self.state
                        .same_heap_identity_parameters
                        .extend(actual.caller_references.iter().copied());
                }
            }
        }
        self.state.effects.invokes_unknown_target |= callee.effects.invokes_unknown_target;
        self.state.effects.may_suspend |= callee.effects.may_suspend;

        if callee.effects.escapes_caller_value {
            let lanes = if callee.escape_lanes.is_empty() {
                BTreeSet::from([EscapeLane::External])
            } else {
                callee.escape_lanes.clone()
            };
            for lane in lanes {
                if let Some(parameters) = callee.escape_parameters.get(&lane) {
                    for actual in indexed_actuals(parameters, actuals) {
                        self.state.record_escape(actual, lane);
                    }
                } else {
                    // Public/dependency summaries and unresolved effects have
                    // no internal selector identity. Preserve their aggregate
                    // effect by conservatively considering every actual.
                    for actual in actuals {
                        self.state.record_escape(actual, lane);
                    }
                }
            }
        }

        let mut returned = map_origins(
            &callee.return_origins,
            actuals,
            return_reference,
            callee.effects.returns_caller_alias,
        );
        let return_formals = caller_parameter_indices(&callee.return_origins);
        if callee.effects.returns_caller_alias && return_formals.is_empty() && any_caller_reference
        {
            for actual in actuals {
                returned
                    .caller_references
                    .extend(actual.caller_references.iter().copied());
                returned.unknown |= actual.unknown;
            }
        }
        if let Some(callable_id) = dependency_return {
            returned
                .origins
                .insert(Origin::DependencyReturn(callable_id));
        }

        let mut thrown = map_origins(
            &callee.throw_origins,
            actuals,
            true,
            callee.effects.throws_caller_alias,
        );
        if callee.effects.throws_caller_alias && any_caller_reference {
            for actual in actuals {
                thrown.join(actual);
            }
        }
        self.state
            .throw_origins
            .extend(thrown.origins.iter().cloned());
        self.state.effects.throws_caller_alias |= thrown.contains_caller_reference()
            || (callee.effects.throws_caller_alias && any_caller_reference)
            || thrown.unknown;

        if thrown.unknown {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnknownCallTarget,
            ));
        }

        if let Some(reason) = callee.unknown {
            returned.unknown |= matches!(
                reason,
                CallableProvenanceUnknownReason::UnknownCallTarget
                    | CallableProvenanceUnknownReason::AnalysisPending
            ) || callee.return_origins.is_empty();
            // Unknown return provenance is an abstract value, not an
            // unconditional callable failure. Its consumer (return, throw or
            // an escape lane) decides whether it becomes caller-visible.
            // A genuinely unresolved/dynamic call target remains fail-closed
            // through its explicit invokes_unknown_target effect.
            if callee.effects.invokes_unknown_target {
                self.state.mark_unknown(reason);
            }
        }
        returned.reference = return_reference;
        if !return_reference {
            returned.caller_references.clear();
        }
        returned
    }
}

fn native_callable_callee(binding_key: &str) -> Option<CallableState> {
    if let Some(semantics) = native_callable_semantics(binding_key) {
        let mut state = CallableState::bottom();
        state.effects = semantics.effects;
        if binding_key == "std.http.stream.emitResponse" && state.effects.escapes_caller_value {
            state.escape_lanes.insert(EscapeLane::External);
            state
                .escape_parameters
                .insert(EscapeLane::External, BTreeSet::from([0]));
        }
        state
            .return_origins
            .insert(Origin::from(semantics.return_provenance.clone()));
        return Some(state);
    }
    match binding_key {
        // runtime/native dispatch decodes into the current request heap and
        // either returns that newly materialized value or raises a detached
        // std.json.DecodeError. It does not retain the input or suspend.
        "std.json.decode" => {
            let mut state = CallableState::bottom();
            state.return_origins.insert(Origin::Fresh);
            Some(state)
        }
        _ => None,
    }
}

fn receiver_callable_callee(op: BuiltinReceiverOp) -> Option<CallableState> {
    if let Some(semantics) = builtin_receiver_callable_semantics(op) {
        let mut state = CallableState::bottom();
        state.effects = semantics.effects;
        if state.effects.writes_caller_reachable {
            state.write_parameters.insert(0);
        }
        if state.effects.requires_same_heap_identity {
            state.same_heap_identity_parameters.insert(0);
        }
        state
            .return_origins
            .insert(Origin::from(semantics.return_provenance.clone()));
        return Some(state);
    }
    let mut state = CallableState::bottom();
    match op.canonical_key {
        "receiver:string.length@1" => {
            state.return_origins.insert(Origin::Constant);
        }
        "receiver:bytes.toUtf8String@1" => {
            state.return_origins.insert(Origin::Fresh);
        }
        _ => return None,
    }
    Some(state)
}

fn detached_contract_callee(operation: &BoundaryOperationDescriptor) -> Option<CallableState> {
    let contract = &operation.contract;
    let guarantee = contract.effect_guarantee;
    if !matches!(contract.stream, BoundaryStreamContract::Unary)
        || !matches!(contract.callbacks, BoundaryCallbackContract::None)
        || !matches!(contract.errors, BoundaryErrorContract::None)
        || !guarantee.detached_parameters
        || !guarantee.detached_return
        || !guarantee.detached_error
        || !guarantee.no_caller_reachable_mutation
        || !guarantee.no_caller_value_escape
        || !guarantee.no_same_heap_identity
    {
        return None;
    }
    let mut state = CallableState::bottom();
    state.effects.may_suspend = contract.may_suspend;
    state.return_origins.insert(Origin::Fresh);
    Some(state)
}

fn map_origins(
    origins: &std::collections::BTreeSet<Origin>,
    actuals: &[AbstractValue],
    reference: bool,
    preserve_caller_references: bool,
) -> AbstractValue {
    let mut mapped = AbstractValue {
        reference,
        ..AbstractValue::default()
    };
    for origin in origins {
        match origin {
            Origin::CallerParameter(index) => {
                let Some(actual) = usize::try_from(*index)
                    .ok()
                    .and_then(|index| actuals.get(index))
                else {
                    mapped.unknown = true;
                    continue;
                };
                let mut actual = actual.clone();
                if !preserve_caller_references {
                    actual.caller_references.clear();
                }
                mapped.join(&actual);
            }
            Origin::Fresh | Origin::Constant | Origin::DependencyReturn(_) => {
                mapped.origins.insert(origin.clone());
            }
        }
    }
    mapped.reference = reference;
    mapped
}

fn map_value(
    value: &AbstractValue,
    actuals: &[AbstractValue],
    preserve_caller_references: bool,
) -> AbstractValue {
    let mut mapped = map_origins(
        &value.origins,
        actuals,
        value.reference,
        preserve_caller_references,
    );
    mapped.unknown |= value.unknown;
    mapped
}

fn caller_parameter_indices(
    origins: &std::collections::BTreeSet<Origin>,
) -> std::collections::BTreeSet<u32> {
    origins
        .iter()
        .filter_map(|origin| match origin {
            Origin::CallerParameter(index) => Some(*index),
            _ => None,
        })
        .collect()
}

fn indexed_actuals<'a>(
    indices: &std::collections::BTreeSet<u32>,
    actuals: &'a [AbstractValue],
) -> Vec<&'a AbstractValue> {
    indices
        .iter()
        .filter_map(|index| usize::try_from(*index).ok())
        .filter_map(|index| actuals.get(index))
        .collect()
}

fn receiver_object_index(callee_start: u32, mut callee: &Expr) -> Option<u32> {
    let mut index = callee_start;
    while let Expr::Generic { callee: inner, .. } = callee {
        index = index.saturating_add(1);
        callee = inner;
    }
    matches!(callee, Expr::Field { .. }).then(|| index.saturating_add(1))
}
