use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryErrorContract, BoundaryUnavailableReason,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    CallableTargetFact, ContractTypeId, ContractTypeRef, LiteralIr, PackageCallableParameter,
    PackageCallableSignature, PackageRuntimeRequirements, PackageTypeRef, TypeRefIr,
    ValueEscapeLane, ValueProvenance,
};

use super::fixtures::{runtime_requirements, safe_facts, signature};
use crate::package_artifact::project_boundary_callable;

#[test]
fn safe_detached_callable_is_available_with_contract_agnostic_body_and_requirements() {
    let parameter_type = ContractTypeId::new("contract-type:request");
    let error_type = ContractTypeId::new("contract-type:error");
    let mut signature = signature(TypeRefIr::native("string"));
    signature.parameters[0].ty = PackageTypeRef::Contract {
        contract_type_id: parameter_type.clone(),
    };
    signature.throw_types = vec![PackageTypeRef::Contract {
        contract_type_id: error_type.clone(),
    }];
    let runtime = runtime_requirements("async");
    let projection = project_boundary_callable(
        "api",
        &signature,
        &safe_facts(),
        &runtime,
        &[],
        &BTreeMap::new(),
    )
    .unwrap();

    let BoundaryCallableProjection::Available {
        operation_contract,
        implementation_requirements,
    } = projection
    else {
        panic!("safe detached callable must be Available");
    };
    assert_eq!(operation_contract.parameters.len(), 1);
    assert_eq!(
        operation_contract.parameters[0].ty,
        ContractTypeRef::contract(parameter_type)
    );
    assert!(matches!(
        &operation_contract.errors,
        BoundaryErrorContract::Typed {
            payload_type: ContractTypeRef::Contract { contract_type_id },
            ..
        } if contract_type_id == &error_type
    ));
    assert!(operation_contract.effect_guarantee.detached_parameters);
    let wire = serde_json::to_value(BoundaryCallableProjection::Available {
        operation_contract: operation_contract.clone(),
        implementation_requirements: implementation_requirements.clone(),
    })
    .unwrap();
    for forbidden in ["descriptor", "operationId", "stableKey"] {
        assert!(wire.get(forbidden).is_none(), "forbidden {forbidden}");
    }
    assert_eq!(implementation_requirements.config[0].path, "app.token");
    assert_eq!(implementation_requirements.state[0].key, "database");
    assert_eq!(
        implementation_requirements.runtime_capabilities,
        vec!["async"]
    );
}

#[test]
fn every_frozen_unavailable_reason_is_projected_fail_closed() {
    let unknown = CallableSemanticFacts {
        effects: CallableEffectSummary::analysis_pending(),
        provenance: CallableProvenanceSummary::Unknown {
            reason: skiff_artifact_model::CallableProvenanceUnknownReason::AnalysisPending,
        },
        resolved_call_targets: BTreeMap::new(),
    };
    assert_reasons(
        signature(TypeRefIr::native("string")),
        unknown,
        &[
            BoundaryUnavailableReason::AnalysisPending,
            BoundaryUnavailableReason::UnknownEffect,
        ],
    );

    let mut unsafe_facts = safe_facts();
    unsafe_facts.effects = CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            writes_caller_reachable: true,
            returns_caller_alias: true,
            throws_caller_alias: true,
            escapes_caller_value: true,
            requires_same_heap_identity: true,
            invokes_unknown_target: true,
            may_suspend: false,
        },
    };
    unsafe_facts.provenance = CallableProvenanceSummary::Analyzed {
        return_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
        throw_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
        escape_lanes: vec![ValueEscapeLane::Capture],
    };
    assert_reasons(
        signature(TypeRefIr::native("string")),
        unsafe_facts,
        &[
            BoundaryUnavailableReason::UnknownCallTarget,
            BoundaryUnavailableReason::WritesCallerReachable,
            BoundaryUnavailableReason::ReturnsCallerAlias,
            BoundaryUnavailableReason::ThrowsCallerAlias,
            BoundaryUnavailableReason::EscapesCallerValue {
                lane: ValueEscapeLane::Capture,
            },
            BoundaryUnavailableReason::RequiresSameHeapIdentity,
        ],
    );

    let mut unknown_target = safe_facts();
    unknown_target
        .resolved_call_targets
        .insert(0, CallableTargetFact::Unknown);
    assert_reasons(
        signature(TypeRefIr::native("string")),
        unknown_target,
        &[BoundaryUnavailableReason::UnknownCallTarget],
    );
    assert_reasons(
        signature(TypeRefIr::Function {
            params: Vec::new(),
            return_type: Box::new(TypeRefIr::native("void")),
        }),
        safe_facts(),
        &[BoundaryUnavailableReason::CallbackAdapterUnavailable],
    );
    assert_reasons(
        signature(TypeRefIr::native("Socket")),
        safe_facts(),
        &[BoundaryUnavailableReason::NativeAdapterUnavailable],
    );
    assert_reasons(
        signature(TypeRefIr::LocalType { type_index: 0 }),
        safe_facts(),
        &[BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    assert_reasons(
        signature(TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::native("string")],
        }),
        safe_facts(),
        &[BoundaryUnavailableReason::UnsupportedStream],
    );
    for literal in [
        LiteralIr::Bool { value: true },
        LiteralIr::Number {
            value: serde_json::Number::from(7),
        },
        LiteralIr::String {
            value: "exact".to_string(),
        },
    ] {
        assert_reasons(
            signature(TypeRefIr::Literal { value: literal }),
            safe_facts(),
            &[BoundaryUnavailableReason::UnsupportedBoundaryType],
        );
    }
}

#[test]
fn null_literal_keeps_its_exact_canonical_boundary_semantics() {
    let projection = project_boundary_callable(
        "api",
        &signature(TypeRefIr::Literal {
            value: LiteralIr::Null,
        }),
        &safe_facts(),
        &PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        panic!("null literal is exactly representable");
    };
    assert_eq!(
        operation_contract.parameters[0].ty,
        ContractTypeRef::builtin("null")
    );
}

#[test]
fn exported_package_nominal_projects_as_exact_public_type_reference() {
    let public_types = BTreeMap::from([(
        ("model".to_string(), "Envelope".to_string()),
        "type:Envelope".to_string(),
    )]);
    let projection = project_boundary_callable(
        "api",
        &signature(TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::Native {
                name: "Array".to_string(),
                args: vec![TypeRefIr::ServiceSymbol {
                    symbol: skiff_artifact_model::ServiceSymbolRef {
                        module_path: "model".to_string(),
                        symbol: "Envelope".to_string(),
                    },
                }],
            }),
        }),
        &safe_facts(),
        &empty_runtime_requirements(),
        &[],
        &public_types,
    )
    .unwrap();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        panic!("api.yml-exported package nominal must be boundary available");
    };
    assert_eq!(
        operation_contract.parameters[0].ty,
        ContractTypeRef::Nullable {
            inner: Box::new(ContractTypeRef::Builtin {
                name: "Array".to_string(),
                arguments: vec![ContractTypeRef::PackagePublic {
                    local_type_id: "type:Envelope".to_string()
                }],
            }),
        }
    );
}

#[test]
fn private_package_nominal_remains_boundary_unavailable() {
    assert_reasons(
        signature(TypeRefIr::ServiceSymbol {
            symbol: skiff_artifact_model::ServiceSymbolRef {
                module_path: "model".to_string(),
                symbol: "PrivateEnvelope".to_string(),
            },
        }),
        safe_facts(),
        &[BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
}

#[test]
fn websocket_ingress_generic_types_project_to_the_exact_contract_abi() {
    let context = PackageTypeRef::Contract {
        contract_type_id: ContractTypeId::new("contract-type:websocket-context"),
    };
    let signature = PackageCallableSignature {
        parameters: vec![PackageCallableParameter {
            name: "event".to_string(),
            ty: PackageTypeRef::Container {
                name: "std.websocket.WebSocketIngressEvent".to_string(),
                arguments: vec![context.clone()],
            },
        }],
        return_type: PackageTypeRef::Nullable {
            inner: Box::new(PackageTypeRef::Container {
                name: "std.websocket.WebSocketConnectResult".to_string(),
                arguments: vec![context],
            }),
        },
        throw_types: Vec::new(),
        may_suspend: false,
    };
    let projection = project_boundary_callable(
        "api",
        &signature,
        &safe_facts(),
        &empty_runtime_requirements(),
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        panic!("canonical WebSocket ingress must be boundary available")
    };
    assert_eq!(operation_contract.parameters[0].name, "event");
    assert!(matches!(
        &operation_contract.parameters[0].ty,
        ContractTypeRef::Builtin { name, arguments }
            if name == "std.websocket.WebSocketIngressEvent" && arguments.len() == 1
    ));
    assert!(matches!(
        &operation_contract.return_value.ty,
        ContractTypeRef::Nullable { inner }
            if matches!(inner.as_ref(), ContractTypeRef::Builtin { name, arguments }
                if name == "std.websocket.WebSocketConnectResult" && arguments.len() == 1)
    ));
}

#[test]
fn websocket_ingress_generic_types_reject_wrong_arity() {
    let projection = project_boundary_callable(
        "api",
        &signature(TypeRefIr::Native {
            name: "std.websocket.WebSocketIngressEvent".to_string(),
            args: Vec::new(),
        }),
        &safe_facts(),
        &empty_runtime_requirements(),
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(matches!(
        projection,
        BoundaryCallableProjection::Unavailable { reasons }
            if reasons.contains(&BoundaryUnavailableReason::UnsupportedBoundaryType)
    ));
}

fn assert_reasons(
    signature: PackageCallableSignature,
    facts: CallableSemanticFacts,
    expected: &[BoundaryUnavailableReason],
) {
    let projection = project_boundary_callable(
        "api",
        &signature,
        &facts,
        &PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        &[],
        &BTreeMap::new(),
    )
    .unwrap();
    let BoundaryCallableProjection::Unavailable { reasons } = projection else {
        panic!("unsafe callable must be Unavailable");
    };
    for reason in expected {
        assert!(
            reasons.contains(reason),
            "missing {reason:?} in {reasons:?}"
        );
    }
}

fn empty_runtime_requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements {
        config: Vec::new(),
        resources: Vec::new(),
        runtime_capabilities: Vec::new(),
    }
}
