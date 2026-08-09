//! Source-owned bracket access facts retained outside File IR.

use std::collections::BTreeMap;

use skiff_artifact_model::{SourceSpanRef, TypeRefIr};
use skiff_compiler_source::{SourceIndexPolicy, SourceIndexReceiverKind, SourceIndexSegmentFact};

/// Compiler-known container selected for one bracket segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirIndexReceiverKind {
    Array,
    Map,
    JsonObject,
}

/// Exact read/write/loan semantics of one bracket segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirIndexPolicy {
    StrictRead,
    IntermediateMustExist,
    TerminalReplace,
    TerminalUpsert,
    LoanMustExist,
}

/// One source-owned bracket segment, keyed in a function by its selector
/// expression index. The selector expression is evaluated exactly once and
/// remains owned by the same [`super::MirFunction`].
#[derive(Debug, Clone, PartialEq)]
pub struct MirIndexAccessFacts {
    pub receiver_kind: MirIndexReceiverKind,
    pub receiver_type: TypeRefIr,
    pub selector_type: TypeRefIr,
    pub result_type: TypeRefIr,
    pub policy: MirIndexPolicy,
    pub source_span: SourceSpanRef,
}

impl MirIndexAccessFacts {
    pub(crate) fn from_source(fact: &SourceIndexSegmentFact) -> Self {
        Self {
            receiver_kind: match fact.receiver_kind {
                SourceIndexReceiverKind::Array => MirIndexReceiverKind::Array,
                SourceIndexReceiverKind::Map => MirIndexReceiverKind::Map,
                SourceIndexReceiverKind::JsonObject => MirIndexReceiverKind::JsonObject,
            },
            receiver_type: fact.receiver_type.clone(),
            selector_type: fact.selector_type.clone(),
            result_type: fact.result_type.clone(),
            policy: match fact.policy {
                SourceIndexPolicy::StrictRead => MirIndexPolicy::StrictRead,
                SourceIndexPolicy::IntermediateMustExist => MirIndexPolicy::IntermediateMustExist,
                SourceIndexPolicy::TerminalReplace => MirIndexPolicy::TerminalReplace,
                SourceIndexPolicy::TerminalUpsert => MirIndexPolicy::TerminalUpsert,
                SourceIndexPolicy::LoanMustExist => MirIndexPolicy::LoanMustExist,
            },
            source_span: crate::source_unit_lowering::source_span_ref(fact.source_span),
        }
    }
}

/// Lowering-owned sidecar used only while File IR is finalized and MIR is
/// constructed. Every entry is moved into its owning [`super::MirFunction`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MirSourceFacts {
    index_accesses: BTreeMap<(String, u32), BTreeMap<u32, MirIndexAccessFacts>>,
}

impl MirSourceFacts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn index_accesses(
        &self,
        module_path: &str,
        executable_index: u32,
    ) -> Option<&BTreeMap<u32, MirIndexAccessFacts>> {
        self.index_accesses
            .get(&(module_path.to_string(), executable_index))
    }

    pub(crate) fn insert_executable(
        &mut self,
        module_path: &str,
        executable_index: u32,
        facts: BTreeMap<u32, MirIndexAccessFacts>,
    ) -> Result<(), String> {
        if self
            .index_accesses
            .insert((module_path.to_string(), executable_index), facts)
            .is_some()
        {
            return Err(format!(
                "duplicate MIR source-fact owner `{module_path}` executable {executable_index}"
            ));
        }
        Ok(())
    }

    pub(crate) fn extend(&mut self, other: Self) -> Result<(), String> {
        for ((module_path, executable_index), facts) in other.index_accesses {
            self.insert_executable(&module_path, executable_index, facts)?;
        }
        Ok(())
    }

    pub(crate) fn index_accesses_mut(
        &mut self,
    ) -> impl Iterator<Item = (&(String, u32), &mut BTreeMap<u32, MirIndexAccessFacts>)> {
        self.index_accesses.iter_mut()
    }

    pub(crate) fn owners(
        &self,
    ) -> impl Iterator<Item = (&(String, u32), &BTreeMap<u32, MirIndexAccessFacts>)> {
        self.index_accesses.iter()
    }
}
