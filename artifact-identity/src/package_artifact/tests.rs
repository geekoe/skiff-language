use skiff_artifact_model::{
    ActorAbiIdentity, ActorAbiInput, ActorCreateSignatureIr, ActorFieldEncodingIr, ActorFieldIr,
    ActorMethodIdentity, ActorPublicMethodIr, BoundaryCallbackContract, BoundaryEffectGuarantee,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryParameter,
    BoundaryReturn, BoundaryStreamContract, BoundaryUnavailableReason, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts, PendingEffectCategory,
    ConstExport, ContractOperationId, ContractRequirement, ContractTypeRef, ExecutableExport,
    ExecutableSignatureIr, FileIrRef, FunctionTypeParamIr, InterfaceMethodSignature,
    NominalTypeRefBaseIr, OperationCallableKind, OperationTargetRef, PackageActorAbi,
    PackageCallableLinkFact, PackageCallableParameter, PackageCallableSignature,
    PackageConfigAccess, PackageConfigRequirement, PackageImplementationLinks, PackageRefIr,
    PackageRequirement, PackageSymbolRef, PackageTypeRef, ParamIr, ServiceProtocolIdentity,
    ServiceRequirement, ServiceSymbolRef, TypeDescriptorIr, TypeExport, TypeRefIr, ValueProvenance,
    ACTOR_RUNTIME_ABI_VERSION_V1, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn current_package_artifact_generation_assigns_and_rejects_stale_domains() {
    let artifact = fixture();
    assert!(artifact
        .package_build_id
        .as_str()
        .starts_with(crate::PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX));
    assert!(artifact
        .package_local_abi
        .local_abi_identity
        .as_str()
        .starts_with(crate::PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX));
    assert_eq!(
        serde_json::to_value(package_artifact_build_identity_projection(&artifact).unwrap())
            .unwrap()["schema"],
        crate::PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER
    );
    assert_eq!(
        serde_json::to_value(package_artifact_local_abi_identity_projection(&artifact).unwrap())
            .unwrap()["schema"],
        crate::PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER
    );
    validate_package_artifact_identities(&artifact).unwrap();

    let mut stale_schema = artifact.clone();
    stale_schema.schema_version = "skiff-package-artifact-v8".to_string();
    assert!(matches!(
        validate_package_artifact_identities(&stale_schema),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));

    let mut stale_local = artifact.clone();
    stale_local.package_local_abi.local_abi_identity = PackageLocalAbiIdentity::new(
        stale_local
            .package_local_abi
            .local_abi_identity
            .as_str()
            .replacen(
                crate::PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
                "skiff-package-local-abi-v6:sha256",
                1,
            ),
    );
    assert!(matches!(
        validate_package_artifact_identities(&stale_local),
        Err(ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch { .. })
    ));

    let mut stale_build = artifact;
    stale_build.package_build_id =
        PackageBuildId::new(stale_build.package_build_id.as_str().replacen(
            crate::PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
            "skiff-package-build-v9:sha256",
            1,
        ));
    assert!(matches!(
        validate_package_artifact_identities(&stale_build),
        Err(ArtifactIdentityError::PackageArtifactBuildIdentityMismatch { .. })
    ));
}

#[test]
fn actor_metadata_tampering_changes_local_abi_and_build_identity() {
    let artifact = actor_fixture();
    let base_local = package_artifact_local_abi_identity(&artifact).unwrap();
    let base_build = package_artifact_build_identity(&artifact).unwrap();

    let mut tampered_key = artifact.clone();
    let PackageLocalAbiSymbol::Type { actor, .. } = tampered_key
        .package_local_abi
        .public_symbols
        .get_mut("ThreadActor")
        .expect("public actor symbol")
    else {
        panic!("public actor must remain a Type symbol");
    };
    let actor = actor
        .as_mut()
        .expect("public actor must carry actor metadata");
    actor.abi.key_field = "label".to_string();
    actor.abi.actor_id_type = TypeRefIr::builtin("string");
    assert_ne!(
        package_artifact_local_abi_identity(&tampered_key).unwrap(),
        base_local
    );
    assert_ne!(
        package_artifact_build_identity(&tampered_key).unwrap(),
        base_build
    );

    let mut tampered_create = artifact.clone();
    let PackageLocalAbiSymbol::Type { actor, .. } = tampered_create
        .package_local_abi
        .public_symbols
        .get_mut("ThreadActor")
        .expect("public actor symbol")
    else {
        panic!("public actor must remain a Type symbol");
    };
    let actor = actor
        .as_mut()
        .expect("public actor must carry actor metadata");
    actor.abi.create.as_mut().unwrap().parameters[0].name = "title".to_string();
    assert_ne!(
        package_artifact_local_abi_identity(&tampered_create).unwrap(),
        base_local
    );
    assert_ne!(
        package_artifact_build_identity(&tampered_create).unwrap(),
        base_build
    );

    let mut tampered_method = artifact.clone();
    let PackageLocalAbiSymbol::Type { actor, .. } = tampered_method
        .package_local_abi
        .public_symbols
        .get_mut("ThreadActor")
        .expect("public actor symbol")
    else {
        panic!("public actor must remain a Type symbol");
    };
    let actor = actor
        .as_mut()
        .expect("public actor must carry actor metadata");
    actor.abi.public_methods[0].name = "fetch".to_string();
    actor.abi.public_methods[0].method_identity =
        ActorMethodIdentity::new("skiff-actor-method-v1:sha256:fetch");
    assert_ne!(
        package_artifact_local_abi_identity(&tampered_method).unwrap(),
        base_local
    );
    assert_ne!(
        package_artifact_build_identity(&tampered_method).unwrap(),
        base_build
    );
}

#[test]
fn package_identity_requires_one_canonical_config_access_per_path() {
    let mut duplicate = fixture();
    duplicate.runtime_requirements.config = vec![
        PackageConfigRequirement {
            path: "provider.apiKey".to_string(),
            access: PackageConfigAccess::Presence,
        },
        PackageConfigRequirement {
            path: "provider.apiKey".to_string(),
            access: PackageConfigAccess::Required {
                value_type: "string".to_string(),
            },
        },
    ];
    assert!(matches!(
        validate_package_artifact_identities(&duplicate),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));

    let mut canonical = fixture();
    canonical.runtime_requirements.config = vec![PackageConfigRequirement {
        path: "provider.apiKey".to_string(),
        access: PackageConfigAccess::Required {
            value_type: "string".to_string(),
        },
    }];
    assign_package_artifact_identities(&mut canonical).unwrap();
    validate_package_artifact_identities(&canonical).unwrap();
}

#[test]
fn validated_package_artifact_owns_one_exact_unforgeable_content_snapshot() {
    let artifact = fixture();
    let admitted = ValidatedPackageArtifact::admit_clone(&artifact).unwrap();
    assert_eq!(
        admitted.reference(),
        &package_artifact_ref(&artifact).unwrap()
    );
    assert!(admitted.exactly_matches(&artifact));
    assert!(
        admitted.has_same_exact_content(&ValidatedPackageArtifact::admit_clone(&artifact).unwrap())
    );

    let mut changed = artifact;
    changed.package_version = "2.0.0".to_string();
    assert!(!admitted.exactly_matches(&changed));
}

#[test]
fn package_artifact_human_version_label_is_not_an_identity_input() {
    let base = fixture();
    let mut relabeled = base.clone();
    relabeled.package_version = "99.0.0".to_string();

    assert_eq!(
        package_artifact_local_abi_identity(&base).unwrap(),
        package_artifact_local_abi_identity(&relabeled).unwrap()
    );
    assert_eq!(
        package_artifact_build_identity(&base).unwrap(),
        package_artifact_build_identity(&relabeled).unwrap()
    );
}

#[test]
fn implementation_throw_facts_without_matching_boundary_projection_are_rejected() {
    let base = callable_fixture();
    let callable_id = base.callable_semantic_facts.keys().next().unwrap().clone();

    let mut changed = base;
    let facts = changed
        .callable_semantic_facts
        .get_mut(&callable_id)
        .unwrap();
    let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
        panic!("fixture effects must be analyzed")
    };
    effects.requires_same_heap_identity = true;
    let CallableProvenanceSummary::Analyzed { throw_origins, .. } = &mut facts.provenance else {
        panic!("fixture provenance must be analyzed")
    };
    *throw_origins = vec![ValueProvenance::CallerParameter { index: 0 }];

    assert_invalid_package_artifact(&changed);
    assert!(matches!(
        package_artifact_local_abi_identity(&changed),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));
}

#[test]
fn package_artifact_build_v10_preimage_excludes_service_selection() {
    let artifact = two_callable_fixture();
    let build =
        serde_json::to_value(package_artifact_build_identity_projection(&artifact).unwrap())
            .unwrap();
    let local =
        serde_json::to_value(package_artifact_local_abi_identity_projection(&artifact).unwrap())
            .unwrap();
    let wire = serde_json::to_value(&artifact).unwrap();

    assert_eq!(
        build["schema"],
        crate::PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER
    );
    assert_eq!(build["schema"], "skiff-package-artifact-build-identity-v9");
    assert!(build.get("serviceCallRoots").is_none());
    assert!(wire.get("serviceCallRoots").is_none());
    assert_eq!(build["serviceCallRefs"], serde_json::json!([]));
    assert_eq!(wire["serviceCallRefs"], serde_json::json!([]));
    assert!(artifact
        .package_build_id
        .as_str()
        .starts_with("skiff-package-build-v10:sha256:"));

    assert_eq!(
        local["schema"],
        "skiff-package-artifact-local-abi-identity-v6"
    );
    assert!(local.get("serviceCallRoots").is_none());
    assert!(artifact
        .package_local_abi
        .local_abi_identity
        .as_str()
        .starts_with("skiff-package-local-abi-v7:sha256:"));
}

#[test]
fn package_artifact_service_call_refs_remain_identity_bearing_and_fail_closed() {
    let mut artifact = fixture();
    let protocol = ServiceProtocolIdentity::new("protocol");
    let operation = ContractOperationId::new("operation");
    let contract_requirement = ContractRequirement {
        alias: "payments".to_string(),
        service_id: "example.payments".to_string(),
        contract_version: "1.0.0".to_string(),
        expected_protocol_identity: protocol.clone(),
    };
    artifact.contract_requirements = vec![contract_requirement.clone()];
    artifact.service_requirements = vec![ServiceRequirement {
        contract_requirement,
        service_binding_slot: 2,
        used_operations: std::collections::BTreeSet::from([operation.clone()]),
    }];
    artifact.service_call_refs = vec![ServiceCallRef {
        service_requirement_slot: 2,
        contract_operation_id: operation,
        expected_protocol_identity: protocol,
    }];
    assign_package_artifact_identities(&mut artifact).unwrap();

    let projection =
        serde_json::to_value(package_artifact_build_identity_projection(&artifact).unwrap())
            .unwrap();
    assert_eq!(projection["serviceCallRefs"].as_array().unwrap().len(), 1);

    let mut missing_ref = artifact.clone();
    missing_ref.service_call_refs.clear();
    assert_invalid_package_artifact(&missing_ref);

    let mut forged_slot = artifact;
    forged_slot.service_call_refs[0].service_requirement_slot = 3;
    assert_invalid_package_artifact(&forged_slot);
}

#[test]
fn callable_semantic_facts_reject_orphan_callable_ids() {
    let mut artifact = callable_fixture();
    let facts = artifact
        .callable_semantic_facts
        .values()
        .next()
        .unwrap()
        .clone();
    artifact.callable_semantic_facts.insert(
        PackageCallableId::new("pkg-callable:example.identity:orphan"),
        facts,
    );
    assert_invalid_package_artifact(&artifact);
}

#[test]
fn package_identity_entrypoints_reject_boundary_plan_and_type_mutations() {
    let canonical = callable_fixture();
    let mut mutations = Vec::new();

    for mutation in 0..8 {
        let mut invalid = canonical.clone();
        let operation = available_operation_mut(&mut invalid);
        let plan = if mutation < 4 {
            &mut operation.parameters[0].value_plan
        } else {
            &mut operation.return_value.value_plan
        };
        match mutation % 4 {
            0 => set_plan_owner(
                plan,
                if mutation < 4 {
                    BoundaryValueOwner::Provider
                } else {
                    BoundaryValueOwner::Caller
                },
            ),
            1 => set_plan_lifetime(plan, BoundaryValueLifetime::Request),
            2 => set_plan_carrier(plan, BoundaryValueCarrier::CallbackCapability),
            3 => set_plan_encoding(plan, BoundaryValueEncoding::OpaqueCapability),
            _ => unreachable!(),
        }
        mutations.push(invalid);
    }

    let mut private_local_as_public_schema = canonical.clone();
    available_operation_mut(&mut private_local_as_public_schema).parameters[0].ty =
        ContractTypeRef::package_schema(
            "example.schema",
            "Input",
            PackageSchemaTypeId::new("type:input"),
        );
    mutations.push(private_local_as_public_schema);

    let mut wrong_requirements = canonical.clone();
    let BoundaryCallableProjection::Available {
        implementation_requirements,
        ..
    } = wrong_requirements
        .boundary_projections
        .values_mut()
        .next()
        .unwrap()
    else {
        unreachable!()
    };
    implementation_requirements.complete_may_effects.may_pending = true;
    implementation_requirements
        .complete_may_effects
        .pending_effect_categories
        .push(PendingEffectCategory::Unknown);
    mutations.push(wrong_requirements);

    let mut wrong_facts = canonical;
    let CallableEffectSummary::Analyzed { effects } = &mut wrong_facts
        .callable_semantic_facts
        .values_mut()
        .next()
        .unwrap()
        .effects
    else {
        unreachable!()
    };
    effects.requires_same_heap_identity = true;
    mutations.push(wrong_facts);

    for (index, invalid) in mutations.iter().enumerate() {
        assert_all_package_identity_entries_reject(invalid, &format!("mutation {index}"));
    }
}

#[test]
fn package_identity_rejects_public_schema_and_unavailable_reason_mutations() {
    let mut public_schema = callable_fixture();
    public_schema.package_requirements.push(PackageRequirement {
        alias: "schema".to_string(),
        package_id: "example.schema".to_string(),
        exact_version: "1.0.0".to_string(),
        expected_local_abi: PackageLocalAbiIdentity::new("schema-abi"),
        expected_package_build: None,
    });
    let schema_type = PackageSchemaTypeId::new("type:input");
    callable_signature_mut(&mut public_schema).parameters[0].ty = PackageTypeRef::PackageSchema {
        package_id: "example.schema".to_string(),
        stable_schema_key: "Input".to_string(),
        package_schema_type_id: schema_type.clone(),
    };
    available_operation_mut(&mut public_schema).parameters[0].ty =
        ContractTypeRef::package_schema("example.schema", "Input", schema_type);
    assign_package_artifact_identities(&mut public_schema).unwrap();

    let mut public_schema_as_private_local = public_schema;
    available_operation_mut(&mut public_schema_as_private_local).parameters[0].ty =
        ContractTypeRef::builtin("string");
    assert_all_package_identity_entries_reject(
        &public_schema_as_private_local,
        "public PackageSchema projected as a private local type",
    );

    let mut unavailable = callable_fixture();
    let CallableEffectSummary::Analyzed { effects } = &mut unavailable
        .callable_semantic_facts
        .values_mut()
        .next()
        .unwrap()
        .effects
    else {
        unreachable!()
    };
    effects.requires_same_heap_identity = true;
    effects.invokes_unknown_target = true;
    let canonical_reasons = vec![
        BoundaryUnavailableReason::UnknownCallTarget,
        BoundaryUnavailableReason::RequiresSameHeapIdentity,
    ];
    set_boundary_unavailable(&mut unavailable, canonical_reasons.clone());
    assign_package_artifact_identities(&mut unavailable).unwrap();

    for (label, reasons) in [
        ("empty", Vec::new()),
        (
            "wrong",
            vec![BoundaryUnavailableReason::RequiresSameHeapIdentity],
        ),
        (
            "out-of-order",
            canonical_reasons.into_iter().rev().collect(),
        ),
    ] {
        let mut invalid = unavailable.clone();
        set_boundary_unavailable(&mut invalid, reasons);
        assert_all_package_identity_entries_reject(
            &invalid,
            &format!("{label} unavailable reasons"),
        );
    }
}

#[test]
fn package_identity_rejects_server_stream_sentinel_setup_and_item_plan_mutations() {
    let mut canonical = callable_fixture();
    callable_signature_mut(&mut canonical).return_type = PackageTypeRef::Container {
        name: "Stream".to_string(),
        arguments: vec![PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        }],
    };
    let operation = available_operation_mut(&mut canonical);
    operation.return_value.ty = ContractTypeRef::builtin("void");
    operation.stream = BoundaryStreamContract::ServerStream {
        item_type: ContractTypeRef::builtin("string"),
        item_value_plan: detached_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream),
    };
    assign_package_artifact_identities(&mut canonical).unwrap();

    for mutation in 0..6 {
        let mut invalid = canonical.clone();
        let operation = available_operation_mut(&mut invalid);
        match mutation {
            0 => operation.return_value.ty = ContractTypeRef::builtin("string"),
            1 => operation.stream = BoundaryStreamContract::Unary,
            2..=5 => {
                let BoundaryStreamContract::ServerStream {
                    item_value_plan, ..
                } = &mut operation.stream
                else {
                    unreachable!()
                };
                match mutation {
                    2 => set_plan_owner(item_value_plan, BoundaryValueOwner::Caller),
                    3 => set_plan_lifetime(item_value_plan, BoundaryValueLifetime::Call),
                    4 => {
                        set_plan_carrier(item_value_plan, BoundaryValueCarrier::CallbackCapability)
                    }
                    5 => {
                        set_plan_encoding(item_value_plan, BoundaryValueEncoding::OpaqueCapability)
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        assert_all_package_identity_entries_reject(
            &invalid,
            &format!("stream mutation {mutation}"),
        );
    }
}

#[test]
fn rehashed_boundary_semantics_are_still_rejected() {
    let mut forged = callable_fixture();
    set_plan_owner(
        &mut available_operation_mut(&mut forged).parameters[0].value_plan,
        BoundaryValueOwner::Provider,
    );
    let local = projection::local_abi_identity_from_validated(&forged).unwrap();
    forged.package_local_abi.local_abi_identity = local.clone();
    let build_projection = projection::build_projection_from_validated(&forged, local).unwrap();
    forged.package_build_id =
        projection::build_identity_from_projection(&build_projection).unwrap();

    assert_all_package_identity_entries_reject(&forged, "rehashed noncanonical boundary semantics");
}

#[test]
fn callable_parameter_return_and_suspend_mutations_change_local_abi_without_throw_set() {
    let base = callable_fixture();
    let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
    let baseline_build = package_artifact_build_identity(&base).unwrap();

    let mut parameter = base.clone();
    callable_signature_mut(&mut parameter).parameters[0].ty = PackageTypeRef::Local {
        local_type: TypeRefIr::builtin("integer"),
    };
    available_operation_mut(&mut parameter).parameters[0].ty = ContractTypeRef::builtin("integer");
    let mut returned = base.clone();
    callable_signature_mut(&mut returned).return_type = PackageTypeRef::Local {
        local_type: TypeRefIr::builtin("bool"),
    };
    available_operation_mut(&mut returned).return_value.ty = ContractTypeRef::builtin("bool");
    let mut suspended = base.clone();
    callable_signature_mut(&mut suspended).may_suspend = true;

    for changed in [&parameter, &returned, &suspended] {
        assert_ne!(
            package_artifact_local_abi_identity(changed).unwrap(),
            baseline_local
        );
        assert_ne!(
            package_artifact_build_identity(changed).unwrap(),
            baseline_build
        );
    }

    let PackageLocalAbiSymbol::Callable { signature, .. } =
        &base.package_local_abi.public_symbols["run"]
    else {
        panic!("fixture run must be callable")
    };
    let wire = serde_json::to_value(signature).unwrap();
    assert!(wire.get("throwTypes").is_none());
    assert_eq!(
        wire.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["typeParams", "parameters", "returnType", "maySuspend"]
    );

    let mut legacy = wire;
    legacy["throwTypes"] = serde_json::json!([]);
    assert!(serde_json::from_value::<PackageCallableSignature>(legacy).is_err());
}

#[test]
fn callable_type_parameter_scope_is_explicit_closed_and_identity_bearing() {
    let mut generic = callable_fixture();
    let signature = callable_signature_mut(&mut generic);
    signature.type_params = vec!["T".to_string(), "Id".to_string()];
    signature.parameters[0].ty = PackageTypeRef::Local {
        local_type: TypeRefIr::TypeParam {
            name: "Id".to_string(),
        },
    };
    signature.return_type = PackageTypeRef::Nullable {
        inner: Box::new(PackageTypeRef::Local {
            local_type: TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        }),
    };
    set_boundary_unavailable(
        &mut generic,
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    assign_package_artifact_identities(&mut generic).unwrap();
    let generic_local = generic.package_local_abi.local_abi_identity.clone();
    let generic_build = generic.package_build_id.clone();

    let mut reordered = generic.clone();
    callable_signature_mut(&mut reordered).type_params.reverse();
    assert_ne!(
        package_artifact_local_abi_identity(&reordered).unwrap(),
        generic_local
    );
    assert_ne!(
        package_artifact_build_identity(&reordered).unwrap(),
        generic_build
    );

    let mut missing = generic.clone();
    callable_signature_mut(&mut missing)
        .type_params
        .retain(|parameter| parameter != "Id");
    let error = package_artifact_build_identity(&missing)
        .expect_err("free Id must fail closed")
        .to_string();
    assert!(error.contains("out-of-scope type parameter Id"));

    for invalid_scope in [
        vec!["T".to_string(), "Id".to_string(), "Id".to_string()],
        vec!["T".to_string(), " Id".to_string()],
        vec!["T".to_string(), "1T".to_string()],
        vec!["T".to_string(), "T.id".to_string()],
    ] {
        let mut invalid = generic.clone();
        callable_signature_mut(&mut invalid).type_params = invalid_scope;
        assert!(matches!(
            package_artifact_build_identity(&invalid),
            Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
        ));
    }
}

#[test]
fn implementation_link_type_parameters_use_the_matching_public_callable_scope() {
    let mut artifact = callable_fixture();
    let signature = callable_signature_mut(&mut artifact);
    signature.type_params = vec!["T".to_string()];
    signature.parameters[0].ty = PackageTypeRef::Local {
        local_type: TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
    };
    signature.return_type = PackageTypeRef::Local {
        local_type: TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
    };
    set_boundary_unavailable(
        &mut artifact,
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    let callable_id = callable_id_for_path(&artifact, "run");
    let target = artifact.callable_links[&callable_id].target.clone();
    artifact.implementation_links.functions.insert(
        "run".to_string(),
        ExecutableExport {
            file: target.file_ref,
            executable_index: target.executable_index,
            symbol: "run".to_string(),
            signature: ExecutableSignatureIr {
                params: vec![ParamIr {
                    name: "value".to_string(),
                    slot: 0,
                    ty: TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                }],
                return_type: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
                self_type: None,
                may_suspend: false,
            },
        },
    );
    assign_package_artifact_identities(&mut artifact).unwrap();

    artifact
        .implementation_links
        .functions
        .get_mut("run")
        .unwrap()
        .signature
        .params[0]
        .ty = TypeRefIr::TypeParam {
        name: "Unbound".to_string(),
    };
    assert!(matches!(
        package_artifact_build_identity(&artifact),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));
}

#[test]
fn implementation_symbol_callable_type_parameter_scope_is_validated() {
    let mut artifact = callable_fixture();
    let mut symbol = artifact
        .package_local_abi
        .public_symbols
        .remove("run")
        .unwrap();
    let public_callable_id = match &symbol {
        PackageLocalAbiSymbol::Callable { callable_id, .. } => callable_id.clone(),
        _ => panic!("fixture run must be callable"),
    };
    let implementation_callable_id = PackageCallableId::new(format!(
        "pkg-callable:{}:top-level:api.run",
        artifact.package_id
    ));
    match &mut symbol {
        PackageLocalAbiSymbol::Callable {
            callable_id,
            signature,
        } => {
            *callable_id = implementation_callable_id.clone();
            signature.type_params = vec!["T".to_string()];
            signature.parameters[0].ty = PackageTypeRef::Local {
                local_type: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            };
            signature.return_type = PackageTypeRef::Local {
                local_type: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
            };
        }
        _ => panic!("fixture run must be callable"),
    }
    artifact
        .package_local_abi
        .implementation_symbols
        .insert("api.run".to_string(), symbol);
    artifact.implementation_links.functions.remove("run");
    artifact.boundary_projections.remove(&public_callable_id);
    let mut link = artifact.callable_links.remove(&public_callable_id).unwrap();
    link.callable_id = implementation_callable_id.clone();
    link.target.callable_abi_id = implementation_callable_id.to_string();
    link.target.callable_kind = OperationCallableKind::InternalFunction;
    artifact
        .callable_links
        .insert(implementation_callable_id.clone(), link);
    let facts = artifact
        .callable_semantic_facts
        .remove(&public_callable_id)
        .unwrap();
    artifact
        .callable_semantic_facts
        .insert(implementation_callable_id, facts);
    assign_package_artifact_identities(&mut artifact).unwrap();

    let PackageLocalAbiSymbol::Callable { signature, .. } = artifact
        .package_local_abi
        .implementation_symbols
        .get_mut("api.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.type_params.clear();
    assert!(matches!(
        package_artifact_build_identity(&artifact),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));
}

#[test]
fn implementation_internal_function_alias_accepts_exact_public_function_coordinate() {
    let (mut artifact, public_id, implementation_id) = shared_function_internal_callable_fixture();
    assign_package_artifact_identities(&mut artifact).unwrap();
    validate_package_artifact_identities(&artifact).unwrap();
    assert_eq!(
        artifact.callable_links[&public_id].target.file_ref,
        artifact.callable_links[&implementation_id].target.file_ref
    );
    assert_eq!(
        artifact.callable_links[&public_id].target.executable_index,
        artifact.callable_links[&implementation_id]
            .target
            .executable_index
    );
    assert_eq!(
        artifact.callable_links[&implementation_id]
            .target
            .callable_kind,
        OperationCallableKind::InternalFunction
    );
    assert!(!artifact
        .package_local_abi
        .public_symbols
        .values()
        .any(|symbol| matches!(
            symbol,
            PackageLocalAbiSymbol::Callable { callable_id, .. }
                if callable_id == &implementation_id
        )));
    assert!(!artifact
        .boundary_projections
        .contains_key(&implementation_id));
}

#[test]
fn implementation_internal_function_alias_rejects_wrong_identity_surface_owner_and_signature() {
    let (artifact, public_id, implementation_id) = shared_function_internal_callable_fixture();

    let mut wrong_id = artifact.clone();
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = wrong_id
        .package_local_abi
        .implementation_symbols
        .get_mut("api.run")
        .unwrap()
    else {
        unreachable!()
    };
    *callable_id = PackageCallableId::new(format!("{implementation_id}:forged"));
    assert_invalid_package_artifact(&wrong_id);

    let mut wrong_surface = artifact.clone();
    wrong_surface.package_local_abi.public_symbols.insert(
        "top-level:api.run".to_string(),
        wrong_surface.package_local_abi.implementation_symbols["api.run"].clone(),
    );
    assert_invalid_package_artifact(&wrong_surface);

    let mut wrong_owner = artifact.clone();
    let symbol = wrong_owner
        .package_local_abi
        .implementation_symbols
        .remove("api.run")
        .unwrap();
    wrong_owner
        .package_local_abi
        .implementation_symbols
        .insert("other.run".to_string(), symbol);
    assert_invalid_package_artifact(&wrong_owner);

    let mut wrong_signature = artifact.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = wrong_signature
        .package_local_abi
        .implementation_symbols
        .get_mut("api.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.return_type = PackageTypeRef::Local {
        local_type: TypeRefIr::TypeParam {
            name: "Missing".to_string(),
        },
    };
    assert_invalid_package_artifact(&wrong_signature);

    let mut missing_public_owner = artifact;
    missing_public_owner
        .package_local_abi
        .public_symbols
        .remove("run");
    missing_public_owner.callable_links.remove(&public_id);
    missing_public_owner
        .callable_semantic_facts
        .remove(&public_id);
    missing_public_owner.boundary_projections.remove(&public_id);
    let error = package_artifact_build_identity(&missing_public_owner)
        .expect_err("an exported function coordinate still requires its public callable")
        .to_string();
    assert!(error.contains("without an exact public function callable"));
}

#[test]
fn implementation_only_impl_callable_scope_accepts_exact_shared_executable() {
    let (mut artifact, public_id, implementation_id) = implementation_only_impl_callable_fixture();
    assign_package_artifact_identities(&mut artifact).unwrap();
    validate_package_artifact_identities(&artifact).unwrap();

    assert!(artifact
        .package_local_abi
        .public_symbols
        .values()
        .any(|symbol| matches!(
            symbol,
            PackageLocalAbiSymbol::Callable { callable_id, .. }
                if callable_id == &public_id
        )));
    assert!(artifact
        .package_local_abi
        .implementation_symbols
        .values()
        .any(|symbol| matches!(
            symbol,
            PackageLocalAbiSymbol::Callable { callable_id, .. }
                if callable_id == &implementation_id
        )));
    assert!(!artifact
        .package_local_abi
        .public_symbols
        .values()
        .any(|symbol| matches!(
            symbol,
            PackageLocalAbiSymbol::Callable { callable_id, .. }
                if callable_id == &implementation_id
        )));
    assert!(artifact.boundary_projections.contains_key(&public_id));
    assert!(!artifact
        .boundary_projections
        .contains_key(&implementation_id));
}

#[test]
fn implementation_only_impl_callable_scope_accepts_private_impl_method() {
    let (artifact, implementation_id) = implementation_only_private_impl_callable_fixture();
    validate_package_artifact_identities(&artifact).unwrap();
    assert!(artifact.package_local_abi.public_symbols.is_empty());
    assert!(artifact.boundary_projections.is_empty());
    assert_eq!(
        artifact.callable_links[&implementation_id]
            .target
            .callable_kind,
        OperationCallableKind::ImplMethod
    );
}

#[test]
fn implementation_only_impl_callable_scope_rejects_duplicate_missing_and_non_callable_owner() {
    let (artifact, _, implementation_id) = implementation_only_impl_callable_fixture();

    let mut duplicate = artifact.clone();
    duplicate.package_local_abi.public_symbols.insert(
        "top-level:api.Worker.run".to_string(),
        duplicate.package_local_abi.implementation_symbols["api.Worker.run"].clone(),
    );
    assert_invalid_package_artifact(&duplicate);

    let mut missing = artifact.clone();
    missing
        .package_local_abi
        .implementation_symbols
        .remove("api.Worker.run");
    let error = package_artifact_build_identity(&missing)
        .expect_err("a link without an exact callable surface must fail")
        .to_string();
    assert!(error.contains(implementation_id.as_str()));

    let mut non_callable = artifact;
    non_callable
        .package_local_abi
        .implementation_symbols
        .insert(
            "api.Worker.run".to_string(),
            PackageLocalAbiSymbol::Constant {
                const_id: format!(
                    "pkg-const:{}:top-level:api.Worker.run",
                    non_callable.package_id
                ),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
            },
        );
    assert_invalid_package_artifact(&non_callable);
}

#[test]
fn implementation_only_impl_callable_scope_rejects_wrong_owner_target_and_kinds() {
    let (artifact, public_id, implementation_id) = implementation_only_impl_callable_fixture();

    let mut wrong_owner = artifact.clone();
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = wrong_owner
        .package_local_abi
        .implementation_symbols
        .get_mut("api.Worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    *callable_id = public_id;
    let error = package_artifact_build_identity(&wrong_owner)
        .expect_err("a non-canonical implementation callable owner must fail")
        .to_string();
    assert!(error.contains("non-canonical callable id"));
    assert!(error.contains(implementation_id.as_str()));

    let mut wrong_target = artifact.clone();
    wrong_target
        .callable_links
        .get_mut(&implementation_id)
        .unwrap()
        .target
        .executable_index += 1;
    let error = package_artifact_build_identity(&wrong_target)
        .expect_err("an implementation method outside the exact method target must fail")
        .to_string();
    assert!(error.contains(implementation_id.as_str()));
    assert!(error.contains("outside implementationLinks.implMethods"));

    for wrong_kind in [
        OperationCallableKind::InternalFunction,
        OperationCallableKind::PublicFunction,
        OperationCallableKind::ReceiverMethod,
    ] {
        let mut wrong = artifact.clone();
        wrong
            .callable_links
            .get_mut(&implementation_id)
            .unwrap()
            .target
            .callable_kind = wrong_kind;
        let error = package_artifact_build_identity(&wrong)
            .expect_err("an implementation-only impl callable kind mismatch must fail")
            .to_string();
        assert!(
            error.contains(implementation_id.as_str()),
            "error must identify the wrong-kind callable: {error}"
        );
    }
}

#[test]
fn implementation_only_impl_callable_scope_validates_every_exact_signature_scope() {
    let (mut compatible, public_id, implementation_id) = shared_impl_callable_scope_fixture();
    let PackageLocalAbiSymbol::Callable { signature, .. } = compatible
        .package_local_abi
        .implementation_symbols
        .get_mut("api.Worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.type_params = vec!["ImplementationT".to_string()];
    package_artifact_build_identity(&compatible)
        .expect("different unused alias scopes are executable-compatible");

    let PackageLocalAbiSymbol::Callable { signature, .. } = compatible
        .package_local_abi
        .public_symbols
        .get_mut("run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.type_params = vec!["PublicT".to_string()];
    signature.return_type = PackageTypeRef::Local {
        local_type: TypeRefIr::TypeParam {
            name: "PublicT".to_string(),
        },
    };
    compatible
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .return_type = TypeRefIr::TypeParam {
        name: "PublicT".to_string(),
    };
    let error = package_artifact_build_identity(&compatible)
        .expect_err("the executable must validate against every exact callable scope")
        .to_string();
    assert!(
        error.contains("out-of-scope type parameter PublicT"),
        "{error}"
    );

    let (mut private, private_id) = implementation_only_private_impl_callable_fixture();
    private
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .signature
        .return_type = TypeRefIr::TypeParam {
        name: "Missing".to_string(),
    };
    let error = package_artifact_build_identity(&private)
        .expect_err("a private impl executable may only use its exact callable scope")
        .to_string();
    assert!(error.contains("out-of-scope type parameter Missing"));
    assert!(private.callable_links.contains_key(&private_id));
    assert!(compatible.callable_links.contains_key(&public_id));
    assert!(compatible.callable_links.contains_key(&implementation_id));
}

#[test]
fn implementation_only_impl_callable_scope_is_canonical_and_identity_bearing() {
    let (mut artifact, _, implementation_id) = implementation_only_impl_callable_fixture();
    assign_package_artifact_identities(&mut artifact).unwrap();
    let baseline = artifact.package_build_id.clone();

    let mut signature_changed = artifact.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = signature_changed
        .package_local_abi
        .implementation_symbols
        .get_mut("api.Worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    signature.type_params.push("T".to_string());
    assert_ne!(
        package_artifact_build_identity(&signature_changed).unwrap(),
        baseline
    );

    let (mut target_changed, private_id) = implementation_only_private_impl_callable_fixture();
    let private_baseline = target_changed.package_build_id.clone();
    target_changed
        .callable_links
        .get_mut(&private_id)
        .unwrap()
        .target
        .executable_index = 1;
    target_changed
        .implementation_links
        .impl_methods
        .get_mut("Worker.run")
        .unwrap()
        .executable_index = 1;
    assert_ne!(
        package_artifact_build_identity(&target_changed).unwrap(),
        private_baseline
    );

    let mut id_changed = artifact.clone();
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = id_changed
        .package_local_abi
        .implementation_symbols
        .get_mut("api.Worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    *callable_id = PackageCallableId::new(format!("{implementation_id}:forged"));
    assert_invalid_package_artifact(&id_changed);
}

#[test]
fn applied_nominal_argument_matrix_changes_local_abi_and_build_and_rejects_tampering() {
    let mut string_box = callable_fixture();
    set_parameter_local_type(
        &mut string_box,
        applied_package_nominal("Box", vec![TypeRefIr::builtin("string")]),
    );
    set_boundary_unavailable(
        &mut string_box,
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    let mut number_box = callable_fixture();
    set_parameter_local_type(
        &mut number_box,
        applied_package_nominal("Box", vec![TypeRefIr::builtin("number")]),
    );
    set_boundary_unavailable(
        &mut number_box,
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    assert_ne!(
        package_artifact_local_abi_identity(&string_box).unwrap(),
        package_artifact_local_abi_identity(&number_box).unwrap()
    );
    assert_ne!(
        package_artifact_build_identity(&string_box).unwrap(),
        package_artifact_build_identity(&number_box).unwrap()
    );

    let mut ordered = callable_fixture();
    set_parameter_local_type(
        &mut ordered,
        applied_package_nominal(
            "Box",
            vec![applied_package_nominal(
                "Pair",
                vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
            )],
        ),
    );
    set_boundary_unavailable(
        &mut ordered,
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    let mut reordered = callable_fixture();
    set_parameter_local_type(
        &mut reordered,
        applied_package_nominal(
            "Box",
            vec![applied_package_nominal(
                "Pair",
                vec![TypeRefIr::builtin("number"), TypeRefIr::builtin("string")],
            )],
        ),
    );
    set_boundary_unavailable(
        &mut reordered,
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    assert_ne!(
        package_artifact_local_abi_identity(&ordered).unwrap(),
        package_artifact_local_abi_identity(&reordered).unwrap()
    );
    assert_ne!(
        package_artifact_build_identity(&ordered).unwrap(),
        package_artifact_build_identity(&reordered).unwrap()
    );

    assign_package_artifact_identities(&mut string_box).unwrap();
    let PackageTypeRef::Local { local_type } =
        &mut callable_signature_mut(&mut string_box).parameters[0].ty
    else {
        panic!("fixture parameter must be local")
    };
    let TypeRefIr::AppliedNominal { arguments, .. } = local_type else {
        panic!("fixture parameter must be applied")
    };
    arguments[0] = TypeRefIr::builtin("number");
    assert!(matches!(
        validate_package_artifact_identities(&string_box),
        Err(ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch { .. })
    ));

    assign_package_artifact_identities(&mut ordered).unwrap();
    let PackageTypeRef::Local { local_type } =
        &mut callable_signature_mut(&mut ordered).parameters[0].ty
    else {
        panic!("fixture parameter must be local")
    };
    let TypeRefIr::AppliedNominal { base, .. } = local_type else {
        panic!("fixture parameter must be applied")
    };
    let NominalTypeRefBaseIr::PackageSymbol { symbol } = base else {
        panic!("fixture base must be a package symbol")
    };
    symbol.package = PackageRefIr::PackageId {
        package_id: "example.other-model".to_string(),
    };
    assert!(matches!(
        validate_package_artifact_identities(&ordered),
        Err(ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch { .. })
    ));
}

#[test]
fn package_artifact_admission_rejects_empty_and_applied_package_schema() {
    let mut empty = callable_fixture();
    set_parameter_local_type(
        &mut empty,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: package_symbol("Box"),
            },
            arguments: Vec::new(),
        },
    );
    assert!(matches!(
        package_artifact_local_abi_identity(&empty),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));

    let mut package_schema = callable_fixture();
    set_parameter_local_type(
        &mut package_schema,
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSchema {
                package_id: "example.model".to_string(),
                stable_schema_key: "Box".to_string(),
                package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new(
                    "schema:box",
                ),
            },
            arguments: vec![TypeRefIr::builtin("string")],
        },
    );
    assert!(matches!(
        package_artifact_local_abi_identity(&package_schema),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));
}

#[test]
fn removed_dependency_collection_mapping_wire_is_rejected() {
    let mut empty = fixture();
    empty.package_requirements.push(PackageRequirement {
        alias: "store".to_string(),
        package_id: "example.store".to_string(),
        exact_version: "1.0.0".to_string(),
        expected_local_abi: PackageLocalAbiIdentity::new("store-abi"),
        expected_package_build: None,
    });
    let mut removed_wire = serde_json::to_value(&empty).unwrap();
    removed_wire["packageRequirements"][0]["collectionNameMapping"] = serde_json::json!({});
    assert!(serde_json::from_value::<PackageArtifact>(removed_wire).is_err());
}

fn fixture() -> PackageArtifact {
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.identity".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode: None,
        package_local_abi: skiff_artifact_model::PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.identity".to_string(),
            package_schema_index_identity: crate::package_schema_index_identity(
                "example.identity",
                &BTreeMap::new(),
            )
            .unwrap(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn actor_fixture() -> PackageArtifact {
    let mut artifact = fixture();
    let file = FileIrRef::new(
        "skiff-file-ir-v12:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "thread_actor",
    );
    artifact.files.push(file.clone());
    let abi = actor_abi();
    let record = TypeDescriptorIr::Record {
        fields: BTreeMap::from([
            ("id".to_string(), TypeRefIr::builtin("u64")),
            ("label".to_string(), TypeRefIr::builtin("string")),
        ]),
    };
    artifact.package_local_abi.public_symbols.insert(
        "ThreadActor".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: "type:ThreadActor".to_string(),
            descriptor: record.clone(),
            is_alias: false,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: Some(abi.clone()),
        },
    );
    artifact.package_local_abi.implementation_symbols.insert(
        "thread_actor.ThreadActor".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: "type:example.identity:top-level:thread_actor.ThreadActor".to_string(),
            descriptor: record.clone(),
            is_alias: false,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: Some(abi.clone()),
        },
    );
    let public_link = TypeExport {
        file: file.clone(),
        type_index: 0,
        symbol: "ThreadActor".to_string(),
        is_interface: false,
        descriptor: Some(record.clone()),
        type_params: Vec::new(),
        interface_methods: Vec::new(),
        actor: Some(abi.clone()),
    };
    artifact
        .implementation_links
        .types
        .insert("ThreadActor".to_string(), public_link);
    artifact.implementation_links.types.insert(
        "thread_actor.ThreadActor".to_string(),
        TypeExport {
            file,
            type_index: 0,
            symbol: "thread_actor.ThreadActor".to_string(),
            is_interface: false,
            descriptor: Some(record),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: Some(abi),
        },
    );
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn actor_abi() -> PackageActorAbi {
    PackageActorAbi {
        actor_abi_identity: ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:thread-actor"),
        abi: ActorAbiInput {
            actor_name: "ThreadActor".to_string(),
            actor_id_type: TypeRefIr::builtin("u64"),
            key_field: "id".to_string(),
            fields: vec![
                ActorFieldIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::builtin("u64"),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
                ActorFieldIr {
                    name: "label".to_string(),
                    ty: TypeRefIr::builtin("string"),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
            ],
            create: Some(ActorCreateSignatureIr {
                parameters: vec![FunctionTypeParamIr {
                    name: "label".to_string(),
                    ty: TypeRefIr::builtin("string"),
                }],
            }),
            public_methods: vec![ActorPublicMethodIr {
                method_identity: ActorMethodIdentity::new("skiff-actor-method-v1:sha256:read"),
                name: "read".to_string(),
                parameters: Vec::new(),
                return_type: TypeRefIr::builtin("string"),
                may_suspend: false,
            }],
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        },
    }
}

fn callable_fixture() -> PackageArtifact {
    let mut artifact = fixture();
    let callable_id = PackageCallableId::new("pkg-callable:example.identity:run");
    let file = FileIrRef::new(
        "skiff-file-ir-v7:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "api",
    );
    artifact.files.push(file.clone());
    artifact.package_local_abi.public_symbols.insert(
        "run".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: callable_id.clone(),
            signature: PackageCallableSignature {
                type_params: Vec::new(),
                parameters: vec![PackageCallableParameter {
                    name: "value".to_string(),
                    ty: PackageTypeRef::Local {
                        local_type: TypeRefIr::builtin("string"),
                    },
                }],
                return_type: PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("string"),
                },
                may_suspend: false,
            },
        },
    );
    artifact.callable_links.insert(
        callable_id.clone(),
        PackageCallableLinkFact {
            callable_id: callable_id.clone(),
            target: OperationTargetRef {
                file_ref: file.clone(),
                executable_index: 0,
                callable_abi_id: callable_id.to_string(),
                callable_kind: OperationCallableKind::PublicFunction,
            },
        },
    );
    artifact.implementation_links.functions.insert(
        "run".to_string(),
        ExecutableExport {
            file,
            executable_index: 0,
            symbol: "run".to_string(),
            signature: ExecutableSignatureIr {
                params: vec![ParamIr {
                    name: "value".to_string(),
                    slot: 0,
                    ty: TypeRefIr::builtin("string"),
                }],
                return_type: TypeRefIr::builtin("string"),
                self_type: None,
                may_suspend: false,
            },
        },
    );
    let effects = CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    };
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: vec![ValueProvenance::Fresh],
        direct_return_origins: vec![ValueProvenance::Fresh],
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    artifact.callable_semantic_facts.insert(
        callable_id.clone(),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed {
                effects: effects.clone(),
            },
            provenance: provenance.clone(),
            resolved_call_targets: BTreeMap::new(),
        },
    );
    artifact.boundary_projections.insert(
        callable_id,
        BoundaryCallableProjection::Available {
            operation_contract: BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "value".to_string(),
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: detached_plan(
                        BoundaryValueOwner::Caller,
                        BoundaryValueLifetime::Call,
                    ),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: detached_plan(
                        BoundaryValueOwner::Provider,
                        BoundaryValueLifetime::Call,
                    ),
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
            },
            implementation_requirements: BoundaryImplementationRequirements {
                config: Vec::new(),
                state: Vec::new(),
                native_capabilities: Vec::new(),
                complete_may_effects: effects,
                provenance,
            },
        },
    );
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn implementation_only_impl_callable_fixture(
) -> (PackageArtifact, PackageCallableId, PackageCallableId) {
    let mut artifact = callable_fixture();
    let old_public_id = callable_id_for_path(&artifact, "run");
    let public_id =
        PackageCallableId::new(format!("pkg-callable:{}:worker.run", artifact.package_id));
    let implementation_id = PackageCallableId::new(format!(
        "pkg-callable:{}:top-level:api.Worker.run",
        artifact.package_id
    ));

    let mut public_symbol = artifact
        .package_local_abi
        .public_symbols
        .remove("run")
        .unwrap();
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = &mut public_symbol else {
        unreachable!()
    };
    *callable_id = public_id.clone();
    let implementation_symbol = match &public_symbol {
        PackageLocalAbiSymbol::Callable { signature, .. } => PackageLocalAbiSymbol::Callable {
            callable_id: implementation_id.clone(),
            signature: signature.clone(),
        },
        _ => unreachable!(),
    };
    artifact
        .package_local_abi
        .public_symbols
        .insert("worker.run".to_string(), public_symbol);

    let mut public_link = artifact.callable_links.remove(&old_public_id).unwrap();
    public_link.callable_id = public_id.clone();
    public_link.target.callable_abi_id = public_id.to_string();
    public_link.target.callable_kind = OperationCallableKind::ImplMethod;
    let file = public_link.target.file_ref.clone();
    let executable_index = public_link.target.executable_index;
    artifact
        .callable_links
        .insert(public_id.clone(), public_link.clone());
    let facts = artifact
        .callable_semantic_facts
        .remove(&old_public_id)
        .unwrap();
    artifact
        .callable_semantic_facts
        .insert(public_id.clone(), facts.clone());
    let boundary = artifact
        .boundary_projections
        .remove(&old_public_id)
        .unwrap();
    artifact
        .boundary_projections
        .insert(public_id.clone(), boundary);

    let receiver_type = TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "api".to_string(),
            symbol: "Worker".to_string(),
        },
    };
    let implementation_receiver_type = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: artifact.package_id.clone(),
            },
            symbol_path: "api.Worker".to_string(),
            abi_expectation: None,
        },
    };
    let interface_methods = vec![InterfaceMethodSignature {
        name: "run".to_string(),
        type_params: Vec::new(),
        params: vec![
            FunctionTypeParamIr {
                name: "self".to_string(),
                ty: TypeRefIr::builtin("Self"),
            },
            FunctionTypeParamIr {
                name: "value".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
        ],
        return_type: TypeRefIr::builtin("string"),
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    }];
    artifact.package_local_abi.implementation_symbols.insert(
        "api.Worker".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: format!("type:{}:top-level:api.Worker", artifact.package_id),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            is_alias: false,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
    artifact.package_local_abi.implementation_symbols.insert(
        "api.WorkerApi".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: format!("type:{}:top-level:api.WorkerApi", artifact.package_id),
            descriptor: TypeDescriptorIr::Interface,
            is_alias: false,
            is_interface: true,
            type_params: Vec::new(),
            interface_methods: interface_methods.clone(),
            actor: None,
        },
    );
    artifact.package_local_abi.implementation_symbols.insert(
        "api.worker".to_string(),
        PackageLocalAbiSymbol::Constant {
            const_id: format!("pkg-const:{}:top-level:api.worker", artifact.package_id),
            ty: PackageTypeRef::Local {
                local_type: implementation_receiver_type.clone(),
            },
        },
    );
    artifact.implementation_links.types.insert(
        "api.Worker".to_string(),
        TypeExport {
            file: file.clone(),
            type_index: 0,
            symbol: "api.Worker".to_string(),
            is_interface: false,
            descriptor: Some(TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            }),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
    artifact.implementation_links.types.insert(
        "api.WorkerApi".to_string(),
        TypeExport {
            file: file.clone(),
            type_index: 1,
            symbol: "api.WorkerApi".to_string(),
            is_interface: true,
            descriptor: Some(TypeDescriptorIr::Interface),
            type_params: Vec::new(),
            interface_methods: interface_methods,
            actor: None,
        },
    );
    artifact.implementation_links.constants.insert(
        "worker".to_string(),
        ConstExport {
            file: file.clone(),
            const_index: 0,
            symbol: "worker".to_string(),
            ty: receiver_type.clone(),
        },
    );
    artifact.implementation_links.constants.insert(
        "api.worker".to_string(),
        ConstExport {
            file: file.clone(),
            const_index: 0,
            symbol: "api.worker".to_string(),
            ty: implementation_receiver_type,
        },
    );
    artifact.package_local_abi.public_symbols.insert(
        "worker".to_string(),
        PackageLocalAbiSymbol::PublicInstance {
            instance_id: "worker".to_string(),
            declared_receiver_type: receiver_type.clone(),
            interfaces: vec![TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: artifact.package_id.clone(),
                    },
                    symbol_path: "api.WorkerApi".to_string(),
                    abi_expectation: None,
                },
            }],
            methods: BTreeMap::from([("run".to_string(), public_id.clone())]),
        },
    );
    artifact.implementation_links.functions.remove("run");
    artifact.implementation_links.impl_methods.insert(
        "Worker.run".to_string(),
        ExecutableExport {
            file: file.clone(),
            executable_index,
            symbol: "Worker.run".to_string(),
            signature: ExecutableSignatureIr {
                params: vec![ParamIr {
                    name: "value".to_string(),
                    slot: 1,
                    ty: TypeRefIr::builtin("string"),
                }],
                return_type: TypeRefIr::builtin("string"),
                self_type: Some(receiver_type),
                may_suspend: false,
            },
        },
    );
    assign_package_artifact_identities(&mut artifact).unwrap();

    artifact
        .package_local_abi
        .implementation_symbols
        .insert("api.Worker.run".to_string(), implementation_symbol);
    let mut implementation_link = public_link;
    implementation_link.callable_id = implementation_id.clone();
    implementation_link.target.callable_abi_id = implementation_id.to_string();
    artifact
        .callable_links
        .insert(implementation_id.clone(), implementation_link);
    artifact
        .callable_semantic_facts
        .insert(implementation_id.clone(), facts);
    (artifact, public_id, implementation_id)
}

fn implementation_only_private_impl_callable_fixture() -> (PackageArtifact, PackageCallableId) {
    let mut artifact = callable_fixture();
    let public_id = callable_id_for_path(&artifact, "run");
    let implementation_id = PackageCallableId::new(format!(
        "pkg-callable:{}:top-level:api.Worker.run",
        artifact.package_id
    ));
    let mut implementation_symbol = artifact
        .package_local_abi
        .public_symbols
        .remove("run")
        .unwrap();
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = &mut implementation_symbol else {
        unreachable!()
    };
    *callable_id = implementation_id.clone();
    artifact
        .package_local_abi
        .implementation_symbols
        .insert("api.Worker.run".to_string(), implementation_symbol);

    let mut link = artifact.callable_links.remove(&public_id).unwrap();
    link.callable_id = implementation_id.clone();
    link.target.callable_abi_id = implementation_id.to_string();
    link.target.callable_kind = OperationCallableKind::ImplMethod;
    artifact
        .callable_links
        .insert(implementation_id.clone(), link);

    let facts = artifact.callable_semantic_facts.remove(&public_id).unwrap();
    artifact
        .callable_semantic_facts
        .insert(implementation_id.clone(), facts);
    artifact.boundary_projections.remove(&public_id);

    let mut executable = artifact
        .implementation_links
        .functions
        .remove("run")
        .unwrap();
    executable.symbol = "Worker.run".to_string();
    artifact
        .implementation_links
        .impl_methods
        .insert("Worker.run".to_string(), executable);

    assign_package_artifact_identities(&mut artifact).unwrap();
    (artifact, implementation_id)
}

fn shared_function_internal_callable_fixture(
) -> (PackageArtifact, PackageCallableId, PackageCallableId) {
    let mut artifact = callable_fixture();
    let public_id = callable_id_for_path(&artifact, "run");
    let implementation_id = PackageCallableId::new(format!(
        "pkg-callable:{}:top-level:api.run",
        artifact.package_id
    ));
    let PackageLocalAbiSymbol::Callable { signature, .. } =
        &artifact.package_local_abi.public_symbols["run"]
    else {
        unreachable!()
    };
    artifact.package_local_abi.implementation_symbols.insert(
        "api.run".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: implementation_id.clone(),
            signature: signature.clone(),
        },
    );
    let mut link = artifact.callable_links[&public_id].clone();
    link.callable_id = implementation_id.clone();
    link.target.callable_abi_id = implementation_id.to_string();
    link.target.callable_kind = OperationCallableKind::InternalFunction;
    artifact
        .callable_links
        .insert(implementation_id.clone(), link);
    artifact.callable_semantic_facts.insert(
        implementation_id.clone(),
        artifact.callable_semantic_facts[&public_id].clone(),
    );
    (artifact, public_id, implementation_id)
}

fn shared_impl_callable_scope_fixture() -> (PackageArtifact, PackageCallableId, PackageCallableId) {
    let (mut artifact, implementation_id) = implementation_only_private_impl_callable_fixture();
    let public_id = PackageCallableId::new(format!("pkg-callable:{}:run", artifact.package_id));
    let PackageLocalAbiSymbol::Callable { signature, .. } = artifact
        .package_local_abi
        .implementation_symbols
        .get("api.Worker.run")
        .unwrap()
    else {
        unreachable!()
    };
    artifact.package_local_abi.public_symbols.insert(
        "run".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: public_id.clone(),
            signature: signature.clone(),
        },
    );
    let mut link = artifact.callable_links[&implementation_id].clone();
    link.callable_id = public_id.clone();
    link.target.callable_abi_id = public_id.to_string();
    artifact.callable_links.insert(public_id.clone(), link);
    artifact.callable_semantic_facts.insert(
        public_id.clone(),
        artifact.callable_semantic_facts[&implementation_id].clone(),
    );
    let template = callable_fixture();
    artifact.boundary_projections.insert(
        public_id.clone(),
        template
            .boundary_projections
            .values()
            .next()
            .unwrap()
            .clone(),
    );
    assign_package_artifact_identities(&mut artifact).unwrap();
    (artifact, public_id, implementation_id)
}

pub(super) fn two_callable_fixture() -> PackageArtifact {
    let mut artifact = callable_fixture();
    let run_id = callable_id_for_path(&artifact, "run");
    let echo_id = PackageCallableId::new("pkg-callable:example.identity:echo");
    let mut echo_symbol = artifact.package_local_abi.public_symbols["run"].clone();
    let PackageLocalAbiSymbol::Callable { callable_id, .. } = &mut echo_symbol else {
        unreachable!()
    };
    *callable_id = echo_id.clone();
    artifact
        .package_local_abi
        .public_symbols
        .insert("echo".to_string(), echo_symbol);

    let mut echo_link = artifact.callable_links[&run_id].clone();
    echo_link.callable_id = echo_id.clone();
    echo_link.target.callable_abi_id = echo_id.to_string();
    echo_link.target.executable_index = 1;
    artifact.callable_links.insert(echo_id.clone(), echo_link);
    let mut echo_export = artifact.implementation_links.functions["run"].clone();
    echo_export.executable_index = 1;
    echo_export.symbol = "echo".to_string();
    artifact
        .implementation_links
        .functions
        .insert("echo".to_string(), echo_export);
    artifact.callable_semantic_facts.insert(
        echo_id.clone(),
        artifact.callable_semantic_facts[&run_id].clone(),
    );
    artifact
        .boundary_projections
        .insert(echo_id, artifact.boundary_projections[&run_id].clone());
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

pub(super) fn assert_invalid_package_artifact(artifact: &PackageArtifact) {
    assert!(matches!(
        package_artifact_build_identity(artifact),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));
}

pub(super) fn callable_id_for_path(artifact: &PackageArtifact, path: &str) -> PackageCallableId {
    let PackageLocalAbiSymbol::Callable { callable_id, .. } =
        &artifact.package_local_abi.public_symbols[path]
    else {
        panic!("{path} must be callable")
    };
    callable_id.clone()
}

fn callable_signature_mut(artifact: &mut PackageArtifact) -> &mut PackageCallableSignature {
    let PackageLocalAbiSymbol::Callable { signature, .. } = artifact
        .package_local_abi
        .public_symbols
        .get_mut("run")
        .unwrap()
    else {
        panic!("fixture run must be callable")
    };
    signature
}

fn detached_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime,
    }
}

fn available_operation_mut(artifact: &mut PackageArtifact) -> &mut BoundaryOperationContract {
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = artifact.boundary_projections.values_mut().next().unwrap()
    else {
        panic!("fixture boundary projection must be available")
    };
    operation_contract
}

fn set_boundary_unavailable(
    artifact: &mut PackageArtifact,
    reasons: Vec<BoundaryUnavailableReason>,
) {
    *artifact.boundary_projections.values_mut().next().unwrap() =
        BoundaryCallableProjection::Unavailable { reasons };
}

fn set_plan_carrier(plan: &mut BoundaryValuePlan, value: BoundaryValueCarrier) {
    let BoundaryValuePlan::Linkable { carrier, .. } = plan else {
        unreachable!()
    };
    *carrier = value;
}

fn set_plan_encoding(plan: &mut BoundaryValuePlan, value: BoundaryValueEncoding) {
    let BoundaryValuePlan::Linkable { encoding, .. } = plan else {
        unreachable!()
    };
    *encoding = value;
}

fn set_plan_owner(plan: &mut BoundaryValuePlan, value: BoundaryValueOwner) {
    let BoundaryValuePlan::Linkable { owner, .. } = plan else {
        unreachable!()
    };
    *owner = value;
}

fn set_plan_lifetime(plan: &mut BoundaryValuePlan, value: BoundaryValueLifetime) {
    let BoundaryValuePlan::Linkable { lifetime, .. } = plan else {
        unreachable!()
    };
    *lifetime = value;
}

fn assert_all_package_identity_entries_reject(artifact: &PackageArtifact, label: &str) {
    macro_rules! assert_invalid {
        ($result:expr) => {
            assert!(
                matches!(
                    $result,
                    Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
                ),
                "{label} must be rejected as InvalidPackageArtifact"
            )
        };
    }

    assert_invalid!(package_artifact_local_abi_identity_projection(artifact));
    assert_invalid!(package_artifact_local_abi_identity(artifact));
    assert_invalid!(package_artifact_build_identity_projection(artifact));
    assert_invalid!(package_artifact_build_identity(artifact));
    assert_invalid!(validate_package_artifact_identities(artifact));
    assert_invalid!(package_artifact_ref(artifact));
    let mut assignable = artifact.clone();
    assert_invalid!(assign_package_artifact_identities(&mut assignable));
}

fn set_parameter_local_type(artifact: &mut PackageArtifact, local_type: TypeRefIr) {
    callable_signature_mut(artifact).parameters[0].ty = PackageTypeRef::Local { local_type };
}

fn applied_package_nominal(symbol_path: &str, arguments: Vec<TypeRefIr>) -> TypeRefIr {
    TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::PackageSymbol {
            symbol: package_symbol(symbol_path),
        },
        arguments,
    }
}

fn package_symbol(symbol_path: &str) -> PackageSymbolRef {
    PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: "example.model".to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: Some("model-abi".to_string()),
    }
}
mod public_instance;

fn bytecode_identity_leaf(character: char) -> String {
    format!(
        "{}:{}",
        crate::BYTECODE_IDENTITY_PREFIX,
        std::iter::repeat_n(character, 64).collect::<String>()
    )
}

#[test]
fn bytecode_identity_enters_build_preimage_but_not_local_abi() {
    let base = fixture();
    let base_local = package_artifact_local_abi_identity(&base).unwrap();
    let base_build = package_artifact_build_identity(&base).unwrap();

    let mut with_bytecode = base.clone();
    with_bytecode.bytecode = Some(skiff_artifact_model::BytecodeArtifactRef {
        bytecode_identity: bytecode_identity_leaf('a'),
        artifact_path: None,
    });
    assign_package_artifact_identities(&mut with_bytecode).unwrap();

    // Local ABI projection stays bytecode-free: direct package dependents do
    // not recompile when bytecode content changes (R-105/R-125).
    assert_eq!(
        package_artifact_local_abi_identity(&with_bytecode).unwrap(),
        base_local
    );
    assert_ne!(
        package_artifact_build_identity(&with_bytecode).unwrap(),
        base_build,
        "bytecode identity must enter the build preimage"
    );

    let build_projection = package_artifact_build_identity_projection(&with_bytecode).unwrap();
    assert_eq!(
        serde_json::to_value(&build_projection).unwrap()["bytecode"]["bytecodeIdentity"],
        serde_json::json!(bytecode_identity_leaf('a'))
    );
    let base_projection = package_artifact_build_identity_projection(&base).unwrap();
    assert!(
        serde_json::to_value(&base_projection)
            .unwrap()
            .get("bytecode")
            .is_none(),
        "bytecode must be skipped when absent (D18)"
    );

    // Any bytecode content change changes the build identity.
    let mut changed_bytecode = with_bytecode.clone();
    changed_bytecode
        .bytecode
        .as_mut()
        .unwrap()
        .bytecode_identity = bytecode_identity_leaf('b');
    assign_package_artifact_identities(&mut changed_bytecode).unwrap();
    assert_ne!(
        package_artifact_build_identity(&changed_bytecode).unwrap(),
        package_artifact_build_identity(&with_bytecode).unwrap()
    );
    assert_eq!(
        package_artifact_local_abi_identity(&changed_bytecode).unwrap(),
        base_local
    );

    validate_package_artifact_identities(&with_bytecode).unwrap();
}

#[test]
fn malformed_bytecode_owner_identity_is_rejected_at_package_surface() {
    let mut malformed = fixture();
    malformed.bytecode = Some(skiff_artifact_model::BytecodeArtifactRef {
        bytecode_identity: "skiff-bytecode-image-v2:sha256:short".to_string(),
        artifact_path: None,
    });
    assert!(matches!(
        assign_package_artifact_identities(&mut malformed),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));
    assert!(matches!(
        validate_package_artifact_identities(&malformed),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));
}
