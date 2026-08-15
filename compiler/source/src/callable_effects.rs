use std::collections::BTreeMap;

use skiff_artifact_model::{CallableEffectSummary, CallableProvenanceSummary};
use skiff_compiler_core::source_role::PublicationSourceRole;

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
    pub role: PublicationSourceRole,
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

/// Initializes the compiler-owned platform facts used by source callable
/// analysis from an explicit, validated repository root.
///
/// Production driver entrypoints already own this initialization. This
/// path-based adapter exists so downstream compiler-crate tests can exercise
/// the same parser, semantic model and lowering without inventing MIR or
/// depending on driver-private publication helpers.
#[doc(hidden)]
pub fn initialize_platform_for_compiler_test(
    platform_root: &std::path::Path,
) -> Result<(), String> {
    let sources = skiff_compiler_input::CompilerPlatformSources::new(platform_root)
        .map_err(|error| error.to_string())?;
    crate::prelude_registry::initialize_prelude_registry(&sources)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
