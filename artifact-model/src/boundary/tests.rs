use serde_json::json;

use crate::{CallableMayEffects, ContractOperationId, ContractTypeRef};

use super::*;

#[test]
fn boundary_lanes_require_explicit_tag_and_semantic_fields() {
    for invalid in [
        json!({
            "carrier": "detachedValueGraph",
            "encoding": "canonicalValue",
            "owner": "caller",
            "lifetime": "call"
        }),
        json!({
            "kind": "linkable",
            "carrier": "detachedValueGraph",
            "encoding": "canonicalValue",
            "owner": "caller"
        }),
        json!({
            "kind": "linkable",
            "carrier": "detachedValueGraph",
            "encoding": "canonicalValue",
            "owner": "caller",
            "lifetime": "call",
            "providerBuildId": "forbidden"
        }),
    ] {
        assert!(serde_json::from_value::<BoundaryValuePlan>(invalid).is_err());
    }

    assert_eq!(
        serde_json::to_value(BoundaryStreamContract::Unsupported {
            reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
        })
        .unwrap(),
        json!({ "kind": "unsupported", "reason": "languageUnsupported" })
    );
}

#[test]
fn unavailable_projection_requires_stable_non_optional_reason_field() {
    assert!(serde_json::from_value::<BoundaryCallableProjection>(json!({
        "kind": "unavailable"
    }))
    .is_err());
    assert!(serde_json::from_value::<BoundaryCallableProjection>(json!({
        "kind": "available",
        "operationContract": {}
    }))
    .is_err());
    assert_eq!(
        serde_json::to_value(BoundaryCallableProjection::Unavailable {
            reasons: vec![BoundaryUnavailableReason::UnknownCallTarget],
        })
        .unwrap(),
        json!({
            "kind": "unavailable",
            "reasons": [{ "kind": "unknownCallTarget" }]
        })
    );
}

#[test]
fn caller_projection_path_wire_is_structured_and_strict() {
    let provenance = ValueProvenance::CallerParameterProjection {
        index: 2,
        path: ValueProjectionPath::new(vec![
            ValueProjectionStep::Field {
                name: "state".to_string(),
            },
            ValueProjectionStep::ContainerElement {},
        ])
        .unwrap(),
    };
    let wire = json!({
        "kind": "callerParameterProjection",
        "index": 2,
        "path": {
            "steps": [
                { "kind": "field", "name": "state" },
                { "kind": "containerElement" }
            ]
        }
    });
    assert_eq!(serde_json::to_value(&provenance).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<ValueProvenance>(wire).unwrap(),
        provenance
    );

    let mut too_long = Vec::new();
    for _ in 0..=MAX_VALUE_PROJECTION_PATH_STEPS {
        too_long.push(json!({ "kind": "containerElement" }));
    }
    for invalid in [
        json!({
            "kind": "callerParameterProjection",
            "index": 2,
            "path": { "steps": [] }
        }),
        json!({
            "kind": "callerParameterProjection",
            "index": 2,
            "path": { "steps": [{ "kind": "field", "name": "" }] }
        }),
        json!({
            "kind": "callerParameterProjection",
            "index": 2,
            "path": { "steps": [{ "kind": "field", "name": " padded " }] }
        }),
        json!({
            "kind": "callerParameterProjection",
            "index": 2,
            "path": { "steps": too_long }
        }),
        json!({
            "kind": "callerParameterProjection",
            "index": 2,
            "path": {
                "steps": [{ "kind": "containerElement", "name": "forbidden" }]
            }
        }),
        json!({
            "kind": "callerParameterProjection",
            "index": 2,
            "path": {
                "steps": [{ "kind": "unknown" }]
            }
        }),
        json!({
            "kind": "callerParameterProjection",
            "index": 2,
            "path": {
                "steps": [{ "kind": "containerElement" }],
                "extra": true
            }
        }),
    ] {
        assert!(
            serde_json::from_value::<ValueProvenance>(invalid.clone()).is_err(),
            "invalid caller projection path must fail strict decoding: {invalid}"
        );
    }
}

#[test]
fn analyzed_provenance_wire_requires_direct_return_origins() {
    let summary = CallableProvenanceSummary::Analyzed {
        return_origins: vec![
            ValueProvenance::Fresh,
            ValueProvenance::CallerParameterProjection {
                index: 0,
                path: ValueProjectionPath::field("payload").unwrap(),
            },
        ],
        direct_return_origins: vec![ValueProvenance::Fresh],
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let wire = json!({
        "kind": "analyzed",
        "returnOrigins": [
            { "kind": "fresh" },
            {
                "kind": "callerParameterProjection",
                "index": 0,
                "path": {
                    "steps": [{ "kind": "field", "name": "payload" }]
                }
            }
        ],
        "directReturnOrigins": [{ "kind": "fresh" }],
        "throwOrigins": [],
        "escapeLanes": []
    });
    assert_eq!(serde_json::to_value(&summary).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<CallableProvenanceSummary>(wire.clone()).unwrap(),
        summary
    );

    let mut missing_direct = wire;
    missing_direct
        .as_object_mut()
        .expect("fixture is an object")
        .remove("directReturnOrigins");
    assert!(
        serde_json::from_value::<CallableProvenanceSummary>(missing_direct).is_err(),
        "directReturnOrigins is a required semantic fact, not a compatibility default"
    );
}

#[test]
fn available_projection_wire_is_contract_agnostic_and_descriptor_is_strict() {
    let operation_contract = operation_contract();
    let projection = BoundaryCallableProjection::Available {
        operation_contract: operation_contract.clone(),
        implementation_requirements: implementation_requirements(),
    };
    let wire = serde_json::to_value(&projection).unwrap();
    assert_eq!(wire["kind"], json!("available"));
    assert_eq!(
        wire["operationContract"],
        serde_json::to_value(&operation_contract).unwrap()
    );
    assert!(wire.get("implementationRequirements").is_some());
    assert_eq!(
        serde_json::from_value::<BoundaryCallableProjection>(wire.clone()).unwrap(),
        projection
    );

    let mut legacy_operation = serde_json::to_value(&operation_contract).unwrap();
    legacy_operation["errors"] = json!({ "kind": "none" });
    assert!(
        serde_json::from_value::<BoundaryOperationContract>(legacy_operation).is_err(),
        "operation-specific errors must not re-enter the open channel contract"
    );
    for (field, value) in [
        ("maySuspend", json!(false)),
        ("cancellation", json!({ "kind": "notCancellable" })),
    ] {
        let mut legacy_operation = serde_json::to_value(&operation_contract).unwrap();
        legacy_operation[field] = value;
        assert!(
            serde_json::from_value::<BoundaryOperationContract>(legacy_operation).is_err(),
            "provider-owned {field} must not re-enter the operation contract"
        );
    }

    for forbidden in ["descriptor", "operationId", "stableKey"] {
        let mut invalid = wire.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .insert(forbidden.to_string(), json!("forbidden"));
        assert!(
            serde_json::from_value::<BoundaryCallableProjection>(invalid).is_err(),
            "available projection must reject {forbidden}"
        );
    }

    for required in ["operationContract", "implementationRequirements"] {
        let mut missing = wire.clone();
        missing.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<BoundaryCallableProjection>(missing).is_err(),
            "available projection must require {required}"
        );
    }

    let descriptor = BoundaryOperationDescriptor {
        operation_id: ContractOperationId::new("operation:echo"),
        stable_key: "echo".to_string(),
        contract: operation_contract,
    };
    let descriptor_wire = serde_json::to_value(descriptor).unwrap();
    for required in ["operationId", "stableKey", "contract"] {
        let mut invalid = descriptor_wire.clone();
        invalid.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<BoundaryOperationDescriptor>(invalid).is_err(),
            "service operation descriptor must require {required}"
        );
    }
}

fn operation_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
        },
        stream: BoundaryStreamContract::Unary,
        callbacks: BoundaryCallbackContract::None,
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    }
}

fn implementation_requirements() -> BoundaryImplementationRequirements {
    BoundaryImplementationRequirements {
        config: Vec::new(),
        state: Vec::new(),
        native_capabilities: Vec::new(),
        complete_may_effects: CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            direct_return_origins: Vec::new(),
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
    }
}
