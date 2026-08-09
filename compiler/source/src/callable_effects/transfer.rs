use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    native_callable_semantics, CallableProvenanceUnknownReason, TypeRefIr, ValueProjectionPath,
};

use crate::{
    shared::ast::TypeRef, ExpressionKey, ExpressionTypeModel, ResolvedCallTargetFacts,
    SourceDependencyAnalysisInput, SourceSymbolKey, TypeResolutionContext, TypeResolutionModel,
};

use super::{
    analysis::ModuleConstantFact,
    provenance::{AbstractValue, CallableState, CallerReference, EscapeLane, FreshRoot, Origin},
    CallableDefinition,
};

mod call;
mod expression;
mod statement;

type Environment = BTreeMap<String, AbstractValue>;

pub(super) fn transfer_callable(
    definition: &CallableDefinition<'_>,
    definitions: &BTreeMap<SourceSymbolKey, CallableDefinition<'_>>,
    module_constants: &BTreeMap<SourceSymbolKey, ModuleConstantFact>,
    summaries: &BTreeMap<SourceSymbolKey, CallableState>,
    resolved_call_targets: &ResolvedCallTargetFacts,
    dependency_analysis: &SourceDependencyAnalysisInput,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
) -> CallableState {
    if definition.function.is_native {
        if let Some(binding_key) = crate::prelude_registry::prelude_registry()
            .native_binding_key(&definition.key.to_source_symbol())
        {
            if let Some(semantics) = native_callable_semantics(binding_key) {
                let mut state = CallableState::bottom();
                state.effects = semantics.effects.clone();
                let origin = Origin::from(semantics.return_provenance.clone());
                state.return_origins.insert(origin.clone());
                state.return_direct_origins.insert(origin);
                return state;
            }
        }
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
        module_constants,
        summaries,
        resolved_call_targets,
        dependency_analysis,
        expression_types,
        type_resolution,
        next_index: 0,
        values: BTreeMap::new(),
        heap: BTreeMap::new(),
        mutated_fresh_roots: BTreeSet::new(),
        state: CallableState::bottom(),
    };
    let mut env = evaluator.parameter_environment();
    evaluator.eval_block(&definition.function.body, &mut env);
    evaluator.state
}

struct Evaluator<'a, 'source> {
    definition: &'a CallableDefinition<'source>,
    definitions: &'a BTreeMap<SourceSymbolKey, CallableDefinition<'source>>,
    module_constants: &'a BTreeMap<SourceSymbolKey, ModuleConstantFact>,
    summaries: &'a BTreeMap<SourceSymbolKey, CallableState>,
    resolved_call_targets: &'a ResolvedCallTargetFacts,
    dependency_analysis: &'a SourceDependencyAnalysisInput,
    expression_types: &'a ExpressionTypeModel,
    type_resolution: &'a TypeResolutionModel,
    next_index: u32,
    values: BTreeMap<u32, AbstractValue>,
    heap: BTreeMap<FreshRoot, AbstractValue>,
    mutated_fresh_roots: BTreeSet<FreshRoot>,
    state: CallableState,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ReferenceNode {
    Fresh(FreshRoot),
    Caller(CallerReference),
}

impl Evaluator<'_, '_> {
    fn materialize_heap_value(&self, value: &AbstractValue) -> AbstractValue {
        let mut materialized = value.clone();
        let direct_origins = value.direct_origins.clone();
        let direct_caller_references = value.direct_caller_references.clone();
        let fresh_roots = value.fresh_roots.clone();
        let needs_fresh_root = value.needs_fresh_root;
        let mut pending = value.fresh_references.iter().cloned().collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(root) = pending.pop() {
            if !visited.insert(root.clone()) {
                continue;
            }
            if let Some(payload) = self.heap.get(&root) {
                materialized.join(payload);
                pending.extend(payload.fresh_references.iter().cloned());
            }
        }
        materialized.direct_origins = direct_origins;
        materialized.direct_caller_references = direct_caller_references;
        materialized.fresh_roots = fresh_roots;
        materialized.needs_fresh_root = needs_fresh_root;
        materialized
    }

    fn store_into_fresh_roots(&mut self, roots: &BTreeSet<FreshRoot>, value: &AbstractValue) {
        for root in roots {
            self.mutated_fresh_roots.insert(root.clone());
            self.heap
                .entry(root.clone())
                .and_modify(|payload| payload.join(value))
                .or_insert_with(|| value.clone());
        }
    }

    fn reference_graph_reaches_any(&self, value: &AbstractValue, targets: &AbstractValue) -> bool {
        let mut pending = value
            .fresh_references
            .iter()
            .cloned()
            .map(ReferenceNode::Fresh)
            .chain(
                value
                    .caller_references
                    .iter()
                    .cloned()
                    .map(ReferenceNode::Caller),
            )
            .collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(node) = pending.pop() {
            let reaches_target = match &node {
                ReferenceNode::Fresh(root) => targets
                    .fresh_roots
                    .iter()
                    .any(|target| root.is_ancestor_of(target)),
                ReferenceNode::Caller(reference) => targets
                    .direct_caller_references
                    .iter()
                    .any(|target| reference.is_ancestor_of(target)),
            };
            if reaches_target {
                return true;
            }
            if !visited.insert(node.clone()) {
                continue;
            }
            let payload = match &node {
                ReferenceNode::Fresh(root) => self.heap.get(root),
                ReferenceNode::Caller(reference) => self.state.parameter_stores.get(reference),
            };
            if let Some(payload) = payload {
                pending.extend(
                    payload
                        .fresh_references
                        .iter()
                        .cloned()
                        .map(ReferenceNode::Fresh),
                );
                pending.extend(
                    payload
                        .caller_references
                        .iter()
                        .cloned()
                        .map(ReferenceNode::Caller),
                );
            }
        }
        false
    }

    fn store_would_create_cycle(&self, target: &AbstractValue, value: &AbstractValue) -> bool {
        self.reference_graph_reaches_any(value, target)
    }

    fn contains_mutated_fresh_root(&self, value: &AbstractValue) -> bool {
        value
            .fresh_references
            .iter()
            .any(|root| self.mutated_fresh_roots.contains(root))
    }

    fn allocate_fresh_container(&mut self, root: u32, payload: AbstractValue) -> AbstractValue {
        let root = FreshRoot::allocation(root);
        let mut payload = payload;
        payload.needs_fresh_root = false;
        self.heap.insert(root.clone(), payload.clone());
        let mut container = payload.with_fresh_container(true);
        container.fresh_roots.clear();
        container.fresh_roots.insert(root.clone());
        container.fresh_references.insert(root);
        container.needs_fresh_root = false;
        container
    }

    fn project_fresh_root(root: &FreshRoot, path: &ValueProjectionPath) -> Result<FreshRoot, ()> {
        let mut parent = root.clone();
        for step in path.steps() {
            parent = parent.project_step(step.clone())?;
        }
        Ok(parent)
    }

    fn projected_reference_sets(
        &mut self,
        value: &AbstractValue,
        path: &ValueProjectionPath,
    ) -> Result<(BTreeSet<FreshRoot>, BTreeSet<CallerReference>), ()> {
        let mut fresh_roots = BTreeSet::new();
        for root in &value.fresh_roots {
            fresh_roots.insert(Self::project_fresh_root(root, path)?);
        }
        let caller_references = value
            .direct_caller_references
            .iter()
            .map(|reference| reference.project(path))
            .collect::<Result<_, _>>()?;
        Ok((fresh_roots, caller_references))
    }

    fn project_value(
        &mut self,
        value: &AbstractValue,
        path: &ValueProjectionPath,
        reference: bool,
        preserve_caller_references: bool,
    ) -> AbstractValue {
        let mut projected = self.materialize_heap_value(value);
        for root in &value.fresh_roots {
            projected.fresh_roots.remove(root);
            projected.fresh_references.remove(root);
        }
        for caller_reference in &value.direct_caller_references {
            projected.caller_references.remove(caller_reference);
            projected.direct_caller_references.remove(caller_reference);
        }
        let roots = self.projected_reference_sets(value, path);
        let origins = projected
            .project_direct_caller_parameter_origins(path, &value.direct_caller_references);
        let (fresh_roots, caller_references) = match (roots, origins) {
            (Ok(roots), Ok(())) => roots,
            _ => {
                self.state.join(&CallableState::fail_closed(
                    CallableProvenanceUnknownReason::UnsupportedHeapStore,
                ));
                return AbstractValue::unknown(reference);
            }
        };
        // The heap is field-insensitive today, so the immediate payload of
        // each possible receiver root remains a conservative direct candidate
        // for the selected field. Do not promote every transitively reachable
        // root: doing so loses the distinction between the selected object and
        // values nested inside it, and manufactures false heap cycles.
        let mut direct_origins = projected.direct_origins.clone();
        let mut direct_fresh_roots = fresh_roots;
        let mut direct_caller_references = caller_references;
        let mut needs_fresh_root = projected.needs_fresh_root;
        for payload in value
            .fresh_roots
            .iter()
            .filter_map(|root| self.heap.get(root))
            .chain(
                value
                    .direct_caller_references
                    .iter()
                    .filter_map(|reference| self.state.parameter_stores.get(reference)),
            )
        {
            direct_origins.extend(payload.direct_origins.iter().cloned());
            direct_fresh_roots.extend(payload.fresh_roots.iter().cloned());
            direct_caller_references.extend(payload.direct_caller_references.iter().cloned());
            needs_fresh_root |= payload.needs_fresh_root;
        }
        projected.direct_origins = direct_origins;
        projected.fresh_roots = direct_fresh_roots;
        projected
            .fresh_references
            .extend(projected.fresh_roots.iter().cloned());
        if preserve_caller_references {
            projected
                .caller_references
                .extend(direct_caller_references.iter().cloned());
            projected.direct_caller_references = direct_caller_references;
        } else {
            projected.caller_references.clear();
            projected.direct_caller_references.clear();
        }
        projected.needs_fresh_root = needs_fresh_root;
        projected.catch_result = None;
        projected.reference = reference;
        if !reference {
            projected.fresh_roots.clear();
            projected.fresh_references.clear();
            projected.caller_references.clear();
            projected.direct_caller_references.clear();
            projected.needs_fresh_root = false;
        }
        projected
    }

    fn project_store_target(
        &mut self,
        actual: &AbstractValue,
        formal: &CallerReference,
    ) -> AbstractValue {
        if formal.path.is_empty() {
            return actual.clone();
        }
        let Ok(path) = ValueProjectionPath::new(formal.path.clone()) else {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnsupportedHeapStore,
            ));
            return AbstractValue::unknown(true);
        };
        let Ok((fresh_roots, caller_references)) = self.projected_reference_sets(actual, &path)
        else {
            self.state.join(&CallableState::fail_closed(
                CallableProvenanceUnknownReason::UnsupportedHeapStore,
            ));
            return AbstractValue::unknown(true);
        };
        let fresh_references = fresh_roots.clone();
        AbstractValue {
            fresh_roots,
            fresh_references,
            caller_references: caller_references.clone(),
            direct_caller_references: caller_references,
            reference: true,
            unknown: actual.unknown,
            ..AbstractValue::default()
        }
    }

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
        TypeRefIr::Builtin { name, args } => {
            args.is_empty()
                && matches!(
                    name.as_str(),
                    "string" | "integer" | "number" | "bool" | "null" | "void" | "never"
                )
        }
        TypeRefIr::Literal { .. } => true,
        TypeRefIr::Union { items } => items.iter().all(type_ir_is_definitely_scalar),
        TypeRefIr::Nullable { inner } => type_ir_is_definitely_scalar(inner),
        TypeRefIr::AppliedNominal { arguments, .. } => {
            let _arguments_are_scalar = arguments.iter().all(type_ir_is_definitely_scalar);
            false
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
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
