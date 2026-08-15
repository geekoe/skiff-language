use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableEffectSummary, CallableProvenanceSummary, CallableProvenanceUnknownReason,
};

use crate::{
    parsed_sources::ParsedCompilerSource,
    semantic::impl_method_declaration_name,
    shared::{ast::Expr, type_syntax::generic_type_parameter_names},
    ExpressionTypeModel, ResolvedCallTargetFacts, SourceDependencyAnalysisInput, SourceSymbolKey,
    TypeResolutionModel,
};

use super::{
    call_graph::LocalCallGraph,
    provenance::CallableState,
    transfer::{transfer_callable, TransferContext},
    CallableDefinition, SourceCallableAnalysis, SourceCallableEffectFacts,
    SourceCallableProvenanceFacts,
};

pub(crate) fn analyze_source_callables(
    parsed_sources: &[ParsedCompilerSource],
    resolved_call_targets: &ResolvedCallTargetFacts,
    dependency_analysis: &SourceDependencyAnalysisInput,
    expression_types: &ExpressionTypeModel,
    type_resolution: &TypeResolutionModel,
) -> SourceCallableAnalysis {
    let definitions = callable_definitions(parsed_sources);
    let module_constants = module_constant_facts(parsed_sources);
    let graph = LocalCallGraph::build(definitions.keys().cloned(), resolved_call_targets);
    let mut states = BTreeMap::new();

    // Local seeds contain only syntax/provenance transfer and canonical
    // dependency facts. A missing local summary is lattice bottom here; local
    // propagation happens exclusively through the SCC loop below.
    let empty_summaries = BTreeMap::new();
    for (key, definition) in &definitions {
        if definition.is_test_source {
            continue;
        }
        states.insert(
            key.clone(),
            transfer_callable(
                definition,
                &TransferContext {
                    definitions: &definitions,
                    module_constants: &module_constants,
                    summaries: &empty_summaries,
                    resolved_call_targets,
                    dependency_analysis,
                    expression_types,
                    type_resolution,
                },
            ),
        );
    }

    for component in graph.callee_first_sccs() {
        let mut iteration = 0usize;
        loop {
            iteration += 1;
            let mut changed = false;
            for key in &component {
                let definition = &definitions[key];
                if definition.is_test_source {
                    continue;
                }
                let candidate = transfer_callable(
                    definition,
                    &TransferContext {
                        definitions: &definitions,
                        module_constants: &module_constants,
                        summaries: &states,
                        resolved_call_targets,
                        dependency_analysis,
                        expression_types,
                        type_resolution,
                    },
                );
                changed |= states
                    .entry(key.clone())
                    .or_insert_with(CallableState::bottom)
                    .join(&candidate);
            }
            if !changed {
                break;
            }
            if iteration > 1024 {
                for key in &component {
                    states
                        .entry(key.clone())
                        .or_insert_with(CallableState::bottom)
                        .join(&CallableState::fail_closed(
                            CallableProvenanceUnknownReason::UnsupportedControlFlow,
                        ));
                }
                break;
            }
        }
    }

    let mut effects = BTreeMap::new();
    let mut provenance = BTreeMap::new();
    for (key, definition) in definitions {
        if definition.is_test_source {
            effects.insert(key.clone(), CallableEffectSummary::analysis_pending());
            provenance.insert(
                key,
                CallableProvenanceSummary::Unknown {
                    reason: CallableProvenanceUnknownReason::AnalysisPending,
                },
            );
            continue;
        }
        let (effect, origin) = states
            .remove(&key)
            .unwrap_or_else(|| {
                CallableState::fail_closed(CallableProvenanceUnknownReason::UnknownCallTarget)
            })
            .into_summaries();
        effects.insert(key.clone(), effect);
        provenance.insert(key, origin);
    }

    SourceCallableAnalysis {
        effects: SourceCallableEffectFacts::from_operations(effects),
        provenance: SourceCallableProvenanceFacts::from_operations(provenance),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModuleConstantFact {
    Exact,
    Unsupported,
}

fn module_constant_facts(
    parsed_sources: &[ParsedCompilerSource],
) -> BTreeMap<SourceSymbolKey, ModuleConstantFact> {
    let declarations = parsed_sources
        .iter()
        .flat_map(|parsed| {
            parsed.ast().consts.iter().map(move |constant| {
                (
                    SourceSymbolKey::new(parsed.module_path(), &constant.name),
                    &constant.value,
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut facts = BTreeMap::new();
    for key in declarations.keys() {
        resolve_module_constant(key, &declarations, &mut facts, &mut Vec::new());
    }
    facts
}

fn resolve_module_constant(
    key: &SourceSymbolKey,
    declarations: &BTreeMap<SourceSymbolKey, &Expr>,
    facts: &mut BTreeMap<SourceSymbolKey, ModuleConstantFact>,
    stack: &mut Vec<SourceSymbolKey>,
) -> ModuleConstantFact {
    if let Some(fact) = facts.get(key) {
        return *fact;
    }
    if stack.contains(key) {
        return ModuleConstantFact::Unsupported;
    }
    let Some(initializer) = declarations.get(key) else {
        return ModuleConstantFact::Unsupported;
    };
    stack.push(key.clone());
    let fact = match initializer {
        Expr::Literal(_) => ModuleConstantFact::Exact,
        Expr::Identifier(name) => {
            let dependency = SourceSymbolKey::new(key.module_path(), name);
            resolve_module_constant(&dependency, declarations, facts, stack)
        }
        _ => ModuleConstantFact::Unsupported,
    };
    stack.pop();
    facts.insert(key.clone(), fact);
    fact
}

pub(super) fn source_callable_keys(
    parsed_sources: &[ParsedCompilerSource],
) -> Vec<SourceSymbolKey> {
    callable_definitions(parsed_sources).into_keys().collect()
}

fn callable_definitions<'a>(
    parsed_sources: &'a [ParsedCompilerSource],
) -> BTreeMap<SourceSymbolKey, CallableDefinition<'a>> {
    let mut definitions = BTreeMap::new();
    for parsed in parsed_sources {
        let module_path = parsed.module_path();
        let is_test_source = parsed.source().is_test_file;
        for function in &parsed.ast().functions {
            let key = SourceSymbolKey::new(module_path, &function.name);
            definitions.insert(
                key.clone(),
                CallableDefinition {
                    key,
                    function,
                    module_path,
                    role: parsed.role(),
                    type_params: function.type_params.clone(),
                    is_test_source,
                },
            );
        }
        for implementation in &parsed.ast().impls {
            let inherited = generic_type_parameter_names(&implementation.target);
            for method in &implementation.method_bodies {
                let key = SourceSymbolKey::new(
                    module_path,
                    impl_method_declaration_name(&implementation.target, &method.name),
                );
                definitions.insert(
                    key.clone(),
                    CallableDefinition {
                        key,
                        function: method,
                        module_path,
                        role: parsed.role(),
                        type_params: inherited
                            .iter()
                            .chain(&method.type_params)
                            .cloned()
                            .collect(),
                        is_test_source,
                    },
                );
            }
        }
    }
    definitions
}
