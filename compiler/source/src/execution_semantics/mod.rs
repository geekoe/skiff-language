use crate::{
    parsed_sources::ParsedCompilerSource, ExpressionOwnerKey, ExpressionSourceMap,
    ResolvedCallTargetFacts, SourceCallableEffectFacts, SourceCompileError,
};

mod collectors;
mod model;
mod mutation;
mod owner;

pub use model::{
    ConcurrentLaneKind, ConcurrentLanePlan, ConcurrentSourcePlan, ExecutionSourceSite,
    SourceExecutionSemantics, TimeoutSourcePlan,
};

use collectors::{
    callable_definitions, expression_key_index, reject_static_execution_scopes,
    top_level_value_names,
};
use owner::OwnerAnalyzer;

pub(crate) fn analyze_source_execution_semantics(
    parsed_sources: &[ParsedCompilerSource],
    expression_sources: &ExpressionSourceMap,
    resolved_targets: &ResolvedCallTargetFacts,
    callable_effects: &SourceCallableEffectFacts,
) -> Result<SourceExecutionSemantics, SourceCompileError> {
    let definitions = callable_definitions(parsed_sources);
    let expression_keys = expression_key_index(&definitions);
    let mut semantics = SourceExecutionSemantics::default();
    let mut diagnostics = Vec::new();
    reject_static_execution_scopes(parsed_sources, &mut diagnostics);

    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        let module_path = parsed.module_path();
        let top_level_value_names = top_level_value_names(parsed.ast());
        for function in &parsed.ast().functions {
            if function.is_native || function.is_provider {
                continue;
            }
            let owner = ExpressionOwnerKey::Function(function.name.clone());
            let mut analyzer = OwnerAnalyzer::new(
                module_path,
                owner,
                function,
                expression_sources,
                &expression_keys,
                resolved_targets,
                callable_effects,
                top_level_value_names.clone(),
                &mut semantics,
                &mut diagnostics,
            );
            analyzer.analyze();
        }
        for implementation in &parsed.ast().impls {
            for method in &implementation.method_bodies {
                if method.is_native || method.is_provider {
                    continue;
                }
                let owner = ExpressionOwnerKey::ImplMethod {
                    type_name: implementation.target.clone(),
                    method: method.name.clone(),
                };
                let mut analyzer = OwnerAnalyzer::new(
                    module_path,
                    owner,
                    method,
                    expression_sources,
                    &expression_keys,
                    resolved_targets,
                    callable_effects,
                    top_level_value_names.clone(),
                    &mut semantics,
                    &mut diagnostics,
                );
                analyzer.analyze();
            }
        }
    }

    if diagnostics.is_empty() {
        semantics.validate_complete()?;
        Ok(semantics)
    } else {
        diagnostics.sort();
        diagnostics.dedup();
        Err(SourceCompileError::ContractValidation {
            message: format!(
                "source execution semantics failed:\n{}",
                diagnostics
                    .into_iter()
                    .map(|diagnostic| format!("- {diagnostic}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        })
    }
}
