use std::collections::BTreeMap;

use crate::{
    BoundaryOperationContract, BoundaryStreamContract, BoundaryUnavailableReason,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary,
    CallableSemanticFacts, PackageArtifact, PackageBuildId, PackageCallableParameter,
    PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
    PackageSchemaTypeId, PackageTypeRef, TypeRefIr, ValueProvenance,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn mutation_wrong_parameter_owner_is_rejected() {
    let signature = unary_signature();
    let facts = safe_facts();
    let runtime = empty_runtime_requirements();
    let mut projection = canonical_boundary_callable_projection(&signature, &facts, &runtime);
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = &mut projection
    else {
        unreachable!()
    };
    operation_contract.parameters[0].value_plan =
        detached_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call);

    assert!(validate_boundary_callable_projection(
        &PackageCallableId::new("pkg-callable:example.pkg:run"),
        &signature,
        &facts,
        &runtime,
        &projection,
    )
    .is_err());
}

#[test]
fn standalone_unary_contract_rejects_every_noncanonical_plan_axis() {
    let canonical = available_contract(&unary_signature());
    assert!(validate_boundary_operation_contract(&canonical).is_ok());

    for mutation in 0..10 {
        let mut invalid = canonical.clone();
        match mutation {
            0 => {
                invalid.parameters[0].value_plan = BoundaryValuePlan::Unsupported {
                    reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                }
            }
            1 => set_plan_carrier(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueCarrier::CallbackCapability,
            ),
            2 => set_plan_encoding(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueEncoding::OpaqueCapability,
            ),
            3 => set_plan_owner(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueOwner::Provider,
            ),
            4 => set_plan_lifetime(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueLifetime::Request,
            ),
            5 => {
                invalid.return_value.value_plan = BoundaryValuePlan::Unsupported {
                    reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                }
            }
            6 => set_plan_carrier(
                &mut invalid.return_value.value_plan,
                BoundaryValueCarrier::CallbackCapability,
            ),
            7 => set_plan_encoding(
                &mut invalid.return_value.value_plan,
                BoundaryValueEncoding::OpaqueCapability,
            ),
            8 => set_plan_owner(
                &mut invalid.return_value.value_plan,
                BoundaryValueOwner::Caller,
            ),
            9 => set_plan_lifetime(
                &mut invalid.return_value.value_plan,
                BoundaryValueLifetime::Request,
            ),
            _ => unreachable!(),
        }
        assert!(
            validate_boundary_operation_contract(&invalid).is_err(),
            "unary mutation {mutation} must be rejected"
        );
    }
}

#[test]
fn standalone_server_stream_contract_rejects_sentinel_and_item_mutations() {
    let mut signature = unary_signature();
    signature.return_type = PackageTypeRef::Container {
        name: "Stream".to_string(),
        arguments: vec![PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        }],
    };
    let canonical = available_contract(&signature);
    assert!(validate_boundary_operation_contract(&canonical).is_ok());

    for mutation in 0..7 {
        let mut invalid = canonical.clone();
        match mutation {
            0 => invalid.return_value.ty = ContractTypeRef::builtin("string"),
            1 => {
                let BoundaryStreamContract::ServerStream {
                    item_value_plan, ..
                } = &mut invalid.stream
                else {
                    unreachable!()
                };
                *item_value_plan = BoundaryValuePlan::Unsupported {
                    reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                };
            }
            2 => mutate_stream_item_plan(&mut invalid, |plan| {
                set_plan_carrier(plan, BoundaryValueCarrier::CallbackCapability)
            }),
            3 => mutate_stream_item_plan(&mut invalid, |plan| {
                set_plan_encoding(plan, BoundaryValueEncoding::OpaqueCapability)
            }),
            4 => mutate_stream_item_plan(&mut invalid, |plan| {
                set_plan_owner(plan, BoundaryValueOwner::Caller)
            }),
            5 => mutate_stream_item_plan(&mut invalid, |plan| {
                set_plan_lifetime(plan, BoundaryValueLifetime::Call)
            }),
            6 => {
                invalid.stream = BoundaryStreamContract::Unsupported {
                    reason: crate::BoundaryFeatureUnavailableReason::LanguageUnsupported,
                }
            }
            _ => unreachable!(),
        }
        assert!(
            validate_boundary_operation_contract(&invalid).is_err(),
            "server-stream mutation {mutation} must be rejected"
        );
    }
}

#[test]
fn unary_signature_and_all_value_plan_axes_are_validated() {
    let signature = unary_signature();
    let facts = safe_facts();
    let runtime = empty_runtime_requirements();
    let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
    let callable_id = PackageCallableId::new("pkg-callable:example.pkg:run");
    assert!(validate_boundary_callable_projection(
        &callable_id,
        &signature,
        &facts,
        &runtime,
        &canonical
    )
    .is_ok());

    for mutation in 0..8 {
        let mut invalid = canonical.clone();
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = &mut invalid
        else {
            unreachable!()
        };
        match mutation {
            0 => operation_contract.parameters[0].name = "renamed".to_string(),
            1 => operation_contract.parameters[0].ty = ContractTypeRef::builtin("integer"),
            2 => {
                operation_contract.parameters[0].value_plan = BoundaryValuePlan::Unsupported {
                    reason: crate::BoundaryValuePlanUnavailableReason::LanguageUnsupported,
                }
            }
            3 => set_plan_carrier(
                &mut operation_contract.parameters[0].value_plan,
                BoundaryValueCarrier::CallbackCapability,
            ),
            4 => set_plan_encoding(
                &mut operation_contract.parameters[0].value_plan,
                BoundaryValueEncoding::OpaqueCapability,
            ),
            5 => set_plan_lifetime(
                &mut operation_contract.parameters[0].value_plan,
                BoundaryValueLifetime::Request,
            ),
            6 => {
                operation_contract.return_value.ty = ContractTypeRef::package_schema(
                    "wrong.owner",
                    "Result",
                    PackageSchemaTypeId::new("type:result"),
                )
            }
            7 => {
                operation_contract.stream = BoundaryStreamContract::Unsupported {
                    reason: crate::BoundaryFeatureUnavailableReason::LanguageUnsupported,
                }
            }
            _ => unreachable!(),
        }
        assert!(
            validate_boundary_callable_projection(
                &callable_id,
                &signature,
                &facts,
                &runtime,
                &invalid,
            )
            .is_err(),
            "mutation {mutation} must be rejected"
        );
    }
}

#[test]
fn server_stream_is_derived_only_from_exact_stream_signature() {
    let mut signature = unary_signature();
    signature.return_type = PackageTypeRef::Container {
        name: "Stream".to_string(),
        arguments: vec![PackageTypeRef::PackageSchema {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "Result".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("type:result"),
        }],
    };
    let facts = safe_facts();
    let runtime = empty_runtime_requirements();
    let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
    let callable_id = PackageCallableId::new("pkg-callable:example.pkg:watch");
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = &canonical
    else {
        panic!("exact Stream<T> must be available")
    };
    assert_eq!(
        operation_contract.return_value.ty,
        ContractTypeRef::builtin("void")
    );
    let BoundaryStreamContract::ServerStream {
        item_type,
        item_value_plan,
    } = &operation_contract.stream
    else {
        panic!("exact Stream<T> must derive server stream")
    };
    assert_eq!(
        item_type,
        &ContractTypeRef::package_schema(
            "example.pkg",
            "Result",
            PackageSchemaTypeId::new("type:result")
        )
    );
    assert_eq!(
        item_value_plan,
        &detached_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream)
    );

    for mutation in 0..7 {
        let mut invalid = canonical.clone();
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = &mut invalid
        else {
            unreachable!()
        };
        match mutation {
            0 => operation_contract.return_value.ty = ContractTypeRef::builtin("string"),
            1 => set_plan_owner(
                &mut operation_contract.return_value.value_plan,
                BoundaryValueOwner::Caller,
            ),
            2 => set_plan_lifetime(
                &mut operation_contract.return_value.value_plan,
                BoundaryValueLifetime::Stream,
            ),
            3 => operation_contract.stream = BoundaryStreamContract::Unary,
            4 => {
                let BoundaryStreamContract::ServerStream { item_type, .. } =
                    &mut operation_contract.stream
                else {
                    unreachable!()
                };
                *item_type = ContractTypeRef::builtin("string");
            }
            5 => {
                let BoundaryStreamContract::ServerStream {
                    item_value_plan, ..
                } = &mut operation_contract.stream
                else {
                    unreachable!()
                };
                set_plan_owner(item_value_plan, BoundaryValueOwner::Caller);
            }
            6 => {
                let BoundaryStreamContract::ServerStream {
                    item_value_plan, ..
                } = &mut operation_contract.stream
                else {
                    unreachable!()
                };
                set_plan_lifetime(item_value_plan, BoundaryValueLifetime::Call);
            }
            _ => unreachable!(),
        }
        assert!(
            validate_boundary_callable_projection(
                &callable_id,
                &signature,
                &facts,
                &runtime,
                &invalid,
            )
            .is_err(),
            "stream mutation {mutation} must be rejected"
        );
    }

    signature.return_type = PackageTypeRef::Container {
        name: "Stream".to_string(),
        arguments: Vec::new(),
    };
    assert_eq!(
        canonical_boundary_callable_projection(&signature, &facts, &runtime),
        BoundaryCallableProjection::Unavailable {
            reasons: vec![BoundaryUnavailableReason::UnsupportedStream]
        }
    );
}

#[test]
fn exact_non_generic_any_interface_is_the_only_callback_position() {
    let interface = PackageSchemaTypeRef {
        package_id: "example.pkg".to_string(),
        stable_schema_key: "api.Reader".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
    };
    let interface_type = ContractTypeRef::package_schema(
        interface.package_id.clone(),
        interface.stable_schema_key.clone(),
        interface.package_schema_type_id.clone(),
    );
    let exact = ContractTypeRef::AnyInterface {
        interface: Box::new(interface_type.clone()),
        arguments: Vec::new(),
    };
    assert_eq!(
        classify_boundary_callback_position(&exact),
        BoundaryCallbackPosition::Exact {
            interface_type: interface
        }
    );
    assert_eq!(
        classify_boundary_callback_position(&interface_type),
        BoundaryCallbackPosition::Detached,
        "a direct PackageSchema is data, not an implicit callback"
    );
    assert_eq!(
        classify_boundary_callback_position(&ContractTypeRef::Builtin {
            name: "Array".to_string(),
            arguments: vec![exact.clone()],
        }),
        BoundaryCallbackPosition::Unsupported
    );
    assert_eq!(
        classify_boundary_callback_position(&ContractTypeRef::AnyInterface {
            interface: Box::new(interface_type),
            arguments: vec![ContractTypeRef::builtin("string")],
        }),
        BoundaryCallbackPosition::Unsupported
    );
}

#[test]
fn unary_any_interface_rederives_request_scoped_callback_contract_exactly() {
    let signature = callback_signature(PackageTypeRef::Local {
        local_type: TypeRefIr::builtin("string"),
    });
    let projection = canonical_boundary_callable_projection(
        &signature,
        &safe_facts(),
        &empty_runtime_requirements(),
    );
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        panic!("exact non-generic any I parameter must be boundary available")
    };
    assert_eq!(
        operation_contract.parameters[0].value_plan,
        callback_plan(BoundaryValueLifetime::Request)
    );
    assert_eq!(
        operation_contract.callbacks,
        BoundaryCallbackContract::RequestScoped {
            interface_types: vec![callback_interface_ref()],
            lifetime: BoundaryCallbackLifetime::TopLevelRequest,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        }
    );
    assert!(validate_boundary_operation_contract(&operation_contract).is_ok());

    for mutation in 0..6 {
        let mut invalid = operation_contract.clone();
        match mutation {
            0 => set_plan_carrier(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueCarrier::DetachedValueGraph,
            ),
            1 => set_plan_encoding(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueEncoding::CanonicalValue,
            ),
            2 => set_plan_owner(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueOwner::Caller,
            ),
            3 => set_plan_lifetime(
                &mut invalid.parameters[0].value_plan,
                BoundaryValueLifetime::Call,
            ),
            4 => invalid.callbacks = BoundaryCallbackContract::None,
            5 => {
                let BoundaryCallbackContract::RequestScoped {
                    expiration_error, ..
                } = &mut invalid.callbacks
                else {
                    unreachable!()
                };
                *expiration_error = BoundaryCallbackExpirationError::CapabilityUnavailable;
            }
            _ => unreachable!(),
        }
        assert!(
            validate_boundary_operation_contract(&invalid).is_err(),
            "callback contract mutation {mutation} must fail exact validation"
        );
    }
}

#[test]
fn server_stream_extends_every_exact_callback_position_to_stream_lifetime() {
    let callback = callback_package_type();
    let signature = PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "callback".to_string(),
            ty: callback.clone(),
        }],
        return_type: PackageTypeRef::Container {
            name: "Stream".to_string(),
            arguments: vec![callback],
        },
        may_suspend: true,
    };
    let projection = canonical_boundary_callable_projection(
        &signature,
        &safe_facts(),
        &empty_runtime_requirements(),
    );
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        panic!("exact any I stream positions must be boundary available")
    };
    assert_eq!(
        operation_contract.parameters[0].value_plan,
        callback_plan(BoundaryValueLifetime::Stream)
    );
    let BoundaryStreamContract::ServerStream {
        item_value_plan, ..
    } = &operation_contract.stream
    else {
        panic!("fixture must project a server stream")
    };
    assert_eq!(
        item_value_plan,
        &callback_plan(BoundaryValueLifetime::Stream)
    );
    assert_eq!(
        operation_contract.callbacks,
        BoundaryCallbackContract::RequestScoped {
            interface_types: vec![callback_interface_ref()],
            lifetime: BoundaryCallbackLifetime::Stream,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        }
    );
    assert!(validate_boundary_operation_contract(&operation_contract).is_ok());
}

#[test]
fn nested_and_generic_any_interface_positions_remain_unavailable() {
    let callback = callback_package_type();
    for parameter_type in [
        PackageTypeRef::Container {
            name: "Array".to_string(),
            arguments: vec![callback.clone()],
        },
        PackageTypeRef::AnyInterface {
            interface: Box::new(PackageTypeRef::PackageSchema {
                package_id: "example.pkg".to_string(),
                stable_schema_key: "api.Reader".to_string(),
                package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
            }),
            arguments: vec![PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            }],
        },
    ] {
        let mut signature = unary_signature();
        signature.parameters[0].ty = parameter_type;
        assert_eq!(
            canonical_boundary_callable_projection(
                &signature,
                &safe_facts(),
                &empty_runtime_requirements(),
            ),
            BoundaryCallableProjection::Unavailable {
                reasons: vec![BoundaryUnavailableReason::CallbackAdapterUnavailable]
            }
        );
    }
}

#[test]
fn unavailable_reasons_are_nonempty_exact_and_canonical() {
    let mut signature = unary_signature();
    signature.parameters[0].ty = PackageTypeRef::Local {
        local_type: TypeRefIr::LocalType { type_index: 7 },
    };
    let mut facts = safe_facts();
    let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
        unreachable!()
    };
    effects.requires_same_heap_identity = true;
    effects.invokes_unknown_target = true;
    let runtime = empty_runtime_requirements();
    let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
    assert_eq!(
        canonical,
        BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::UnknownCallTarget,
                BoundaryUnavailableReason::RequiresSameHeapIdentity,
                BoundaryUnavailableReason::UnsupportedBoundaryType,
            ]
        }
    );
    let callable_id = PackageCallableId::new("pkg-callable:example.pkg:private");
    for invalid in [
        BoundaryCallableProjection::Unavailable {
            reasons: Vec::new(),
        },
        BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::UnsupportedBoundaryType,
                BoundaryUnavailableReason::RequiresSameHeapIdentity,
                BoundaryUnavailableReason::UnknownCallTarget,
            ],
        },
        BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::UnknownCallTarget,
                BoundaryUnavailableReason::RequiresSameHeapIdentity,
            ],
        },
        BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::UnknownCallTarget,
                BoundaryUnavailableReason::UnknownCallTarget,
                BoundaryUnavailableReason::RequiresSameHeapIdentity,
                BoundaryUnavailableReason::UnsupportedBoundaryType,
            ],
        },
    ] {
        assert!(validate_boundary_callable_projection(
            &callable_id,
            &signature,
            &facts,
            &runtime,
            &invalid,
        )
        .is_err());
    }
}

#[test]
fn unavailable_projection_accepts_only_canonical_type_closure_saturation() {
    let mut signature = unary_signature();
    signature.parameters[0].ty = PackageTypeRef::Local {
        local_type: TypeRefIr::LocalType { type_index: 7 },
    };
    let facts = safe_facts();
    let runtime = empty_runtime_requirements();
    let callable_id = PackageCallableId::new("pkg-callable:example.pkg:private");

    let saturated = BoundaryCallableProjection::Unavailable {
        reasons: vec![
            BoundaryUnavailableReason::UnsupportedBoundaryType,
            BoundaryUnavailableReason::UnsupportedStream,
        ],
    };
    assert!(validate_boundary_callable_projection(
        &callable_id,
        &signature,
        &facts,
        &runtime,
        &saturated,
    )
    .is_ok());

    for invalid in [
        BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::UnsupportedStream,
                BoundaryUnavailableReason::UnsupportedBoundaryType,
            ],
        },
        BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::UnknownEffect,
                BoundaryUnavailableReason::UnsupportedBoundaryType,
            ],
        },
    ] {
        assert!(validate_boundary_callable_projection(
            &callable_id,
            &signature,
            &facts,
            &runtime,
            &invalid,
        )
        .is_err());
    }
}

#[test]
fn implementation_requirements_must_match_complete_facts_and_runtime_requirements() {
    let signature = unary_signature();
    let facts = safe_facts();
    let runtime = empty_runtime_requirements();
    let canonical = canonical_boundary_callable_projection(&signature, &facts, &runtime);
    let callable_id = PackageCallableId::new("pkg-callable:example.pkg:run");
    for mutation in 0..2 {
        let mut invalid = canonical.clone();
        let BoundaryCallableProjection::Available {
            implementation_requirements,
            ..
        } = &mut invalid
        else {
            unreachable!()
        };
        match mutation {
            0 => implementation_requirements.complete_may_effects.may_pending = true,
            1 => {
                implementation_requirements.provenance = CallableProvenanceSummary::Unknown {
                    reason: crate::CallableProvenanceUnknownReason::AnalysisPending,
                }
            }
            _ => unreachable!(),
        }
        assert!(validate_boundary_callable_projection(
            &callable_id,
            &signature,
            &facts,
            &runtime,
            &invalid,
        )
        .is_err());
    }
}

#[test]
fn package_validator_requires_exact_public_callable_coverage() {
    let signature = unary_signature();
    let facts = safe_facts();
    let runtime = empty_runtime_requirements();
    let callable_id = PackageCallableId::new("pkg-callable:example.pkg:run");
    let projection = canonical_boundary_callable_projection(&signature, &facts, &runtime);
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.pkg".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("build"),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode: None,
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            public_symbols: BTreeMap::from([(
                "run".to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature,
                },
            )]),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.pkg".to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("index"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: runtime,
        callable_semantic_facts: BTreeMap::from([(callable_id.clone(), facts)]),
        boundary_projections: BTreeMap::from([(callable_id.clone(), projection)]),
        service_call_refs: Vec::new(),
    };
    assert!(validate_package_boundary_projections(&artifact).is_ok());
    artifact.boundary_projections.clear();
    assert!(validate_package_boundary_projections(&artifact).is_err());
}

fn unary_signature() -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "input".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
        }],
        return_type: PackageTypeRef::PackageSchema {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "Result".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("type:result"),
        },
        may_suspend: false,
    }
}

fn callback_signature(return_type: PackageTypeRef) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: "callback".to_string(),
            ty: callback_package_type(),
        }],
        return_type,
        may_suspend: true,
    }
}

fn callback_package_type() -> PackageTypeRef {
    PackageTypeRef::AnyInterface {
        interface: Box::new(PackageTypeRef::PackageSchema {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "api.Reader".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
        }),
        arguments: Vec::new(),
    }
}

fn callback_interface_ref() -> PackageSchemaTypeRef {
    PackageSchemaTypeRef {
        package_id: "example.pkg".to_string(),
        stable_schema_key: "api.Reader".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("type:reader"),
    }
}

fn safe_facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: false,
                pending_effect_categories: Vec::new(),
                inout_path_effects: Vec::new(),
},
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::Fresh],
            direct_return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn empty_runtime_requirements() -> PackageRuntimeRequirements {
    PackageRuntimeRequirements { config: Vec::new() }
}

fn available_contract(signature: &PackageCallableSignature) -> BoundaryOperationContract {
    let projection = canonical_boundary_callable_projection(
        signature,
        &safe_facts(),
        &empty_runtime_requirements(),
    );
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = projection
    else {
        panic!("fixture signature must be available")
    };
    operation_contract
}

fn mutate_stream_item_plan(
    contract: &mut BoundaryOperationContract,
    mutation: impl FnOnce(&mut BoundaryValuePlan),
) {
    let BoundaryStreamContract::ServerStream {
        item_value_plan, ..
    } = &mut contract.stream
    else {
        unreachable!()
    };
    mutation(item_value_plan);
}

fn detached_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn set_plan_carrier(plan: &mut BoundaryValuePlan, carrier: BoundaryValueCarrier) {
    let BoundaryValuePlan::Linkable {
        carrier: actual, ..
    } = plan
    else {
        unreachable!()
    };
    *actual = carrier;
}

fn set_plan_encoding(plan: &mut BoundaryValuePlan, encoding: BoundaryValueEncoding) {
    let BoundaryValuePlan::Linkable {
        encoding: actual, ..
    } = plan
    else {
        unreachable!()
    };
    *actual = encoding;
}

fn set_plan_owner(plan: &mut BoundaryValuePlan, owner: BoundaryValueOwner) {
    let BoundaryValuePlan::Linkable { owner: actual, .. } = plan else {
        unreachable!()
    };
    *actual = owner;
}

fn set_plan_lifetime(plan: &mut BoundaryValuePlan, lifetime: BoundaryValueLifetime) {
    let BoundaryValuePlan::Linkable {
        lifetime: actual, ..
    } = plan
    else {
        unreachable!()
    };
    *actual = lifetime;
}
