use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Sound callable effect facts keyed by stable operation ABI identity.
///
/// The map itself is always present in the artifact envelope. An empty map
/// therefore means that the owning surface has no operations, not that effect
/// analysis was omitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableEffectFacts {
    pub operations: BTreeMap<String, CallableEffectSummary>,
}

impl CallableEffectFacts {
    pub fn from_operations(operations: BTreeMap<String, CallableEffectSummary>) -> Self {
        Self { operations }
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

/// A callable either has a completed sound may-analysis or carries an explicit
/// reason why no such result is available yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CallableEffectSummary {
    Unknown { reason: CallableEffectUnknownReason },
    Analyzed { effects: CallableMayEffects },
}

impl CallableEffectSummary {
    pub const fn analysis_pending() -> Self {
        Self::Unknown {
            reason: CallableEffectUnknownReason::AnalysisPending,
        }
    }

    /// Boundary consumers must use this fallible accessor. Unknown never
    /// becomes an empty/safe effect set by default.
    pub const fn effects_for_boundary(
        &self,
    ) -> Result<&CallableMayEffects, CallableEffectUnknownReason> {
        match self {
            Self::Unknown { reason } => Err(*reason),
            Self::Analyzed { effects } => Ok(effects),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallableEffectUnknownReason {
    AnalysisPending,
}

/// Sound may-effects. Every field is required on the wire: adding or omitting
/// a field cannot silently grant a boundary optimization.
///
/// The three aggregate alias flags (`writesCallerReachable`,
/// `returnsCallerAlias`, `throwsCallerAlias`) were retired: ordinary aggregate
/// parameters/returns/throw payloads are logical snapshots, so only explicit
/// InOut paths (not yet exercised) write caller places. Identity-bearing
/// escape and heap-identity facts remain, and `mayPending` replaces the old
/// `maySuspend` with an explicit effect-category trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableMayEffects {
    /// Identity-bearing (resource/capability) values escape into an external
    /// lane. Aggregates never set this.
    pub escapes_caller_value: bool,
    /// An identity-sensitive operation observed a caller-owned value.
    pub requires_same_heap_identity: bool,
    /// The callable may invoke a target that is not statically resolved.
    pub invokes_unknown_target: bool,
    /// The callable may suspend/park the current execution. Semantically
    /// equivalent to `!pending_effect_categories.is_empty()`; the field is
    /// kept on the wire as a required, cheaply assertable fact.
    pub may_pending: bool,
    /// Trace of the pending effect categories observed. Empty means the
    /// callable never suspends. Order is deterministic and deduplicated.
    pub pending_effect_categories: Vec<PendingEffectCategory>,
    /// Per-parameter InOut path effects (read/write selector paths). Not yet
    /// exercised by the Phase 2 compiler; always empty for now.
    pub inout_path_effects: Vec<InOutPathEffect>,
}

impl CallableMayEffects {
    /// Semantic definition of pending: any recorded pending category makes the
    /// callable potentially suspending.
    pub fn may_pending(&self) -> bool {
        !self.pending_effect_categories.is_empty()
    }
}

/// Why a callable may park/suspend the current execution. Unknown is the
/// conservative catch-all for dynamic, unresolved, or not-yet-classified
/// targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingEffectCategory {
    ServiceCall,
    ActorCall,
    InterfaceCall,
    NativeCall,
    Stream,
    HostEffect,
    Unknown,
}

/// One parameter's InOut path effects: which selector paths of the parameter
/// are read and which are written by the callable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InOutPathEffect {
    pub parameter_index: u32,
    pub read: Vec<SelectorPath>,
    pub write: Vec<SelectorPath>,
}

/// A typed selector path inside an InOut parameter (field / index sequence).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SelectorPath(pub Vec<SelectorPathSegment>);

impl SelectorPath {
    pub fn steps(&self) -> &[SelectorPathSegment] {
        &self.0
    }
}

/// One step of a selector path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SelectorPathSegment {
    Field { name: String },
    Index {},
}

#[cfg(test)]
mod tests;
