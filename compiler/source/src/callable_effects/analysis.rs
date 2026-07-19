use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableEffectSummary, CallableProvenanceSummary, CallableProvenanceUnknownReason,
};

use crate::{
    parsed_sources::ParsedCompilerSource, semantic::impl_method_declaration_name,
    shared::type_syntax::generic_type_parameter_names, ExpressionTypeModel,
    ResolvedCallTargetFacts, SourceDependencyAnalysisInput, SourceSymbolKey, TypeResolutionModel,
};

use super::{
    call_graph::LocalCallGraph, provenance::CallableState, transfer::transfer_callable,
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
    let graph = LocalCallGraph::build(definitions.keys().cloned(), resolved_call_targets);
    let mut states = BTreeMap::new();

    // Local seeds contain only syntax/provenance transfer and canonical
    // dependency facts. A missing local summary is lattice bottom here; local
    // propagation happens exclusively through the SCC loop below.
    for (key, definition) in &definitions {
        if definition.is_test_source {
            continue;
        }
        states.insert(
            key.clone(),
            transfer_callable(
                definition,
                &definitions,
                &BTreeMap::new(),
                resolved_call_targets,
                dependency_analysis,
                expression_types,
                type_resolution,
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
                    &definitions,
                    &states,
                    resolved_call_targets,
                    dependency_analysis,
                    expression_types,
                    type_resolution,
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
