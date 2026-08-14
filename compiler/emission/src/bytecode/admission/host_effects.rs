use std::collections::BTreeMap;

use skiff_artifact_model::{
    host_effect_registry, CallIr, CallableMayEffects, CallableRegistryTypeExpression, ExprIr,
    HostEffectExecutorIdentity, HostEffectReceiverSemantics, HostEffectRegistryEntry, NativeTarget,
    NominalTypeRefBaseIr, PackageRefIr, TypeRefIr,
};
use skiff_compiler_lowering::mir::MirFunction;

#[derive(Debug)]
pub(super) struct HostEffectAdmissionError {
    pub expression_index: u32,
    pub detail: String,
}

#[derive(Debug, Clone, Copy)]
enum RegistryValueRole {
    Parameter { ordinal: usize },
    Result { ordinal: usize },
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
        };
        template.is_some_and(|template| match_type_expression(template, actual).is_ok())
    }
}

#[derive(Debug, Default)]
pub(super) struct HostEffectAdmissions {
    calls: BTreeMap<u32, HostEffectExecutorIdentity>,
    expressions: BTreeMap<u32, Vec<RegistryValueAuthority>>,
    slots: BTreeMap<u32, Vec<RegistryValueAuthority>>,
    entries: Vec<&'static HostEffectRegistryEntry>,
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
            admissions.entries.push(entry);
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

    pub(super) fn validate_effect_coverage(
        &self,
        actual: &CallableMayEffects,
    ) -> Result<(), String> {
        for entry in &self.entries {
            let expected = &entry.signature.effects;
            if expected.may_pending && !actual.may_pending {
                return Err(format!(
                    "registry executor {:?} is pending but the callable summary is not",
                    entry.executor_identity
                ));
            }
            for category in &expected.pending_effect_categories {
                if !actual.pending_effect_categories.contains(category) {
                    return Err(format!(
                        "registry executor {:?} requires pending category {category:?}",
                        entry.executor_identity
                    ));
                }
            }
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

fn is_void(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if name == "void" && args.is_empty())
}
