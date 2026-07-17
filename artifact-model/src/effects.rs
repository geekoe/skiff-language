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
mod tests {
    use serde_json::json;

    use super::*;

    fn analyzed_effects() -> CallableMayEffects {
        CallableMayEffects {
            writes_caller_reachable: false,
            returns_caller_alias: true,
            throws_caller_alias: false,
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: true,
            may_suspend: false,
        }
    }

    #[test]
    fn unknown_round_trips_with_reason_and_is_not_boundary_available() {
        let summary = CallableEffectSummary::analysis_pending();
        let value = serde_json::to_value(&summary).expect("serialize unknown effects");
        assert_eq!(
            value,
            json!({ "kind": "unknown", "reason": "analysisPending" })
        );
        assert_eq!(
            serde_json::from_value::<CallableEffectSummary>(value)
                .expect("deserialize unknown effects"),
            summary
        );
        assert_eq!(
            summary.effects_for_boundary(),
            Err(CallableEffectUnknownReason::AnalysisPending)
        );
    }

    #[test]
    fn analyzed_normal_return_and_throw_alias_are_independent() {
        let summary = CallableEffectSummary::Analyzed {
            effects: analyzed_effects(),
        };
        let effects = summary
            .effects_for_boundary()
            .expect("analyzed effects are available");
        assert!(effects.returns_caller_alias);
        assert!(!effects.throws_caller_alias);

        let mut throw_alias = analyzed_effects();
        throw_alias.returns_caller_alias = false;
        throw_alias.throws_caller_alias = true;
        let throw_summary = CallableEffectSummary::Analyzed {
            effects: throw_alias,
        };
        let effects = throw_summary
            .effects_for_boundary()
            .expect("analyzed effects are available");
        assert!(!effects.returns_caller_alias);
        assert!(effects.throws_caller_alias);
    }

    #[test]
    fn typed_wire_rejects_missing_tags_fields_and_unknown_fields() {
        for invalid in [
            json!({ "reason": "analysisPending" }),
            json!({ "kind": "unknown" }),
            json!({
                "kind": "unknown",
                "reason": "analysisPending",
                "detail": "diagnostic text must not enter semantic bytes"
            }),
            json!({
                "kind": "analyzed",
                "effects": {
                    "writesCallerReachable": false,
                    "returnsCallerAlias": false,
                    "throwsCallerAlias": false,
                    "escapesCallerValue": false,
                    "requiresSameHeapIdentity": false,
                    "invokesUnknownTarget": false
                }
            }),
            json!({
                "kind": "analyzed",
                "effects": {
                    "writesCallerReachable": false,
                    "returnsCallerAlias": false,
                    "throwsCallerAlias": false,
                    "escapesCallerValue": false,
                    "requiresSameHeapIdentity": false,
                    "invokesUnknownTarget": false,
                    "maySuspend": false,
                    "futureField": false
                }
            }),
        ] {
            assert!(
                serde_json::from_value::<CallableEffectSummary>(invalid).is_err(),
                "invalid typed effect wire must be rejected"
            );
        }
    }

    #[test]
    fn callable_effect_facts_require_the_operation_map() {
        assert!(serde_json::from_value::<CallableEffectFacts>(json!({})).is_err());
        assert!(serde_json::from_value::<CallableEffectFacts>(json!({
            "operations": {},
            "unknown": true
        }))
        .is_err());
    }

    #[test]
    fn transport_envelope_requires_the_typed_effect_owner() {
        assert!(
            serde_json::from_value::<crate::ConfigAndEffectMetadata>(json!({ "config": {} }))
                .is_err()
        );
    }
}
