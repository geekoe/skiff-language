use std::collections::BTreeMap;

use skiff_artifact_model::{InstructionSourceSite, StatementAttributionClass};

use crate::{
    parsed_sources::ParsedCompilerSource, ExpressionKey, ExpressionOwnerKey, ExpressionSourceMap,
};

mod collector;
mod expressions;
mod owners;
mod spans;

#[cfg(test)]
mod tests;

type SourceOwnerInventory = BTreeMap<(String, ExpressionOwnerKey), u32>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceStatementKey {
    module_path: String,
    owner: ExpressionOwnerKey,
    statement_preorder_index: u32,
}

impl SourceStatementKey {
    /// Creates an owner-local source-AST statement coordinate. This preorder
    /// is independent of expression preorder and is not a File IR index.
    pub fn new(
        module_path: impl Into<String>,
        owner: ExpressionOwnerKey,
        statement_preorder_index: u32,
    ) -> Self {
        Self {
            module_path: module_path.into(),
            owner,
            statement_preorder_index,
        }
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    pub fn owner(&self) -> &ExpressionOwnerKey {
        &self.owner
    }

    pub fn statement_preorder_index(&self) -> u32 {
        self.statement_preorder_index
    }
}

/// Typed source identity consumed before File IR indexes exist.
///
/// The variant is the attribution class authority. Source collection has no
/// generated variant; compiler-generated events are introduced downstream
/// with an explicit synthetic site.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceEventKey {
    Statement(SourceStatementKey),
    Expression(ExpressionKey),
}

impl SourceEventKey {
    pub const fn attribution_class(&self) -> StatementAttributionClass {
        match self {
            Self::Statement(_) => StatementAttributionClass::Statement,
            Self::Expression(_) => StatementAttributionClass::Expression,
        }
    }

    pub fn module_path(&self) -> &str {
        match self {
            Self::Statement(key) => key.module_path(),
            Self::Expression(key) => key.module_path(),
        }
    }

    pub fn owner(&self) -> &ExpressionOwnerKey {
        match self {
            Self::Statement(key) => key.owner(),
            Self::Expression(key) => key.owner(),
        }
    }

    pub fn preorder_index(&self) -> u32 {
        match self {
            Self::Statement(key) => key.statement_preorder_index(),
            Self::Expression(key) => key.preorder_index(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEventFact {
    key: SourceEventKey,
    site: InstructionSourceSite,
}

impl SourceEventFact {
    pub fn key(&self) -> &SourceEventKey {
        &self.key
    }

    pub fn site(&self) -> &InstructionSourceSite {
        &self.site
    }
}

#[derive(Clone, Debug)]
pub struct SourceEventFacts {
    owners: SourceOwnerInventory,
    facts: BTreeMap<SourceEventKey, SourceEventFact>,
    expression_sources: ExpressionSourceMap,
}

impl SourceEventFacts {
    /// Walks each source owner once and records both statement and expression
    /// facts while preserving the existing expression-only preorder.
    pub fn build(parsed_sources: &[ParsedCompilerSource]) -> Result<Self, String> {
        collector::collect_source_events(parsed_sources)
    }

    pub fn fact(&self, key: &SourceEventKey) -> Option<&SourceEventFact> {
        self.facts.get(key)
    }

    /// Reports whether source collection observed this exact module-local
    /// owner exactly once, including unique owners whose bodies contain no
    /// source events. Missing and ambiguous owners both fail closed.
    pub fn contains_owner(&self, module_path: &str, owner: &ExpressionOwnerKey) -> bool {
        self.owners.get(&(module_path.to_string(), owner.clone())) == Some(&1)
    }

    /// Iterates in typed key order for deterministic validation. This order is
    /// not an execution order and must not be used as a File IR index.
    pub fn iter(&self) -> impl Iterator<Item = &SourceEventFact> {
        self.facts.values()
    }

    pub fn is_empty(&self) -> bool {
        self.owners.is_empty() && self.facts.is_empty()
    }

    pub(crate) fn expression_sources(&self) -> &ExpressionSourceMap {
        &self.expression_sources
    }

    pub(crate) fn into_expression_sources(self) -> ExpressionSourceMap {
        self.expression_sources
    }
}
