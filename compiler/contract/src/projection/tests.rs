use skiff_artifact_model::{
    BoundaryCallbackExpirationError, BoundaryCallbackLifetime, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryParameter, BoundaryReturn, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    PackageBuildId, PackageCallableSignature, PackageImplementationLinks, PackageLocalAbi,
    PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexIdentity,
    PackageSchemaIndexRef, PackageSchemaTypeRef, PackageTypeRef, TypeRefIr,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn operation_schema_roots_are_only_parameter_return_stream_and_callback_types() {
    let ids = ["parameter", "return", "stream", "callback"]
        .map(|key| PackageSchemaTypeId::new(format!("type:{key}")));
    let reference = |key: &str, id: PackageSchemaTypeId| {
        ContractTypeRef::package_schema("example.types", key, id)
    };
    let operation = BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "input".to_string(),
            ty: reference("parameter", ids[0].clone()),
            value_plan: value_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: reference("return", ids[1].clone()),
            value_plan: value_plan(BoundaryValueOwner::Provider),
        },
        stream: BoundaryStreamContract::ServerStream {
            item_type: reference("stream", ids[2].clone()),
            item_value_plan: value_plan(BoundaryValueOwner::Provider),
        },
        callbacks: BoundaryCallbackContract::RequestScoped {
            interface_types: vec![PackageSchemaTypeRef {
                package_id: "example.types".to_string(),
                stable_schema_key: "callback".to_string(),
                package_schema_type_id: ids[3].clone(),
            }],
            lifetime: BoundaryCallbackLifetime::Stream,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        },
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: true,
        },
    };

    let mut roots = Vec::new();
    collect_operation_refs(&operation, &mut roots);
    assert_eq!(
        roots.into_iter().collect::<BTreeSet<_>>(),
        ["parameter", "return", "stream", "callback"]
            .into_iter()
            .zip(ids)
            .map(|(key, id)| ("example.types".to_string(), key.to_string(), id))
            .collect()
    );
}

#[test]
fn manifest_selection_projection_selects_function_and_complete_instance_methods() {
    let package = package_fixture();
    let selected = selection(&["worker", "selected"]);
    let projected =
        project_service_api("example.service", &selected, &package, &BTreeMap::new()).unwrap();

    assert_eq!(
        projected
            .available
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["selected", "worker.run", "worker.stop"]
    );
    assert!(projected.unavailable.is_empty());
    assert_eq!(projected.contract.operations.len(), 3);
    assert!(projected.contract.package_type_requirements.is_empty());
    let operation_paths = projected
        .contract
        .diagnostic_text
        .operations
        .values()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        operation_paths,
        BTreeSet::from(["selected", "worker.run", "worker.stop"])
    );

    let status = |path: &str| {
        &projected
            .visibility
            .functions
            .iter()
            .find(|function| function.public_path == path)
            .unwrap()
            .status
    };
    assert!(matches!(
        status("selected"),
        ServiceApiFunctionStatus::Available {
            service_operation_id: Some(_)
        }
    ));
    assert!(matches!(
        status("packageOnly"),
        ServiceApiFunctionStatus::Available {
            service_operation_id: None
        }
    ));
    assert!(matches!(
        status("worker.helper"),
        ServiceApiFunctionStatus::Available {
            service_operation_id: None
        }
    ));
    assert!(matches!(
        status("blocked"),
        ServiceApiFunctionStatus::Unavailable { reasons }
            if reasons == &vec![
                BoundaryUnavailableReason::AnalysisPending,
                BoundaryUnavailableReason::UnknownEffect,
            ]
    ));
}

#[test]
fn manifest_selection_projection_reports_all_unavailable_callables_and_reasons() {
    let package = package_fixture();
    let selected = selection(&["blockedTwo", "blocked"]);

    let ContractDefinitionError::UnavailableServiceCalls { unavailable } =
        project_service_api("example.service", &selected, &package, &BTreeMap::new()).unwrap_err()
    else {
        panic!("selected unavailable callables must fail as one structured error")
    };
    assert_eq!(
        unavailable.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["blocked", "blockedTwo"]
    );
    assert_eq!(
        unavailable["blocked"],
        vec![
            BoundaryUnavailableReason::AnalysisPending,
            BoundaryUnavailableReason::UnknownEffect,
        ]
    );
    assert_eq!(
        unavailable["blockedTwo"],
        vec![BoundaryUnavailableReason::UnsupportedBoundaryType]
    );
}

#[test]
fn manifest_selection_projection_allows_stable_zero_operation_contract() {
    let package = package_fixture();

    let first = project_service_api("example.empty", &[], &package, &BTreeMap::new()).unwrap();
    let second = project_service_api("example.empty", &[], &package, &BTreeMap::new()).unwrap();
    assert!(first.contract.operations.is_empty());
    assert!(first.contract.package_type_requirements.is_empty());
    assert!(first.available.is_empty());
    assert!(first.unavailable.is_empty());
    assert_eq!(
        first.contract.service_protocol_identity,
        second.contract.service_protocol_identity
    );
    assert!(first.visibility.functions.iter().all(|function| {
        !matches!(
            &function.status,
            ServiceApiFunctionStatus::Available {
                service_operation_id: Some(_)
            }
        )
    }));
}

fn package_fixture() -> PackageArtifact {
    let paths = [
        "selected",
        "packageOnly",
        "blocked",
        "blockedTwo",
        "worker.run",
        "worker.stop",
        "worker.helper",
    ];
    let mut public_symbols = paths
        .into_iter()
        .map(|path| {
            (
                path.to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: package_callable_id(path),
                    signature: PackageCallableSignature {
                        type_params: Vec::new(),
                        parameters: Vec::new(),
                        return_type: PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("void"),
                        },
                        may_suspend: false,
                    },
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    public_symbols.insert(
        "worker".to_string(),
        PackageLocalAbiSymbol::PublicInstance {
            instance_id: "worker".to_string(),
            declared_receiver_type: TypeRefIr::builtin("Worker"),
            interfaces: vec![TypeRefIr::builtin("WorkerApi")],
            methods: BTreeMap::from([
                ("run".to_string(), package_callable_id("worker.run")),
                ("stop".to_string(), package_callable_id("worker.stop")),
            ]),
        },
    );

    let mut boundary_projections = paths
        .into_iter()
        .map(|path| {
            (
                package_callable_id(path),
                BoundaryCallableProjection::Available {
                    operation_contract: operation(ContractTypeRef::builtin("void")),
                    implementation_requirements:
                        skiff_artifact_model::BoundaryImplementationRequirements {
                            config: Vec::new(),
                            state: Vec::new(),
                            native_capabilities: Vec::new(),
                            complete_may_effects: skiff_artifact_model::CallableMayEffects {
                                escapes_caller_value: false,
                                requires_same_heap_identity: false,
                                invokes_unknown_target: false,
                                may_pending: false,
                                pending_effect_categories: Vec::new(),
                                inout_path_effects: Vec::new(),
},
                            provenance: skiff_artifact_model::CallableProvenanceSummary::Analyzed {
                                return_origins: Vec::new(),
                                direct_return_origins: Vec::new(),
                                throw_origins: Vec::new(),
                                escape_lanes: Vec::new(),
                            },
                        },
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    boundary_projections.insert(
        package_callable_id("packageOnly"),
        BoundaryCallableProjection::Available {
            operation_contract: operation(ContractTypeRef::package_schema(
                "example.types",
                "Unused",
                PackageSchemaTypeId::new("type:unused"),
            )),
            implementation_requirements: skiff_artifact_model::BoundaryImplementationRequirements {
                config: Vec::new(),
                state: Vec::new(),
                native_capabilities: Vec::new(),
                complete_may_effects: skiff_artifact_model::CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: false,
                    pending_effect_categories: Vec::new(),
                    inout_path_effects: Vec::new(),
},
                provenance: skiff_artifact_model::CallableProvenanceSummary::Analyzed {
                    return_origins: Vec::new(),
                    direct_return_origins: Vec::new(),
                    throw_origins: Vec::new(),
                    escape_lanes: Vec::new(),
                },
            },
        },
    );
    boundary_projections.insert(
        package_callable_id("blocked"),
        BoundaryCallableProjection::Unavailable {
            reasons: vec![
                BoundaryUnavailableReason::AnalysisPending,
                BoundaryUnavailableReason::UnknownEffect,
            ],
        },
    );
    boundary_projections.insert(
        package_callable_id("blockedTwo"),
        BoundaryCallableProjection::Unavailable {
            reasons: vec![BoundaryUnavailableReason::UnsupportedBoundaryType],
        },
    );

    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.package".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("build"),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            public_symbols,
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.package".to_string(),
            package_schema_index_identity: PackageSchemaIndexIdentity::new("index"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections,
        service_call_refs: Vec::new(),
        bytecode: None,
    }
}

fn selection(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_string()).collect()
}

fn package_callable_id(path: &str) -> PackageCallableId {
    PackageCallableId::new(format!("pkg-callable:example.package:{path}"))
}

fn operation(return_type: ContractTypeRef) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: return_type,
            value_plan: value_plan(BoundaryValueOwner::Provider),
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

fn value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}
