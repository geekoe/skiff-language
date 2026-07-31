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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableMayEffects {
    pub writes_caller_reachable: bool,
    pub returns_caller_alias: bool,
    pub throws_caller_alias: bool,
    pub escapes_caller_value: bool,
    pub requires_same_heap_identity: bool,
    pub invokes_unknown_target: bool,
    pub may_suspend: bool,
}

#[cfg(test)]
mod tests;
