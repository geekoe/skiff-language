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
            Some(ResolvedCallTarget::DependencyPackageFunction { .. })
                | Some(ResolvedCallTarget::ContractOperation { .. })
        ) {
            self.eval_exact_dependency_callee(callee, env)
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
        match target {
            Some(ResolvedCallTarget::LocalFunction { .. })
            | Some(ResolvedCallTarget::LocalImplMethod { .. }) => {
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
        }
    }

    fn eval_exact_dependency_callee(
        &mut self,
        callee: &Expr,
        env: &mut Environment,
    ) -> AbstractValue {
        // A dependency address is safe only as the syntactic callee of the
        // exact target attached to this call key. Every other evaluation path
        // still goes through `eval_expr` and retains its fail-closed transfer.
        match callee {
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
                let value = self.eval_exact_dependency_callee(callee, env);
                self.values.insert(key.preorder_index(), value.clone());
                value
            }
            Expr::Field { object, .. } => {
                let key = self.current_key();
                self.next_index = self.next_index.saturating_add(1);
                let reference = self.expression_may_be_reference(&key);
                let mut value = self.eval_exact_dependency_callee(object, env);
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
        op: BuiltinReceiverOp,
        callee_value: &AbstractValue,
        receiver: AbstractValue,
        args: &[AbstractValue],
        return_reference: bool,
    ) -> AbstractValue {
        let mut actuals = Vec::with_capacity(args.len().saturating_add(1));
        actuals.push(receiver);
        actuals.extend_from_slice(args);
        let Some(callee) = receiver_callable_callee(op) else {
            return self.apply_unknown_call_with_callee(
                callee_value,
                &actuals,
                return_reference,
                EscapeLane::Native,
            );
        };
        self.apply_callee(&callee, &actuals, return_reference, None)
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
        let any_caller_value = actuals
            .iter()
            .any(|value| value.contains_caller_value() || value.unknown);
        self.state.effects.writes_caller_reachable |=
            callee.effects.writes_caller_reachable && any_caller_reference;
        self.state.effects.requires_same_heap_identity |=
            callee.effects.requires_same_heap_identity;
        self.state.effects.invokes_unknown_target |= callee.effects.invokes_unknown_target;
        self.state.effects.may_suspend |= callee.effects.may_suspend;

        if callee.effects.escapes_caller_value && any_caller_value {
            self.state.effects.escapes_caller_value = true;
            if callee.escape_lanes.is_empty() {
                self.state.escape_lanes.insert(EscapeLane::External);
            } else {
                self.state
                    .escape_lanes
                    .extend(callee.escape_lanes.iter().copied());
            }
        }

        let mut returned = map_origins(
            &callee.return_origins,
            actuals,
            return_reference,
            callee.effects.returns_caller_alias,
        );
        if callee.effects.returns_caller_alias && any_caller_reference {
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

        if returned.unknown || thrown.unknown {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnknownCallTarget,
            ));
        }

        if let Some(reason) = callee.unknown {
            self.state.mark_unknown(reason);
            returned.unknown = true;
        }
        returned.reference = return_reference;
        if !return_reference {
            returned.caller_references.clear();
        }
        returned
    }
}

fn native_callable_callee(binding_key: &str) -> Option<CallableState> {
    let semantics = native_callable_semantics(binding_key)?;
    let mut state = CallableState::bottom();
    state.effects = semantics.effects;
    state
        .return_origins
        .insert(Origin::from(semantics.return_provenance.clone()));
    Some(state)
}

fn receiver_callable_callee(op: BuiltinReceiverOp) -> Option<CallableState> {
    let semantics = builtin_receiver_callable_semantics(op)?;
    let mut state = CallableState::bottom();
    state.effects = semantics.effects;
    state
        .return_origins
        .insert(Origin::from(semantics.return_provenance.clone()));
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

fn receiver_object_index(callee_start: u32, mut callee: &Expr) -> Option<u32> {
    let mut index = callee_start;
    while let Expr::Generic { callee: inner, .. } = callee {
        index = index.saturating_add(1);
        callee = inner;
    }
    matches!(callee, Expr::Field { .. }).then(|| index.saturating_add(1))
}
