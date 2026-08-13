use serde_json::json;

use super::*;

fn analyzed_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: true,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
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
fn analyzed_pending_categories_round_trip_and_drive_may_pending() {
    let effects = CallableMayEffects {
        may_pending: true,
        pending_effect_categories: vec![
            PendingEffectCategory::ServiceCall,
            PendingEffectCategory::Unknown,
        ],
        inout_path_effects: vec![InOutPathEffect {
            parameter_index: 1,
            read: vec![SelectorPath(vec![SelectorPathSegment::Field {
                name: "data".to_string(),
            }])],
            write: Vec::new(),
        }],
        ..analyzed_effects()
    };
    let summary = CallableEffectSummary::Analyzed { effects };
    let value = serde_json::to_value(&summary).expect("serialize analyzed effects");
    assert_eq!(
        value,
        json!({
            "kind": "analyzed",
            "effects": {
                "escapesCallerValue": false,
                "requiresSameHeapIdentity": false,
                "invokesUnknownTarget": true,
                "mayPending": true,
                "pendingEffectCategories": ["serviceCall", "unknown"],
                "inoutPathEffects": [{
                    "parameterIndex": 1,
                    "read": [[{ "kind": "field", "name": "data" }]],
                    "write": []
                }]
            }
        })
    );
    let decoded = serde_json::from_value::<CallableEffectSummary>(value)
        .expect("deserialize analyzed effects");
    let effects = decoded
        .effects_for_boundary()
        .expect("analyzed effects are available");
    assert!(effects.may_pending);
    assert!(effects.may_pending());
    assert_eq!(
        effects.pending_effect_categories,
        vec![
            PendingEffectCategory::ServiceCall,
            PendingEffectCategory::Unknown
        ]
    );
    assert_eq!(
        effects.inout_path_effects[0].read[0].steps(),
        &[SelectorPathSegment::Field {
            name: "data".to_string()
        }]
    );
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
        // Old aggregate flags are rejected (retired from the wire).
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
                "mayPending": false,
                "pendingEffectCategories": [],
                "inoutPathEffects": []
            }
        }),
        // Old maySuspend field is rejected even alone.
        json!({
            "kind": "analyzed",
            "effects": {
                "escapesCallerValue": false,
                "requiresSameHeapIdentity": false,
                "invokesUnknownTarget": false,
                "maySuspend": false,
                "mayPending": false,
                "pendingEffectCategories": [],
                "inoutPathEffects": []
            }
        }),
        // Every new field is required on the wire.
        json!({
            "kind": "analyzed",
            "effects": {
                "escapesCallerValue": false,
                "requiresSameHeapIdentity": false,
                "invokesUnknownTarget": false,
                "mayPending": false,
                "pendingEffectCategories": []
            }
        }),
        json!({
            "kind": "analyzed",
            "effects": {
                "escapesCallerValue": false,
                "requiresSameHeapIdentity": false,
                "invokesUnknownTarget": false,
                "mayPending": false,
                "pendingEffectCategories": [],
                "inoutPathEffects": [],
                "futureField": false
            }
        }),
        json!({
            "kind": "analyzed",
            "effects": {
                "escapesCallerValue": false,
                "requiresSameHeapIdentity": false,
                "invokesUnknownTarget": false,
                "mayPending": true,
                "pendingEffectCategories": ["unknown", "futureCategory"],
                "inoutPathEffects": []
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
        serde_json::from_value::<crate::ConfigAndEffectMetadata>(json!({ "config": {} })).is_err()
    );
}
