use std::collections::BTreeMap;

use skiff_artifact_model::{CallableEffectSummary, CallableProvenanceSummary};

use crate::{parsed_sources::ParsedCompilerSource, SourceSymbolKey};

mod analysis;
mod call_graph;
mod provenance;
mod transfer;

#[cfg(test)]
mod tests;

/// Owner-local callable effect facts. Source symbols are retained only inside
/// source compilation; the compiled handoff resolves them to executable keys.
#[derive(Debug, Clone, Default)]
pub struct SourceCallableEffectFacts {
    operations: BTreeMap<SourceSymbolKey, CallableEffectSummary>,
}

/// Provenance facts paired with SourceCallableEffectFacts by the same stable
/// source owner key. Boundary projection must consume both facts.
#[derive(Debug, Clone, Default)]
pub struct SourceCallableProvenanceFacts {
    operations: BTreeMap<SourceSymbolKey, CallableProvenanceSummary>,
}

pub(crate) struct SourceCallableAnalysis {
    pub effects: SourceCallableEffectFacts,
    pub provenance: SourceCallableProvenanceFacts,
}

pub(super) struct CallableDefinition<'a> {
    pub key: SourceSymbolKey,
    pub function: &'a crate::shared::ast::FunctionDecl,
    pub module_path: &'a str,
    pub type_params: Vec<String>,
    pub is_test_source: bool,
}

impl CallableDefinition<'_> {
    pub fn has_receiver(&self) -> bool {
        !self.function.is_static
            && self.key.symbol().contains('.')
            && (self.function.implicit_self.is_some()
                || self
                    .function
                    .params
                    .first()
                    .is_some_and(|param| param.name == "self"))
    }
}

impl SourceCallableEffectFacts {
    /// Explicit diagnostic/test seed. Production model construction calls the
    /// fixed-point analyzer and never uses this placeholder.
    pub fn analysis_pending(parsed_sources: &[ParsedCompilerSource]) -> Self {
        let operations = analysis::source_callable_keys(parsed_sources)
            .into_iter()
            .map(|key| (key, CallableEffectSummary::analysis_pending()))
            .collect();
        Self { operations }
    }

    pub fn operations(&self) -> &BTreeMap<SourceSymbolKey, CallableEffectSummary> {
        &self.operations
    }

    pub(crate) fn from_operations(
        operations: BTreeMap<SourceSymbolKey, CallableEffectSummary>,
    ) -> Self {
        Self { operations }
    }
}

impl SourceCallableProvenanceFacts {
    pub fn operations(&self) -> &BTreeMap<SourceSymbolKey, CallableProvenanceSummary> {
        &self.operations
    }

    pub(crate) fn from_operations(
        operations: BTreeMap<SourceSymbolKey, CallableProvenanceSummary>,
    ) -> Self {
        Self { operations }
    }
}

pub(crate) use analysis::analyze_source_callables;
