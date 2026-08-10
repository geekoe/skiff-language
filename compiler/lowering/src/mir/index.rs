//! Source-owned bracket access facts retained outside File IR.

use std::collections::BTreeMap;

use skiff_artifact_model::{SourceSpanRef, TypeRefIr};
use skiff_compiler_source::{SourceIndexPolicy, SourceIndexReceiverKind, SourceIndexSegmentFact};

use super::MirSourceEventPlan;

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
    source_event_plans: BTreeMap<(String, u32), MirSourceEventPlan>,
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

    pub fn source_event_plan(
        &self,
        module_path: &str,
        executable_index: u32,
    ) -> Option<&MirSourceEventPlan> {
        self.source_event_plans
            .get(&(module_path.to_string(), executable_index))
    }

    pub(crate) fn insert_executable(
        &mut self,
        module_path: &str,
        executable_index: u32,
        facts: BTreeMap<u32, MirIndexAccessFacts>,
        source_event_plan: MirSourceEventPlan,
    ) -> Result<(), String> {
        let owner = (module_path.to_string(), executable_index);
        if self.index_accesses.contains_key(&owner) || self.source_event_plans.contains_key(&owner)
        {
            return Err(format!(
                "duplicate MIR source-fact owner `{module_path}` executable {executable_index}"
            ));
        }
        if self.index_accesses.insert(owner.clone(), facts).is_some() {
            return Err(format!(
                "duplicate MIR source-fact owner `{module_path}` executable {executable_index}"
            ));
        }
        self.source_event_plans.insert(owner, source_event_plan);
        Ok(())
    }

    pub(crate) fn extend(&mut self, other: Self) -> Result<(), String> {
        let Self {
            index_accesses,
            source_event_plans,
        } = other;
        if !index_accesses.keys().eq(source_event_plans.keys()) {
            return Err(
                "MIR source facts and source event plans have different executable owners"
                    .to_string(),
            );
        }
        for ((module_path, executable_index), facts) in index_accesses {
            let owner = (module_path.clone(), executable_index);
            let source_event_plan = source_event_plans.get(&owner).cloned().ok_or_else(|| {
                format!(
                    "MIR source-fact owner `{module_path}` executable {executable_index} has no source event plan"
                )
            })?;
            self.insert_executable(&module_path, executable_index, facts, source_event_plan)?;
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

    pub(crate) fn event_plan_owners(
        &self,
    ) -> impl Iterator<Item = (&(String, u32), &MirSourceEventPlan)> {
        self.source_event_plans.iter()
    }
}
