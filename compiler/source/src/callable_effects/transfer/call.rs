use std::collections::BTreeSet;

use skiff_artifact_model::{
    builtin_receiver_callable_semantics, host_effect_registry, native_callable_semantics,
    BoundaryCallbackContract, BoundaryOperationDescriptor, BoundaryStreamContract,
    BuiltinReceiverOp, CallableMayEffects, CallableProvenanceUnknownReason,
    HostEffectExecutorIdentity, PackageCallableId, PendingEffectCategory, TypeRefIr,
    ValueProjectionPath,
};
use skiff_compiler_core::{id::SKIFF_STD_PUBLICATION_ID, public_package_callable_id};

use crate::{
    shared::ast::{CallArg, Expr},
    ExpressionKey, ResolvedCallTarget,
};

use super::{
    super::provenance::{
        record_pending_category, AbstractValue, CallableState, EscapeLane, Origin,
    },
    Environment, Evaluator,
};

impl Evaluator<'_, '_> {
    pub(super) fn eval_call(
        &mut self,
        call_key: &ExpressionKey,
        callee: &Expr,
        args: &[CallArg],
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
        let receiver_index = receiver_object_index(callee_start, callee);
        let receiver = receiver_index.and_then(|index| self.value_at(index).cloned());
        let callback_receiver =
            self.is_callback_carrier_receiver_call(call_key, callee, receiver_index);
        let mut actuals = args
            .iter()
            .map(|arg| self.eval_expr(arg.expr(), env))
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
                let previous_pending_categories =
                    if matches!(target, ResolvedCallTarget::ActorMethod { .. }) {
                        Some(self.state.effects.pending_effect_categories.clone())
                    } else {
                        None
                    };
                let result = self.apply_callee(&callee, &actuals, return_reference, None);
                if matches!(target, ResolvedCallTarget::ActorMethod { .. }) {
                    if let Some(previous) = previous_pending_categories {
                        self.state.effects.pending_effect_categories = previous;
                    }
                    record_pending_category(
                        &mut self.state.effects,
                        PendingEffectCategory::ActorCall,
                    );
                }
                result
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
                compiler_owned,
                package_callable_id,
                expected_local_abi,
                exact_signature,
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
                let mut callee = CallableState::from_semantic_facts(
                    &callable.semantic_facts().effects,
                    &callable.semantic_facts().provenance,
                );
                // The exact signature carries the File IR suspension channel; keep
                // exact pending categories when the summary knows them, and only
                // fall back to Unknown for a genuinely unspecified suspension.
                let may_pending = exact_signature
                    .as_ref()
                    .map(|signature| signature.may_suspend)
                    .unwrap_or(true);
                callee.effects.may_pending = may_pending;
                if !may_pending {
                    callee.effects.pending_effect_categories.clear();
                } else {
                    if compiler_owned {
                        project_exact_package_executor_category(
                            &mut callee.effects,
                            &package_callable_id,
                        );
                    }
                    if callee.effects.pending_effect_categories.is_empty() {
                        callee
                            .effects
                            .pending_effect_categories
                            .push(PendingEffectCategory::Unknown);
                    }
                }
                let result = self.apply_callee(
                    &callee,
                    &actuals,
                    return_reference,
                    Some(package_callable_id.to_string()),
                );
                if package_callable_id.as_str().ends_with(":std.actor.get") {
                    self.state
                        .effects
                        .pending_effect_categories
                        .retain(|category| {
                            !matches!(
                                category,
                                PendingEffectCategory::NativeCall
                                    | PendingEffectCategory::HostEffect
                            )
                        });
                    record_pending_category(
                        &mut self.state.effects,
                        PendingEffectCategory::ActorCall,
                    );
                }
                result
            }
            Some(ResolvedCallTarget::InterfaceMethod { .. }) if callback_receiver => {
                self.apply_callback_interface_call(&callee_value, &actuals, return_reference)
            }
            Some(ResolvedCallTarget::InterfaceMethod { .. }) => self
                .apply_unknown_call_with_callee(
                    &callee_value,
                    &actuals,
                    return_reference,
                    EscapeLane::External,
                ),
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
            Some(ResolvedCallTarget::RemoteInterface { .. }) => self
                .apply_unknown_call_with_callee(
                    &callee_value,
                    &actuals,
                    return_reference,
                    EscapeLane::External,
                ),
            Some(ResolvedCallTarget::Unknown { .. }) | None => self.apply_unknown_call_with_callee(
                &callee_value,
                &actuals,
                return_reference,
                EscapeLane::External,
            ),
        };
        if return_reference && !result.unknown && result.needs_fresh_root {
            // A summary says which roots are directly returned and which
            // origins are merely reachable, but it is intentionally
            // field-insensitive. Materialize the direct Fresh branch as a new
            // call-site root and retain all non-root reachability as possible
            // one-level payload. This lets a later field read recover a caller
            // alias embedded by the callee without treating that alias as the
            // wrapper's own identity.
            let mut payload = result.clone();
            payload.direct_origins = payload.origins.clone();
            payload.direct_origins.remove(&Origin::Fresh);
            payload.direct_caller_references = payload.caller_references.clone();
            payload.fresh_roots = payload.fresh_references.clone();
            payload.needs_fresh_root = false;
            let fresh = self.allocate_fresh_container(call_key.preorder_index(), payload);
            let mut result = result;
            result.needs_fresh_root = false;
            result.join(&fresh);
            result.needs_fresh_root = false;
            result
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
                    value.direct_caller_references.clear();
                    value.fresh_roots.clear();
                    value.fresh_references.clear();
                    value.needs_fresh_root = false;
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
        let Some(mut callee) = native_callable_callee(binding_key) else {
            return self.apply_unknown_call_with_callee(
                callee_value,
                actuals,
                return_reference,
                EscapeLane::Native,
            );
        };
        project_exact_executor_category(&mut callee.effects, binding_key);
        if binding_key == "std.actor.get" {
            callee.effects.pending_effect_categories.retain(|category| {
                !matches!(
                    category,
                    PendingEffectCategory::NativeCall | PendingEffectCategory::HostEffect
                )
            });
            record_pending_category(&mut callee.effects, PendingEffectCategory::ActorCall);
        }
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
        let stored_value = receiver_store_value(op, args);
        if let Some(value) = stored_value {
            if receiver.unknown || value.unknown || self.store_would_create_cycle(&receiver, value)
            {
                self.state.join(&CallableState::fail_closed(
                    CallableProvenanceUnknownReason::UnsupportedHeapStore,
                ));
                return AbstractValue::unknown(return_reference);
            }
        }
        let mut actuals = Vec::with_capacity(args.len().saturating_add(1));
        actuals.push(receiver.clone());
        actuals.extend_from_slice(args);
        let Some(callee) = receiver_callable_callee(op) else {
            return self.apply_unknown_call_with_callee(
                callee_value,
                &actuals,
                return_reference,
                EscapeLane::Native,
            );
        };
        let result = self.apply_callee(&callee, &actuals, return_reference, None);
        if let Some(value) = stored_value {
            if !receiver.fresh_roots.is_empty() {
                self.store_into_fresh_roots(&receiver.fresh_roots, value);
            }
            for reference in &receiver.direct_caller_references {
                self.state
                    .parameter_stores
                    .entry(reference.clone())
                    .and_modify(|stored| stored.join(value))
                    .or_insert_with(|| value.clone());
            }
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

    fn apply_callback_interface_call(
        &mut self,
        _callee_value: &AbstractValue,
        actuals: &[AbstractValue],
        return_reference: bool,
    ) -> AbstractValue {
        let mut callee = CallableState::bottom();
        callee.effects = CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: vec![PendingEffectCategory::InterfaceCall],
            inout_path_effects: Vec::new(),
        };
        callee.return_origins.insert(Origin::Fresh);
        callee.return_direct_origins.insert(Origin::Fresh);
        callee.throw_origins.insert(Origin::Fresh);
        self.apply_callee(&callee, actuals, return_reference, None)
    }

    fn is_callback_carrier_receiver_call(
        &self,
        call_key: &ExpressionKey,
        callee: &Expr,
        receiver_index: Option<u32>,
    ) -> bool {
        if self.definition.has_receiver()
            || !matches!(
                self.definition.role,
                skiff_compiler_core::source_role::PublicationSourceRole::Package
            )
        {
            return false;
        }
        let Expr::Field { object, .. } = callee else {
            return false;
        };
        let Expr::Identifier(name) = object.as_ref() else {
            return false;
        };
        if !self
            .definition
            .function
            .params
            .iter()
            .any(|parameter| parameter.name == *name)
        {
            return false;
        }
        let Some(receiver_index) = receiver_index else {
            return false;
        };
        let receiver_key = ExpressionKey::new(
            call_key.module_path().to_string(),
            call_key.owner().clone(),
            receiver_index,
        );
        let Some(receiver_type) = self
            .expression_types
            .fact(&receiver_key)
            .and_then(|fact| fact.ty.as_ref())
            .map(|resolved| &resolved.ir)
        else {
            return false;
        };
        matches!(
            receiver_type,
            TypeRefIr::AnyInterface { interface }
                if interface.canonical_type_args.is_empty()
        )
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
        if matches!(
            callee.unknown,
            Some(CallableProvenanceUnknownReason::UnsupportedHeapStore)
        ) {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnsupportedHeapStore,
            ));
        }
        let any_caller_reference = actuals
            .iter()
            .any(|value| value.contains_caller_reference() || value.unknown);
        for (formal, stored) in &callee.parameter_stores {
            let Some(actual) = usize::try_from(formal.parameter)
                .ok()
                .and_then(|index| actuals.get(index))
            else {
                self.state.join(&CallableState::fail_closed(
                    CallableProvenanceUnknownReason::UnsupportedHeapStore,
                ));
                continue;
            };
            let mapped_target = self.project_store_target(actual, formal);
            let mapped = self.map_value(stored, actuals, true);
            if mapped_target.unknown
                || mapped.unknown
                || self.store_would_create_cycle(&mapped_target, &mapped)
            {
                self.state.join(&CallableState::fail_closed(
                    CallableProvenanceUnknownReason::UnsupportedHeapStore,
                ));
            } else {
                let mut transferred = false;
                if !mapped_target.fresh_roots.is_empty() {
                    self.store_into_fresh_roots(&mapped_target.fresh_roots, &mapped);
                    transferred = true;
                }
                if mapped_target.contains_direct_caller_reference() {
                    self.state.write_parameters.extend(
                        mapped_target
                            .direct_caller_references
                            .iter()
                            .map(|reference| reference.parameter),
                    );
                    for reference in &mapped_target.direct_caller_references {
                        self.state
                            .parameter_stores
                            .entry(reference.clone())
                            .and_modify(|value| value.join(&mapped))
                            .or_insert_with(|| mapped.clone());
                    }
                    transferred = true;
                }
                if !transferred
                    && (!mapped_target.origins.is_empty() || !mapped_target.fresh_roots.is_empty())
                {
                    self.state.join(&CallableState::fail_closed(
                        CallableProvenanceUnknownReason::UnsupportedHeapStore,
                    ));
                }
            }
        }
        if callee.effects.requires_same_heap_identity {
            let identity_actuals = indexed_actuals(&callee.same_heap_identity_parameters, actuals);
            // Only a mapped caller-owned identity makes the callee's real
            // observation visible here. An unknown actual remains rejected by
            // its unknown facts without manufacturing an identity observation.
            let identity_is_observable = if callee.same_heap_identity_parameters.is_empty() {
                actuals
                    .iter()
                    .any(|actual| actual.contains_direct_caller_reference())
            } else {
                identity_actuals
                    .iter()
                    .any(|actual| actual.contains_direct_caller_reference())
            };
            self.state.effects.requires_same_heap_identity |= identity_is_observable;
            if identity_is_observable {
                let relevant_actuals = if callee.same_heap_identity_parameters.is_empty() {
                    actuals.iter().collect::<Vec<_>>()
                } else {
                    identity_actuals
                };
                for actual in relevant_actuals {
                    self.state.same_heap_identity_parameters.extend(
                        actual
                            .direct_caller_references
                            .iter()
                            .map(|reference| reference.parameter),
                    );
                }
            }
        }
        self.state.effects.invokes_unknown_target |= callee.effects.invokes_unknown_target;
        self.state.effects.may_pending |= callee.effects.may_pending;
        super::super::provenance::union_pending_categories(
            &mut self.state.effects,
            &callee.effects.pending_effect_categories,
        );

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

        let mut returned = self.map_value_origins(
            &callee.return_origins,
            &callee.return_direct_origins,
            actuals,
            return_reference,
            true,
        );
        let return_formals = caller_parameter_indices(&callee.return_origins);
        // A fresh return that cannot be attributed to any parameter may still
        // embed caller-owned values when the callee wrote into the caller's
        // reachable graph (local summaries carry that internal fact; the wire
        // summary no longer does since the aggregate alias flags are retired).
        if (!callee.parameter_stores.is_empty() || !callee.write_parameters.is_empty())
            && return_formals.is_empty()
            && any_caller_reference
        {
            for actual in actuals {
                returned
                    .caller_references
                    .extend(actual.caller_references.iter().cloned());
                returned
                    .direct_caller_references
                    .extend(actual.direct_caller_references.iter().cloned());
                returned.unknown |= actual.unknown;
            }
        }
        if let Some(callable_id) = dependency_return {
            // This is a dependency-trace origin used by package build
            // identity, not an additional possible heap identity of the
            // returned value. The dependency's directReturnOrigins already
            // carry that semantic fact exactly.
            returned
                .origins
                .insert(Origin::DependencyReturn(callable_id));
        }

        let thrown = self.map_value_origins(
            &callee.throw_origins,
            &callee.throw_origins,
            actuals,
            true,
            true,
        );
        self.state
            .throw_origins
            .extend(thrown.origins.iter().cloned());

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
            returned.direct_caller_references.clear();
            returned.fresh_roots.clear();
            returned.fresh_references.clear();
            returned.needs_fresh_root = false;
        }
        returned
    }
}

fn receiver_store_value(op: BuiltinReceiverOp, args: &[AbstractValue]) -> Option<&AbstractValue> {
    match op.canonical_key {
        "receiver:Array.push@1" => args.first(),
        "receiver:Array.set@1" | "receiver:Map.set@1" | "receiver:JsonObject.set@1" => args.get(1),
        _ => None,
    }
}

fn native_callable_callee(binding_key: &str) -> Option<CallableState> {
    if let Some(semantics) = native_callable_semantics(binding_key) {
        let mut state = CallableState::bottom();
        state.effects = semantics.effects.clone();
        if binding_key == "std.http.stream.emitResponse" && state.effects.escapes_caller_value {
            state.escape_lanes.insert(EscapeLane::External);
            state
                .escape_parameters
                .insert(EscapeLane::External, BTreeSet::from([0]));
        }
        let origin = Origin::from(semantics.return_provenance.clone());
        state.return_origins.insert(origin.clone());
        state.return_direct_origins.insert(origin);
        return Some(state);
    }
    match binding_key {
        // runtime/native dispatch decodes into the current request heap and
        // either returns that newly materialized value or raises a detached
        // std.json.DecodeError. It does not retain the input or suspend.
        "std.json.decode" => {
            let mut state = CallableState::bottom();
            state.return_origins.insert(Origin::Fresh);
            state.return_direct_origins.insert(Origin::Fresh);
            Some(state)
        }
        _ => None,
    }
}

fn project_exact_executor_category(effects: &mut CallableMayEffects, binding_key: &str) {
    if !effects.may_pending {
        return;
    }
    let Some(executor) = host_effect_registry()
        .entries()
        .iter()
        .find(|entry| entry.binding_key == binding_key)
        .and_then(|entry| entry.executor_identity)
    else {
        return;
    };
    project_executor_category(effects, executor);
}

fn project_exact_package_executor_category(
    effects: &mut CallableMayEffects,
    callable_id: &PackageCallableId,
) {
    if !effects.may_pending {
        return;
    }
    let mut matches = host_effect_registry().entries().iter().filter_map(|entry| {
        let executor = entry.executor_identity?;
        let expected = public_package_callable_id(SKIFF_STD_PUBLICATION_ID, &entry.target).ok()?;
        (expected == *callable_id).then_some(executor)
    });
    let Some(executor) = matches.next() else {
        return;
    };
    if matches.next().is_some() {
        return;
    }
    project_executor_category(effects, executor);
}

fn project_executor_category(
    effects: &mut CallableMayEffects,
    executor: HostEffectExecutorIdentity,
) {
    let category = match executor {
        HostEffectExecutorIdentity::Sleep => PendingEffectCategory::NativeCall,
        HostEffectExecutorIdentity::HttpClientRequest
        | HostEffectExecutorIdentity::HttpClientStream => PendingEffectCategory::HostEffect,
        HostEffectExecutorIdentity::ActorGet => PendingEffectCategory::ActorCall,
    };
    effects.pending_effect_categories.retain(|candidate| {
        !matches!(
            candidate,
            PendingEffectCategory::NativeCall | PendingEffectCategory::HostEffect
        )
    });
    effects.pending_effect_categories.push(category);
}

fn receiver_callable_callee(op: BuiltinReceiverOp) -> Option<CallableState> {
    if let Some(semantics) = builtin_receiver_callable_semantics(op) {
        let mut state = CallableState::bottom();
        state.effects = semantics.effects.clone();
        // The receiver write fact lives in the op support table now; the
        // retired writesCallerReachable aggregate flag no longer exists.
        if receiver_mutates_receiver(op) {
            state.write_parameters.insert(0);
        }
        if state.effects.requires_same_heap_identity {
            state.same_heap_identity_parameters.insert(0);
        }
        let origin = Origin::from(semantics.return_provenance.clone());
        state.return_origins.insert(origin.clone());
        state.return_direct_origins.insert(origin);
        if matches!(
            op.canonical_key,
            "receiver:Map.get@1" | "receiver:JsonObject.get@1"
        ) {
            state.return_origins.remove(&Origin::CallerParameter(0));
            state
                .return_direct_origins
                .remove(&Origin::CallerParameter(0));
            let projection = Origin::CallerParameterProjection {
                index: 0,
                path: ValueProjectionPath::container_element(),
            };
            state.return_origins.insert(projection.clone());
            state.return_direct_origins.insert(projection);
        }
        return Some(state);
    }
    let mut state = CallableState::bottom();
    match op.canonical_key {
        "receiver:string.length@1" => {
            state.return_origins.insert(Origin::Constant);
            state.return_direct_origins.insert(Origin::Constant);
        }
        "receiver:bytes.toUtf8String@1" => {
            state.return_origins.insert(Origin::Fresh);
            state.return_direct_origins.insert(Origin::Fresh);
        }
        _ => return None,
    }
    Some(state)
}

fn receiver_mutates_receiver(op: BuiltinReceiverOp) -> bool {
    skiff_artifact_model::validate_supported_receiver_builtin_op(&op)
        .map(|spec| spec.mutates_receiver)
        .unwrap_or(false)
}

fn detached_contract_callee(operation: &BoundaryOperationDescriptor) -> Option<CallableState> {
    let contract = &operation.contract;
    let guarantee = contract.effect_guarantee;
    if !matches!(contract.stream, BoundaryStreamContract::Unary)
        || matches!(
            contract.callbacks,
            BoundaryCallbackContract::Unsupported { .. }
        )
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
    state.effects.may_pending = true;
    state
        .effects
        .pending_effect_categories
        .push(PendingEffectCategory::ServiceCall);
    state.return_origins.insert(Origin::Fresh);
    state.return_direct_origins.insert(Origin::Fresh);
    state.throw_origins.insert(Origin::Fresh);
    Some(state)
}

impl Evaluator<'_, '_> {
    fn map_origins(
        &mut self,
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
                        actual.direct_caller_references.clear();
                    }
                    if !reference {
                        actual.fresh_roots.clear();
                        actual.fresh_references.clear();
                        actual.caller_references.clear();
                        actual.direct_caller_references.clear();
                    }
                    mapped.join(&actual);
                }
                Origin::CallerParameterProjection { index, path } => {
                    let Some(actual) = usize::try_from(*index)
                        .ok()
                        .and_then(|index| actuals.get(index))
                    else {
                        mapped.unknown = true;
                        continue;
                    };
                    let projection =
                        self.project_value(actual, path, reference, preserve_caller_references);
                    mapped.join(&projection);
                }
                Origin::Fresh | Origin::Constant | Origin::DependencyReturn(_) => {
                    mapped.origins.insert(origin.clone());
                    mapped.direct_origins.insert(origin.clone());
                    mapped.needs_fresh_root |= reference && matches!(origin, Origin::Fresh);
                }
            }
        }
        mapped.reference = reference;
        if !reference {
            mapped.fresh_roots.clear();
            mapped.fresh_references.clear();
            mapped.caller_references.clear();
            mapped.direct_caller_references.clear();
            mapped.needs_fresh_root = false;
        }
        mapped
    }

    fn map_value_origins(
        &mut self,
        origins: &std::collections::BTreeSet<Origin>,
        direct_origins: &std::collections::BTreeSet<Origin>,
        actuals: &[AbstractValue],
        reference: bool,
        preserve_caller_references: bool,
    ) -> AbstractValue {
        let mut mapped = self.map_origins(origins, actuals, reference, preserve_caller_references);
        let direct = self.map_origins(
            direct_origins,
            actuals,
            reference,
            preserve_caller_references,
        );
        mapped.direct_origins = direct.direct_origins;
        mapped.direct_caller_references = direct.direct_caller_references;
        mapped.fresh_roots = direct.fresh_roots;
        mapped.needs_fresh_root = direct.needs_fresh_root;
        mapped.unknown |= direct.unknown;
        mapped
    }

    fn map_value(
        &mut self,
        value: &AbstractValue,
        actuals: &[AbstractValue],
        preserve_caller_references: bool,
    ) -> AbstractValue {
        let mut mapped = self.map_value_origins(
            &value.origins,
            &value.direct_origins,
            actuals,
            value.reference,
            preserve_caller_references,
        );
        mapped.unknown |= value.unknown;
        mapped
    }
}

fn caller_parameter_indices(
    origins: &std::collections::BTreeSet<Origin>,
) -> std::collections::BTreeSet<u32> {
    origins
        .iter()
        .filter_map(|origin| match origin {
            Origin::CallerParameter(index) | Origin::CallerParameterProjection { index, .. } => {
                Some(*index)
            }
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
