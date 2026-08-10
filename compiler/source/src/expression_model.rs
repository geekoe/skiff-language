use std::collections::BTreeMap;

use crate::{
    parsed_sources::ParsedCompilerSource,
    shared::{ast::RecordFieldSourceSpans, error::SourceSpan},
    source_events::SourceEventFacts,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExpressionKey {
    module_path: String,
    owner: ExpressionOwnerKey,
    preorder_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExpressionOwnerKey {
    Function(String),
    ImplMethod { type_name: String, method: String },
    Const(String),
    Test(String),
    DbIndexWhere { db: String, index: String },
}

#[derive(Clone, Debug)]
pub struct ExpressionSourceFact {
    pub span: SourceSpan,
    pub record_fields: Vec<RecordFieldSourceSpans>,
}

#[derive(Clone, Debug, Default)]
pub struct ExpressionSourceMap {
    facts: BTreeMap<ExpressionKey, ExpressionSourceFact>,
}

impl ExpressionKey {
    pub fn new(
        module_path: impl Into<String>,
        owner: ExpressionOwnerKey,
        preorder_index: u32,
    ) -> Self {
        Self {
            module_path: module_path.into(),
            owner,
            preorder_index,
        }
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    pub fn owner(&self) -> &ExpressionOwnerKey {
        &self.owner
    }

    pub fn preorder_index(&self) -> u32 {
        self.preorder_index
    }
}

impl ExpressionSourceMap {
    pub fn build(parsed_sources: &[ParsedCompilerSource]) -> Result<Self, String> {
        SourceEventFacts::build(parsed_sources).map(SourceEventFacts::into_expression_sources)
    }

    pub fn fact(&self, key: &ExpressionKey) -> Option<&ExpressionSourceFact> {
        self.facts.get(key)
    }

    pub(crate) fn from_facts(facts: BTreeMap<ExpressionKey, ExpressionSourceFact>) -> Self {
        Self { facts }
    }

    #[cfg(test)]
    pub fn facts(&self) -> &BTreeMap<ExpressionKey, ExpressionSourceFact> {
        &self.facts
    }
}
