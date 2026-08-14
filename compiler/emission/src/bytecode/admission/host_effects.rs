use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    host_effect_registry, CallIr, CallableMayEffects, CallableRegistryTypeExpression, ExprIr,
    HostEffectExecutorIdentity, HostEffectReceiverSemantics, HostEffectRegistryEntry, NativeTarget,
    NominalTypeRefBaseIr, PackageRefIr, PendingEffectCategory, PrivilegedAffineCompositeIdentity,
    PrivilegedAffineFieldAccess, TypeRefIr,
};
use skiff_compiler_lowering::mir::{MirForInBinding, MirForInItemKind, MirFunction, MirStmtKind};

#[derive(Debug)]
pub(super) struct HostEffectAdmissionError {
    pub expression_index: u32,
    pub detail: String,
}

#[derive(Debug, Clone, Copy)]
enum RegistryValueRole {
    Parameter {
        ordinal: usize,
    },
    Result {
        ordinal: usize,
    },
    PrivilegedField {
        identity: PrivilegedAffineCompositeIdentity,
        ordinal: usize,
    },
    StreamItem {
        identity: PrivilegedAffineCompositeIdentity,
        field_ordinal: usize,
    },
}

/// Registry-owned authority for one exact value occurrence.
///
/// This is deliberately richer than an expression-position allow-list: every
/// occurrence retains the exact registry row, closed executor identity and
/// parameter/result ordinal that justified it. Merely having the same type,
/// appearing in the same function, or sharing a required context grants
/// nothing.
#[derive(Debug, Clone, Copy)]
pub(super) struct RegistryValueAuthority {
    entry: &'static HostEffectRegistryEntry,
    executor_identity: HostEffectExecutorIdentity,
    role: RegistryValueRole,
}

impl RegistryValueAuthority {
    pub(super) fn admits(&self, actual: &TypeRefIr) -> bool {
        if self.entry.executor_identity != Some(self.executor_identity) {
            return false;
        }
        let template = match self.role {
            RegistryValueRole::Parameter { ordinal } => {
                self.entry.signature.parameter_types.get(ordinal)
            }
            RegistryValueRole::Result { ordinal } => self.entry.signature.result_types.get(ordinal),
            RegistryValueRole::PrivilegedField { identity, ordinal } => {
                skiff_artifact_model::native_value_lifecycle_registry()
                    .privileged_affine_composite(identity)
                    .and_then(|schema| schema.fields.get(ordinal))
                    .map(|field| &field.ty)
            }
            RegistryValueRole::StreamItem {
                identity,
                field_ordinal,
            } => skiff_artifact_model::native_value_lifecycle_registry()
                .privileged_affine_composite(identity)
                .and_then(|schema| schema.fields.get(field_ordinal))
                .and_then(|field| match &field.ty {
                    CallableRegistryTypeExpression::Builtin { name, arguments }
                        if name == "Stream" && arguments.len() == 1 =>
                    {
                        arguments.first()
                    }
                    _ => None,
                }),
        };
        template.is_some_and(|template| match_type_expression(template, actual).is_ok())
    }

    fn is_http_stream_result(&self) -> bool {
        self.executor_identity == HostEffectExecutorIdentity::HttpClientStream
            && matches!(self.role, RegistryValueRole::Result { ordinal: 0 })
    }

    fn is_parameter(&self) -> bool {
        matches!(self.role, RegistryValueRole::Parameter { .. })
    }

    fn is_privileged_stream(&self) -> bool {
        matches!(self.role, RegistryValueRole::PrivilegedField { .. })
    }

    fn is_stream_item(&self) -> bool {
        matches!(self.role, RegistryValueRole::StreamItem { .. })
    }

    fn stream_item_authority(&self) -> Option<Self> {
        let RegistryValueRole::PrivilegedField { identity, ordinal } = self.role else {
            return None;
        };
        let field = skiff_artifact_model::native_value_lifecycle_registry()
            .privileged_affine_composite(identity)?
            .fields
            .get(ordinal)?;
        if field.access != PrivilegedAffineFieldAccess::AffineTake {
            return None;
        }
        Some(Self {
            entry: self.entry,
            executor_identity: self.executor_identity,
            role: RegistryValueRole::StreamItem {
                identity,
                field_ordinal: ordinal,
            },
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct HostEffectAdmissions {
    calls: BTreeMap<u32, HostEffectExecutorIdentity>,
    expressions: BTreeMap<u32, Vec<RegistryValueAuthority>>,
    slots: BTreeMap<u32, Vec<RegistryValueAuthority>>,
    stream_for_in_statements: BTreeSet<u32>,
    stream_next_statements: BTreeSet<u32>,
}

impl HostEffectAdmissions {
    /// Matches every native call except the separately admitted pure Duration
    /// constructor before any value-shape admission runs.
    pub(super) fn analyze(
        function: &MirFunction,
        duration_constructor_binding: &str,
    ) -> Result<Self, HostEffectAdmissionError> {
        let mut admissions = Self::default();
        for expression in &function.expressions {
            let ExprIr::Call { call } = &expression.expression else {
                continue;
            };
            let skiff_artifact_model::CallTargetIr::Native { target } = &call.target else {
                continue;
            };
            if target.binding_key.as_deref() == Some(duration_constructor_binding) {
                continue;
            }
            let (entry, executor_identity) =
                match_executable_call(function, expression.index, &expression.ty, call, target)
                    .map_err(|detail| HostEffectAdmissionError {
                        expression_index: expression.index,
                        detail,
                    })?;

            admissions.calls.insert(expression.index, executor_identity);
            for (ordinal, argument) in call.args.iter().enumerate() {
                let authority = RegistryValueAuthority {
                    entry,
                    executor_identity,
                    role: RegistryValueRole::Parameter { ordinal },
                };
                admissions
                    .expressions
                    .entry(argument.expression)
                    .or_default()
                    .push(authority);
                if let ExprIr::LoadSlot { slot } = &function
                    .expression(*argument)
                    .map_err(|error| HostEffectAdmissionError {
                        expression_index: expression.index,
                        detail: format!("host argument expression is absent: {error}"),
                    })?
                    .expression
                {
                    admissions.slots.entry(*slot).or_default().push(authority);
                }
            }
            if !entry.signature.result_types.is_empty() {
                admissions
                    .expressions
                    .entry(expression.index)
                    .or_default()
                    .push(RegistryValueAuthority {
                        entry,
                        executor_identity,
                        role: RegistryValueRole::Result { ordinal: 0 },
                    });
            }
        }

        // A registry result may acquire frame ownership only through the
        // exact source InitSlot edge that consumes that call expression.
        // Assign/copy/type equality never propagates authority.
        for block in &function.blocks {
            for statement in &block.statements {
                let MirStmtKind::InitSlot { slot, value } = &statement.kind else {
                    continue;
                };
                let slot_type =
                    function
                        .slot_type(*slot)
                        .map_err(|error| HostEffectAdmissionError {
                            expression_index: value.expression,
                            detail: format!("host result destination slot is absent: {error}"),
                        })?;
                let authorities = admissions
                    .expressions
                    .get(&value.expression)
                    .cloned()
                    .unwrap_or_default();
                for authority in authorities {
                    if authority.is_http_stream_result() && authority.admits(slot_type) {
                        admissions.slots.entry(*slot).or_default().push(authority);
                    }
                }
            }
        }

        // A host parameter may be constructed in one source local before the
        // exact call loads it. Propagate only backwards across that InitSlot
        // edge, retaining the exact executor/parameter ordinal and requiring
        // both sides to match its registry type. No type-only or position-only
        // inference is performed.
        for block in &function.blocks {
            for statement in &block.statements {
                let MirStmtKind::InitSlot { slot, value } = &statement.kind else {
                    continue;
                };
                let slot_type =
                    function
                        .slot_type(*slot)
                        .map_err(|error| HostEffectAdmissionError {
                            expression_index: value.expression,
                            detail: format!("host parameter source slot is absent: {error}"),
                        })?;
                let value_type = &function
                    .expression(*value)
                    .map_err(|error| HostEffectAdmissionError {
                        expression_index: value.expression,
                        detail: format!("host parameter source expression is absent: {error}"),
                    })?
                    .ty;
                if slot_type != value_type {
                    continue;
                }
                let authorities = admissions.slots.get(slot).cloned().unwrap_or_default();
                for authority in authorities {
                    if authority.is_parameter() && authority.admits(value_type) {
                        admissions
                            .expressions
                            .entry(value.expression)
                            .or_default()
                            .push(authority);
                    }
                }
            }
        }

        // The affine body edge is exact in all four coordinates: producer
        // result slot, LoadSlot occurrence, privileged registry field, and
        // field result type. A second take from the same aggregate is rejected
        // here rather than left to expression position or a later type guess.
        let mut body_objects = BTreeSet::new();
        let mut taken_handle_slots = BTreeSet::new();
        for expression in &function.expressions {
            let ExprIr::Field { object, field } = &expression.expression else {
                continue;
            };
            if field != "body" {
                continue;
            }
            let object_expression =
                function
                    .expression(*object)
                    .map_err(|error| HostEffectAdmissionError {
                        expression_index: expression.index,
                        detail: format!("privileged body owner expression is absent: {error}"),
                    })?;
            let ExprIr::LoadSlot { slot } = object_expression.expression else {
                continue;
            };
            let result_authority = admissions.slots.get(&slot).and_then(|authorities| {
                authorities
                    .iter()
                    .copied()
                    .find(RegistryValueAuthority::is_http_stream_result)
            });
            let Some(result_authority) = result_authority else {
                continue;
            };
            if !result_authority.admits(&object_expression.ty) {
                return Err(HostEffectAdmissionError {
                    expression_index: expression.index,
                    detail: "privileged body owner type differs from its exact host result"
                        .to_string(),
                });
            }
            let identity = privileged_identity(&object_expression.ty).ok_or_else(|| {
                HostEffectAdmissionError {
                    expression_index: expression.index,
                    detail: "host stream result lacks exact privileged composite registry identity"
                        .to_string(),
                }
            })?;
            let schema = skiff_artifact_model::native_value_lifecycle_registry()
                .privileged_affine_composite(identity)
                .expect("registry identity came from the same registry");
            let Some(ordinal) = schema.fields.iter().position(|candidate| {
                candidate.name == *field
                    && candidate.access == PrivilegedAffineFieldAccess::AffineTake
            }) else {
                return Err(HostEffectAdmissionError {
                    expression_index: expression.index,
                    detail: "body is not the exact affine-take registry field".to_string(),
                });
            };
            let field_authority = RegistryValueAuthority {
                entry: result_authority.entry,
                executor_identity: result_authority.executor_identity,
                role: RegistryValueRole::PrivilegedField { identity, ordinal },
            };
            if !field_authority.admits(&expression.ty) {
                return Err(HostEffectAdmissionError {
                    expression_index: expression.index,
                    detail: "body result type differs from the privileged registry field"
                        .to_string(),
                });
            }
            if !taken_handle_slots.insert(slot) {
                return Err(HostEffectAdmissionError {
                    expression_index: expression.index,
                    detail: format!("host stream handle slot {slot} has a second affine body take"),
                });
            }
            body_objects.insert(object.expression);
            admissions
                .expressions
                .entry(object.expression)
                .or_default()
                .push(result_authority);
            admissions
                .expressions
                .entry(expression.index)
                .or_default()
                .push(field_authority);
        }

        // An exact body stream may be parked in one request-local source slot
        // only by its InitSlot edge. This retains the original registry field
        // authority rather than minting capability from Stream<T>.
        for block in &function.blocks {
            for statement in &block.statements {
                let MirStmtKind::InitSlot { slot, value } = &statement.kind else {
                    continue;
                };
                let slot_type =
                    function
                        .slot_type(*slot)
                        .map_err(|error| HostEffectAdmissionError {
                            expression_index: value.expression,
                            detail: format!("stream destination slot is absent: {error}"),
                        })?;
                let authorities = admissions
                    .expressions
                    .get(&value.expression)
                    .cloned()
                    .unwrap_or_default();
                for authority in authorities {
                    if authority.is_privileged_stream() && authority.admits(slot_type) {
                        admissions.slots.entry(*slot).or_default().push(authority);
                    }
                }
            }
        }

        let mut authorized_stream_loads = BTreeSet::new();
        for block in &function.blocks {
            for statement in &block.statements {
                match &statement.kind {
                    MirStmtKind::ForIn {
                        iterable, facts, ..
                    } => {
                        let iterable_expression =
                            function.expression(*iterable).map_err(|error| {
                                HostEffectAdmissionError {
                                    expression_index: iterable.expression,
                                    detail: format!(
                                        "stream iterable expression is absent: {error}"
                                    ),
                                }
                            })?;
                        if facts.iterable_type != iterable_expression.ty {
                            return Err(HostEffectAdmissionError {
                                expression_index: iterable.expression,
                                detail:
                                    "for-in iterable type differs from its producer-owned MIR facts"
                                        .to_string(),
                            });
                        }
                        let iterable_authority = admissions
                            .expressions
                            .get(&iterable.expression)
                            .and_then(|authorities| {
                                authorities
                                    .iter()
                                    .copied()
                                    .find(RegistryValueAuthority::is_privileged_stream)
                            })
                            .or_else(|| {
                                let ExprIr::LoadSlot { slot } = iterable_expression.expression
                                else {
                                    return None;
                                };
                                admissions.slots.get(&slot).and_then(|authorities| {
                                    authorities
                                        .iter()
                                        .copied()
                                        .find(RegistryValueAuthority::is_privileged_stream)
                                })
                            });
                        let Some(iterable_authority) = iterable_authority else {
                            continue;
                        };
                        if !iterable_authority.admits(&iterable_expression.ty) {
                            return Err(HostEffectAdmissionError {
                                expression_index: iterable.expression,
                                detail: "for-in stream type differs from exact body authority"
                                    .to_string(),
                            });
                        }
                        let MirForInBinding::Item {
                            slot,
                            ty,
                            kind: MirForInItemKind::StreamItem,
                        } = &facts.binding
                        else {
                            return Err(HostEffectAdmissionError {
                                expression_index: iterable.expression,
                                detail: "exact body stream lacks StreamItem for-in binding facts"
                                    .to_string(),
                            });
                        };
                        let item_authority = iterable_authority
                            .stream_item_authority()
                            .expect("privileged stream authority is the affine stream field");
                        if !item_authority.admits(ty) || function.slot_type(*slot).ok() != Some(ty)
                        {
                            return Err(HostEffectAdmissionError {
                                expression_index: iterable.expression,
                                detail: "for-in item slot/type differs from the exact stream item"
                                    .to_string(),
                            });
                        }
                        if let ExprIr::LoadSlot { .. } = iterable_expression.expression {
                            admissions
                                .expressions
                                .entry(iterable.expression)
                                .or_default()
                                .push(iterable_authority);
                            authorized_stream_loads.insert(iterable.expression);
                        }
                        admissions
                            .slots
                            .entry(*slot)
                            .or_default()
                            .push(item_authority);
                        admissions
                            .stream_for_in_statements
                            .insert(statement.statement_index);
                    }
                    MirStmtKind::StreamNext {
                        endpoint_slot,
                        item_type,
                    } => {
                        let authority =
                            admissions.slots.get(endpoint_slot).and_then(|authorities| {
                                authorities
                                    .iter()
                                    .copied()
                                    .find(RegistryValueAuthority::is_privileged_stream)
                            });
                        let Some(authority) = authority else {
                            continue;
                        };
                        if authority
                            .stream_item_authority()
                            .is_none_or(|item| !item.admits(item_type))
                        {
                            return Err(HostEffectAdmissionError {
                                expression_index: u32::MAX,
                                detail: "StreamNext item type differs from exact body authority"
                                    .to_string(),
                            });
                        }
                        admissions
                            .stream_next_statements
                            .insert(statement.statement_index);
                    }
                    _ => {}
                }
            }
        }

        // Stream-item slot loads retain the producer-owned for-in item row.
        // Handle and stream-owner loads remain restricted to the exact
        // body/for-in occurrences collected above.
        for expression in &function.expressions {
            let ExprIr::LoadSlot { slot } = expression.expression else {
                continue;
            };
            let slot_authorities = admissions.slots.get(&slot).cloned().unwrap_or_default();
            for authority in slot_authorities {
                if authority.is_stream_item() && authority.admits(&expression.ty) {
                    admissions
                        .expressions
                        .entry(expression.index)
                        .or_default()
                        .push(authority);
                } else if authority.is_http_stream_result()
                    && !body_objects.contains(&expression.index)
                {
                    return Err(HostEffectAdmissionError {
                        expression_index: expression.index,
                        detail: format!(
                            "host stream handle slot {slot} is loaded outside its exact body take"
                        ),
                    });
                } else if authority.is_privileged_stream()
                    && !authorized_stream_loads.contains(&expression.index)
                {
                    return Err(HostEffectAdmissionError {
                        expression_index: expression.index,
                        detail: format!(
                            "body stream slot {slot} is loaded outside exact stream consumption"
                        ),
                    });
                }
            }
        }
        Ok(admissions)
    }

    pub(super) fn expression_authorities(
        &self,
        expression_index: u32,
    ) -> &[RegistryValueAuthority] {
        self.expressions
            .get(&expression_index)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn slot_authorities(&self, slot: u32) -> &[RegistryValueAuthority] {
        self.slots.get(&slot).map(Vec::as_slice).unwrap_or_default()
    }

    pub(super) fn executor_for_call(
        &self,
        expression_index: u32,
    ) -> Option<HostEffectExecutorIdentity> {
        self.calls.get(&expression_index).copied()
    }

    pub(super) fn admits_stream_for_in(&self, statement_index: u32) -> bool {
        self.stream_for_in_statements.contains(&statement_index)
    }

    pub(super) fn admits_stream_next(&self, statement_index: u32) -> bool {
        self.stream_next_statements.contains(&statement_index)
    }

    pub(super) fn has_stream_pending(&self) -> bool {
        !self.stream_for_in_statements.is_empty() || !self.stream_next_statements.is_empty()
    }

    pub(super) fn validate_effect_coverage(
        &self,
        actual: &CallableMayEffects,
        stream_pending: bool,
    ) -> Result<(), String> {
        let mut expected = BTreeSet::new();
        for executor in self.calls.values() {
            expected.insert(match executor {
                HostEffectExecutorIdentity::Sleep => PendingEffectCategory::NativeCall,
                HostEffectExecutorIdentity::HttpClientRequest
                | HostEffectExecutorIdentity::HttpClientStream => PendingEffectCategory::HostEffect,
            });
        }
        if stream_pending {
            expected.insert(PendingEffectCategory::Stream);
        }

        let actual_categories = actual
            .pending_effect_categories
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if actual_categories.len() != actual.pending_effect_categories.len() {
            return Err("callable summary repeats a pending category".to_string());
        }
        if actual.may_pending != !actual_categories.is_empty() {
            return Err("callable mayPending disagrees with its exact category set".to_string());
        }
        if actual_categories != expected {
            return Err(format!(
                "callable categories {actual_categories:?} differ from exact producer categories {expected:?}"
            ));
        }
        Ok(())
    }
}

fn match_executable_call(
    function: &MirFunction,
    expression_index: u32,
    result_type: &TypeRefIr,
    call: &CallIr,
    target: &NativeTarget,
) -> Result<(&'static HostEffectRegistryEntry, HostEffectExecutorIdentity), String> {
    if !call.type_args.is_empty() {
        return Err("host call carries type arguments".to_string());
    }
    if !call.inout_args.is_empty() {
        return Err("host call carries inout arguments".to_string());
    }
    if call.concrete_receiver.is_some() {
        return Err("host call carries a concrete receiver".to_string());
    }
    if !call.metadata.is_empty() {
        return Err("host call carries callsite metadata".to_string());
    }

    let canonical_target = if target.namespace.is_empty() {
        target.symbol.clone()
    } else {
        format!("{}.{}", target.namespace, target.symbol)
    };
    let entry = host_effect_registry()
        .entries()
        .iter()
        .find(|entry| entry.target == canonical_target)
        .ok_or_else(|| format!("native target `{canonical_target}` has no exact registry row"))?;
    let binding_key = target
        .binding_key
        .as_deref()
        .ok_or_else(|| "native target lacks an exact binding key".to_string())?;
    if binding_key != entry.binding_key {
        return Err(format!(
            "native target `{canonical_target}` binding `{binding_key}` differs from its registry row"
        ));
    }
    if !entry.metadata.matches(&target.metadata) {
        return Err(format!(
            "native target `{canonical_target}` metadata differs from its registry row"
        ));
    }
    if !matches!(entry.receiver, HostEffectReceiverSemantics::None) {
        return Err(format!(
            "native target `{canonical_target}` has unsupported receiver semantics"
        ));
    }
    let executor_identity = entry
        .executor_identity
        .ok_or_else(|| format!("registry row `{binding_key}` has no bytecode executor identity"))?;
    match executor_identity {
        HostEffectExecutorIdentity::Sleep
        | HostEffectExecutorIdentity::HttpClientRequest
        | HostEffectExecutorIdentity::HttpClientStream => {}
    }

    if call.args.len() != entry.signature.parameter_types.len() {
        return Err(format!(
            "registry executor {executor_identity:?} expects {} parameters but call has {}",
            entry.signature.parameter_types.len(),
            call.args.len()
        ));
    }
    for (ordinal, (argument, template)) in call
        .args
        .iter()
        .zip(&entry.signature.parameter_types)
        .enumerate()
    {
        let actual = &function
            .expression(*argument)
            .map_err(|error| format!("parameter {ordinal} expression is absent: {error}"))?
            .ty;
        match_type_expression(template, actual).map_err(|detail| {
            format!("registry executor {executor_identity:?} parameter {ordinal}: {detail}")
        })?;
    }
    match entry.signature.result_types.as_slice() {
        [] if is_void(result_type) => {}
        [template] => match_type_expression(template, result_type).map_err(|detail| {
            format!("registry executor {executor_identity:?} result: {detail}")
        })?,
        [] => {
            return Err(format!(
                "registry executor {executor_identity:?} has void result but expression {expression_index} is {result_type:?}"
            ))
        }
        results => {
            return Err(format!(
                "registry executor {executor_identity:?} has unsupported result arity {}",
                results.len()
            ))
        }
    }
    Ok((entry, executor_identity))
}

fn match_type_expression(
    template: &CallableRegistryTypeExpression,
    actual: &TypeRefIr,
) -> Result<(), String> {
    match template {
        CallableRegistryTypeExpression::TypeParameter { .. } => {
            Err("generic host executor signatures are outside this admission slice".to_string())
        }
        CallableRegistryTypeExpression::Builtin {
            name,
            arguments: expected_arguments,
        } => {
            let (actual_name, actual_arguments): (&str, &[TypeRefIr]) = match actual {
                TypeRefIr::Builtin { name, args } => (name, args),
                TypeRefIr::Nullable { inner } if name == "Nullable" => {
                    ("Nullable", std::slice::from_ref(inner.as_ref()))
                }
                _ => return Err(format!("expected builtin `{name}`")),
            };
            if actual_name != name || actual_arguments.len() != expected_arguments.len() {
                return Err(format!("expected builtin `{name}` with exact arity"));
            }
            for (expected, actual) in expected_arguments.iter().zip(actual_arguments) {
                match_type_expression(expected, actual)?;
            }
            Ok(())
        }
        CallableRegistryTypeExpression::PackageSymbol {
            package_id,
            symbol_path,
        } => {
            let symbol = match actual {
                TypeRefIr::PackageSymbol { symbol } => symbol,
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::PackageSymbol { symbol },
                    arguments,
                } if arguments.is_empty() => symbol,
                _ => return Err("expected an exact package symbol".to_string()),
            };
            let PackageRefIr::PackageId {
                package_id: actual_package_id,
            } = &symbol.package
            else {
                return Err("package symbol retains a dependency alias".to_string());
            };
            if actual_package_id != package_id || symbol.symbol_path != *symbol_path {
                return Err("package symbol owner/path differs from registry signature".to_string());
            }
            // The callable registry expression owns the package id and public
            // symbol path, but it does not contain a package ABI identity.
            // Production publication/linking owns that additional fence; the
            // compiler admission boundary must neither invent one nor compare
            // it to an unowned constant.
            Ok(())
        }
    }
}

fn privileged_identity(ty: &TypeRefIr) -> Option<PrivilegedAffineCompositeIdentity> {
    let symbol = match ty {
        TypeRefIr::PackageSymbol { symbol } => symbol,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol { symbol },
            arguments,
        } if arguments.is_empty() => symbol,
        _ => return None,
    };
    skiff_artifact_model::native_value_lifecycle_registry()
        .privileged_affine_composite_for_symbol(symbol)
        .map(|schema| schema.identity)
}

fn is_void(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if name == "void" && args.is_empty())
}
