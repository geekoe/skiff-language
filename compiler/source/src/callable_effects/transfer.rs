use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{CallableProvenanceUnknownReason, TypeRefIr};

use crate::{
    shared::ast::TypeRef, ExpressionKey, ExpressionTypeModel, ResolvedCallTargetFacts,
    SourceDependencyAnalysisInput, SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

use super::{
    provenance::{AbstractValue, CallableState, EscapeLane},
    CallableDefinition,
};

mod call;
mod expression;
mod statement;

type Environment = BTreeMap<String, AbstractValue>;

pub(super) fn transfer_callable(
    definition: &CallableDefinition<'_>,
    definitions: &BTreeMap<SourceSymbolKey, CallableDefinition<'_>>,
    summaries: &BTreeMap<SourceSymbolKey, CallableState>,
    resolved_call_targets: &ResolvedCallTargetFacts,
    dependency_analysis: &SourceDependencyAnalysisInput,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
) -> CallableState {
    if definition.function.is_native {
        let mut state =
            CallableState::fail_closed(CallableProvenanceUnknownReason::UnknownCallTarget);
        state.escape_lanes = BTreeSet::from([EscapeLane::Native]);
        return state;
    }
    if definition.function.is_provider {
        return CallableState::fail_closed(CallableProvenanceUnknownReason::UnknownCallTarget);
    }

    let mut evaluator = Evaluator {
        definition,
        definitions,
        summaries,
        resolved_call_targets,
        dependency_analysis,
        expression_types,
        type_resolution,
        next_index: 0,
        values: BTreeMap::new(),
        state: CallableState::bottom(),
    };
    let mut env = evaluator.parameter_environment();
    evaluator.eval_block(&definition.function.body, &mut env);
    evaluator.state
}

struct Evaluator<'a, 'source> {
    definition: &'a CallableDefinition<'source>,
    definitions: &'a BTreeMap<SourceSymbolKey, CallableDefinition<'source>>,
    summaries: &'a BTreeMap<SourceSymbolKey, CallableState>,
    resolved_call_targets: &'a ResolvedCallTargetFacts,
    dependency_analysis: &'a SourceDependencyAnalysisInput,
    expression_types: &'a ExpressionTypeModel,
    type_resolution: &'a TypeResolutionModel,
    next_index: u32,
    values: BTreeMap<u32, AbstractValue>,
    state: CallableState,
}

impl Evaluator<'_, '_> {
    fn parameter_environment(&self) -> Environment {
        let mut env = Environment::new();
        let mut next_parameter = 0u32;
        if let Some(self_type) = self.definition.function.implicit_self.as_ref() {
            env.insert(
                "self".to_string(),
                AbstractValue::parameter(next_parameter, self.type_ref_may_be_reference(self_type)),
            );
            next_parameter += 1;
        }
        for param in &self.definition.function.params {
            let index = if param.name == "self" && env.contains_key("self") {
                0
            } else {
                let index = next_parameter;
                next_parameter += 1;
                index
            };
            env.insert(
                param.name.clone(),
                AbstractValue::parameter(index, self.type_ref_may_be_reference(&param.ty)),
            );
        }
        env
    }

    fn current_key(&self) -> ExpressionKey {
        ExpressionKey::new(
            self.definition.module_path,
            owner_key(self.definition),
            self.next_index,
        )
    }

    fn type_ref_may_be_reference(&self, ty: &TypeRef) -> bool {
        let context = TypeResolutionContext::with_type_params(
            self.definition.module_path,
            self.definition.type_params.iter().cloned().collect(),
        );
        self.type_resolution
            .resolve_type_ref(ty, &context)
            .map(|resolved| type_ir_may_be_reference(&resolved.ir))
            .unwrap_or(true)
    }

    fn expression_may_be_reference(&self, key: &ExpressionKey) -> bool {
        self.expression_types
            .fact(key)
            .and_then(|fact| fact.ty.as_ref())
            .map(|resolved| type_ir_may_be_reference(&resolved.ir))
            .unwrap_or(true)
    }

    fn value_at(&self, preorder_index: u32) -> Option<&AbstractValue> {
        self.values.get(&preorder_index)
    }

    fn values_in_range(&self, start: u32, end: u32) -> AbstractValue {
        let mut value = AbstractValue::default();
        for (_, part) in self.values.range(start..end) {
            value.join(part);
        }
        value
    }
}

fn owner_key(definition: &CallableDefinition<'_>) -> crate::ExpressionOwnerKey {
    if let Some((type_name, method)) = split_impl_method_symbol(definition.key.symbol()) {
        crate::ExpressionOwnerKey::ImplMethod {
            type_name: type_name.to_string(),
            method: method.to_string(),
        }
    } else {
        crate::ExpressionOwnerKey::Function(definition.key.symbol().to_string())
    }
}

fn split_impl_method_symbol(symbol: &str) -> Option<(&str, &str)> {
    symbol.rsplit_once('.')
}

fn join_environments(target: &mut Environment, source: &Environment) -> bool {
    let before = target.clone();
    for (name, value) in source {
        target
            .entry(name.clone())
            .and_modify(|existing| existing.join(value))
            .or_insert_with(|| value.clone());
    }
    *target != before
}

fn type_ir_may_be_reference(ty: &TypeRefIr) -> bool {
    !type_ir_is_definitely_scalar(ty)
}

fn type_ir_is_definitely_scalar(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Native { name, args } => {
            args.is_empty()
                && matches!(
                    name.as_str(),
                    "string" | "integer" | "number" | "bool" | "null" | "void" | "never"
                )
        }
        TypeRefIr::Literal { .. } => true,
        TypeRefIr::Union { items } => items.iter().all(type_ir_is_definitely_scalar),
        TypeRefIr::Nullable { inner } => type_ir_is_definitely_scalar(inner),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => false,
    }
}

fn pattern_bindings(pattern: &crate::shared::ast::Pattern, names: &mut BTreeSet<String>) {
    use crate::shared::ast::Pattern;

    match pattern {
        Pattern::Binding(name) => {
            names.insert(name.clone());
        }
        Pattern::Nominal { fields, .. } | Pattern::Record { fields } => {
            for field in fields {
                if let Some(pattern) = &field.pattern {
                    pattern_bindings(pattern, names);
                } else {
                    names.insert(field.name.clone());
                }
            }
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                pattern_bindings(pattern, names);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}
