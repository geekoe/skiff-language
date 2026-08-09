use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, package_artifact_build_identity,
    package_artifact_build_identity_projection, package_artifact_local_abi_identity,
    package_artifact_local_abi_identity_projection, service_protocol_identity,
    validate_package_artifact_identities,
};
use skiff_artifact_model::{
    config_shape_from_package_requirements, ActorMethodIdentity, BoundaryCallableProjection,
    BoundaryOperationDescriptor, BoundaryUnavailableReason, CallableEffectSummary,
    CallableProvenanceSummary, ContractDiagnosticText, PackageLocalAbiSymbol, ServiceContract,
    ServiceProtocolIdentity, ValueProjectionPath, ValueProvenance, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};
use skiff_compiler_core::{implementation_package_callable_id, ImplementationCallableKind};

use super::fixtures::{
    callable_id, exact_typed_signature, project_actor_fixture, project_fixture,
    project_fixture_with_runtime_requirements, project_fixture_without_local_conformance_facts,
    runtime_requirements, SignatureSet,
};

#[test]
fn package_actor_declarations_project_into_local_abi_and_links() {
    let artifact = project_actor_fixture().unwrap();
    validate_package_artifact_identities(&artifact).unwrap();
    let PackageLocalAbiSymbol::Type {
        actor: public_actor,
        ..
    } = &artifact.package_local_abi.public_symbols["ThreadActor"]
    else {
        panic!("public actor type must remain a typed package-local declaration");
    };
    let public_actor = public_actor
        .as_ref()
        .expect("public actor type must carry actor metadata, not a plain record");
    assert_eq!(public_actor.abi.actor_name, "ThreadActor");
    assert_eq!(public_actor.abi.key_field, "id");
    assert_eq!(
        public_actor.abi.actor_id_type,
        skiff_artifact_model::TypeRefIr::builtin("u64")
    );
    assert_eq!(
        public_actor
            .abi
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["id", "label"]
    );
    let create = public_actor
        .abi
        .create
        .as_ref()
        .expect("actor create signature must project");
    assert_eq!(create.parameters.len(), 1);
    assert_eq!(create.parameters[0].name, "label");
    assert_eq!(public_actor.abi.public_methods.len(), 1);
    assert_eq!(public_actor.abi.public_methods[0].name, "read");
    assert_eq!(
        public_actor.abi.public_methods[0].return_type,
        skiff_artifact_model::TypeRefIr::builtin("string")
    );
    assert_eq!(
        public_actor.actor_abi_identity.as_str(),
        "skiff-actor-abi-v1:sha256:thread-actor"
    );
    assert_eq!(
        artifact.implementation_links.types["ThreadActor"]
            .actor
            .as_ref(),
        Some(public_actor)
    );

    let PackageLocalAbiSymbol::Type {
        actor: implementation_actor,
        ..
    } = &artifact.package_local_abi.implementation_symbols["thread_actor.ThreadActor"]
    else {
        panic!("implementation actor type must remain a typed package-local declaration");
    };
    let implementation_actor = implementation_actor
        .as_ref()
        .expect("implementation actor type must carry actor metadata");
    assert_eq!(implementation_actor.abi.key_field, "id");
    assert_eq!(
        artifact.implementation_links.types["thread_actor.ThreadActor"]
            .actor
            .as_ref(),
        Some(implementation_actor)
    );

    let wire = serde_json::to_string(&artifact).unwrap();
    assert!(
        wire.contains("\"actor\":{"),
        "artifact wire must carry actor metadata"
    );
    assert!(wire.contains("\"actorName\":\"ThreadActor\""));
    assert!(wire.contains("\"keyField\":\"id\""));

    assert_eq!(artifact.actor_implementations.len(), 1);
    let implementation = &artifact.actor_implementations[0];
    assert_eq!(implementation.actor.module_path, "thread_actor");
    assert_eq!(implementation.actor.symbol, "ThreadActor");
    assert_eq!(
        implementation.methods[&ActorMethodIdentity::new("skiff-actor-method-v1:sha256:read")],
        implementation_package_callable_id(
            "example.actor.pkg",
            "thread_actor",
            "thread_actor.ThreadActor.read",
            ImplementationCallableKind::ImplMethod,
        )
        .unwrap()
    );
    let create = implementation
        .create
        .as_ref()
        .expect("actor create must have independent implementation authority");
    assert_eq!(
        create.method_identity,
        ActorMethodIdentity::new("skiff-actor-method-v1:sha256:create")
    );
    assert_eq!(
        create.package_callable_id,
        implementation_package_callable_id(
            "example.actor.pkg",
            "thread_actor",
            "thread_actor.ThreadActor.create",
            ImplementationCallableKind::ImplMethod,
        )
        .unwrap()
    );
    assert!(artifact.bytecode.is_none());
}

#[test]
fn package_api_callables_have_exact_local_abi_and_boundary_coverage() {
    let artifact = project_fixture(SignatureSet::Complete).unwrap();
    validate_package_artifact_identities(&artifact).unwrap();
    assert_eq!(artifact.schema_version, PACKAGE_ARTIFACT_SCHEMA_VERSION);
    assert_eq!(artifact.schema_version, "skiff-package-artifact-v12");
    assert!(artifact
        .package_build_id
        .as_str()
        .starts_with("skiff-package-build-v11:sha256:"));
    assert_eq!(
        serde_json::to_value(package_artifact_build_identity_projection(&artifact).unwrap())
            .unwrap()["schema"],
        "skiff-package-artifact-build-identity-v10"
    );
    assert_eq!(
        serde_json::to_value(package_artifact_local_abi_identity_projection(&artifact).unwrap())
            .unwrap()["schema"],
        "skiff-package-artifact-local-abi-identity-v6"
    );

    let callable_paths = artifact
        .package_local_abi
        .public_symbols
        .iter()
        .filter_map(|(path, symbol)| {
            matches!(symbol, PackageLocalAbiSymbol::Callable { .. }).then_some(path.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        callable_paths,
        vec!["mutate", "run", "runAlias", "worker.handle"]
    );
    assert!(matches!(
        artifact.package_local_abi.public_symbols["Worker"],
        PackageLocalAbiSymbol::Type { .. }
    ));
    assert!(matches!(
        artifact.package_local_abi.public_symbols["VERSION"],
        PackageLocalAbiSymbol::Constant { .. }
    ));
    assert_eq!(artifact.callable_links.len(), 5);
    assert_eq!(artifact.callable_semantic_facts.len(), 5);
    assert_eq!(artifact.boundary_projections.len(), 4);
    assert_eq!(artifact.package_requirements.len(), 1);
    assert_eq!(artifact.contract_requirements.len(), 1);
    assert_eq!(artifact.service_requirements.len(), 1);
    assert_eq!(artifact.service_call_refs.len(), 1);
    assert_eq!(artifact.service_call_refs[0].service_requirement_slot, 3);
    let config_shape =
        config_shape_from_package_requirements(&artifact.runtime_requirements.config).unwrap();
    assert_eq!(config_shape.entries.len(), 1);
    assert_eq!(config_shape.entries[0].path, "app.token");

    let PackageLocalAbiSymbol::PublicInstance { methods, .. } =
        &artifact.package_local_abi.public_symbols["worker"]
    else {
        panic!("public instance must remain in Local ABI");
    };
    assert_eq!(methods.len(), 1);
    let mutate_id = callable_id(&artifact, "mutate");
    assert!(
        matches!(
            &artifact.boundary_projections[&mutate_id],
            BoundaryCallableProjection::Available { .. }
        ),
        "ordinary aggregate mutation must project boundary-available under R-134"
    );
    assert!(artifact
        .implementation_links
        .functions
        .contains_key("mutate"));
    assert!(artifact.implementation_links.types.contains_key("Worker"));
    assert!(artifact
        .implementation_links
        .constants
        .contains_key("VERSION"));
    assert_eq!(
        artifact.implementation_links.constants["worker"].const_index,
        0
    );
    let worker_handle_id = callable_id(&artifact, "worker.handle");
    assert_eq!(
        artifact.callable_links[&worker_handle_id]
            .target
            .executable_index,
        2
    );
    assert!(artifact.callable_links.contains_key(&mutate_id));
    let run_id = callable_id(&artifact, "run");
    let run_alias_id = callable_id(&artifact, "runAlias");
    assert_ne!(run_id, run_alias_id);
    assert_eq!(
        artifact.callable_links[&run_id].target.file_ref,
        artifact.callable_links[&run_alias_id].target.file_ref
    );
    assert_eq!(
        artifact.callable_links[&run_id].target.executable_index,
        artifact.callable_links[&run_alias_id]
            .target
            .executable_index
    );
    assert_eq!(artifact.local_interface_conformances.len(), 1);
    let conformance = &artifact.local_interface_conformances[0];
    assert!(conformance.type_parameters.is_empty());
    assert_eq!(
        conformance.receiver,
        skiff_artifact_model::TypeRefIr::PackageSymbol {
            symbol: skiff_artifact_model::PackageSymbolRef {
                package: skiff_artifact_model::PackageRefIr::PackageId {
                    package_id: "example.pkg".to_string(),
                },
                symbol_path: "api.Worker".to_string(),
                abi_expectation: None,
            },
        }
    );
    assert_eq!(
        serde_json::from_str::<skiff_artifact_model::TypeRefIr>(
            &conformance.interface.interface_abi_id
        )
        .unwrap(),
        skiff_artifact_model::TypeRefIr::PackageSymbol {
            symbol: skiff_artifact_model::PackageSymbolRef {
                package: skiff_artifact_model::PackageRefIr::PackageId {
                    package_id: "example.pkg".to_string(),
                },
                symbol_path: "api.WorkerInterface".to_string(),
                abi_expectation: None,
            },
        }
    );
    assert_eq!(
        conformance.methods,
        vec![implementation_package_callable_id(
            "example.pkg",
            "api",
            "api.Worker.handle",
            ImplementationCallableKind::ImplMethod,
        )
        .unwrap()]
    );
    assert_ne!(conformance.methods[0], worker_handle_id);
    assert!(artifact.bytecode.is_none());

    let wire = serde_json::to_string(&artifact).unwrap();
    for forbidden in [
        "throwTypes",
        "\"errors\"",
        "publicationAbi",
        "packageUnit",
        "serviceUnit",
        "providerBuildId",
        "deploymentRevision",
        "route",
        "operationAbiId",
        "methodAbiId",
        "serviceCallRoots",
    ] {
        assert!(!wire.contains(forbidden), "forbidden field {forbidden}");
    }
}

#[test]
fn package_implementation_projection_includes_exact_impl_method_callable() {
    let artifact = project_fixture(SignatureSet::Complete).unwrap();
    let PackageLocalAbiSymbol::Callable {
        callable_id,
        signature,
    } = &artifact.package_local_abi.implementation_symbols["api.Worker.handle"]
    else {
        panic!("implementation receiver method must be projected as a package callable");
    };
    assert_eq!(
        callable_id.as_str(),
        "pkg-callable:example.pkg:top-level:api.Worker.handle"
    );
    assert_eq!(signature.parameters.len(), 2);
    assert_eq!(signature.parameters[0].name, "self");
    assert_eq!(
        signature.parameters[0].ty,
        skiff_artifact_model::PackageTypeRef::Local {
            local_type: skiff_artifact_model::TypeRefIr::PackageSymbol {
                symbol: skiff_artifact_model::PackageSymbolRef {
                    package: skiff_artifact_model::PackageRefIr::PackageId {
                        package_id: "example.pkg".to_string()
                    },
                    symbol_path: "api.Worker".to_string(),
                    abi_expectation: None,
                }
            }
        }
    );
    assert_eq!(signature.parameters[1].name, "value");
    let link = &artifact.callable_links[callable_id];
    assert_eq!(link.target.executable_index, 2);
    assert_eq!(
        link.target.callable_kind,
        skiff_artifact_model::OperationCallableKind::ImplMethod
    );
}

#[test]
fn ordinary_and_service_package_projection_share_artifact_and_local_abi() {
    let ordinary_package = project_fixture(SignatureSet::Complete).unwrap();
    // A service root uses this exact same Package producer. There is no
    // service-manifest or source-role input at this projection boundary.
    let service_package = project_fixture(SignatureSet::Complete).unwrap();
    assert_eq!(service_package, ordinary_package);
    assert_eq!(
        service_package.package_local_abi.local_abi_identity,
        ordinary_package.package_local_abi.local_abi_identity
    );
}

#[test]
fn exact_typed_signatures_reach_local_abi_and_public_instance_receiver_is_trimmed() {
    let artifact = project_fixture(SignatureSet::ExactTyped).unwrap();
    assert!(artifact
        .package_local_abi
        .local_abi_identity
        .as_str()
        .starts_with("skiff-package-local-abi-v7:sha256:"));
    let PackageLocalAbiSymbol::Callable {
        signature: run_signature,
        ..
    } = &artifact.package_local_abi.public_symbols["run"]
    else {
        panic!("run must be a Local ABI callable");
    };
    assert_eq!(run_signature, &exact_typed_signature());

    let PackageLocalAbiSymbol::Callable {
        signature: instance_signature,
        ..
    } = &artifact.package_local_abi.public_symbols["worker.handle"]
    else {
        panic!("public-instance operation must be a Local ABI callable");
    };
    assert_eq!(instance_signature.parameters.len(), 1);
    assert_eq!(instance_signature.parameters[0].name, "value");
}

#[test]
fn stale_package_artifact_schema_and_identity_prefixes_fail_closed() {
    let base = project_fixture(SignatureSet::Complete).unwrap();

    let mut stale_schema = base.clone();
    stale_schema.schema_version = "skiff-package-artifact-v8".to_string();
    assert!(validate_package_artifact_identities(&stale_schema).is_err());

    let mut stale_local = base.clone();
    stale_local.package_local_abi.local_abi_identity =
        skiff_artifact_model::PackageLocalAbiIdentity::new(
            stale_local
                .package_local_abi
                .local_abi_identity
                .as_str()
                .replacen(
                    "skiff-package-local-abi-v7:sha256",
                    "skiff-package-local-abi-v6:sha256",
                    1,
                ),
        );
    assert!(validate_package_artifact_identities(&stale_local).is_err());

    let mut stale_build = base;
    stale_build.package_build_id =
        skiff_artifact_model::PackageBuildId::new(stale_build.package_build_id.as_str().replacen(
            "skiff-package-build-v11:sha256",
            "skiff-package-build-v10:sha256",
            1,
        ));
    assert!(validate_package_artifact_identities(&stale_build).is_err());
}

#[test]
fn canonical_projection_rejects_invalid_or_duplicate_config_requirements() {
    let mut invalid_type = runtime_requirements();
    invalid_type.config[0].access = skiff_artifact_model::PackageConfigAccess::Required {
        value_type: "bytes".to_string(),
    };
    let error = project_fixture_with_runtime_requirements(SignatureSet::Complete, invalid_type)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("canonical runtime config requirements are invalid"),
        "unexpected error: {error}"
    );
    assert!(error.contains("app.token"), "unexpected error: {error}");

    let mut duplicate = runtime_requirements();
    duplicate.config.push(duplicate.config[0].clone());
    let error = project_fixture_with_runtime_requirements(SignatureSet::Complete, duplicate)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("declared more than once"),
        "unexpected error: {error}"
    );
}

#[test]
fn missing_signature_is_not_reconstructed_from_executable_ir() {
    let missing = project_fixture(SignatureSet::Missing)
        .unwrap_err()
        .to_string();
    assert!(missing.contains("missing="), "unexpected error: {missing}");
    assert!(missing.contains("mutate"), "unexpected error: {missing}");
}

#[test]
fn missing_typed_local_conformance_facts_fail_closed() {
    let error = project_fixture_without_local_conformance_facts()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("must exactly cover File IR implements declarations"),
        "unexpected error: {error}"
    );
    assert!(error.contains("missing="), "unexpected error: {error}");
}

#[test]
fn canonical_signature_set_rejects_extra_and_target_mismatched_entries() {
    let extra = project_fixture(SignatureSet::Extra)
        .unwrap_err()
        .to_string();
    assert!(extra.contains("extra="), "unexpected error: {extra}");
    assert!(extra.contains("internal"), "unexpected error: {extra}");

    let target_mismatch = project_fixture(SignatureSet::TargetMismatch)
        .unwrap_err()
        .to_string();
    assert!(
        target_mismatch.contains("api#0") && target_mismatch.contains("api#9"),
        "unexpected error: {target_mismatch}"
    );
}

#[test]
fn implementation_requirements_change_build_not_local_abi_or_operation_contract() {
    let first = project_fixture(SignatureSet::Complete).unwrap();
    let mut changed_requirements = runtime_requirements();
    changed_requirements.config[0].path = "app.changed-token".to_string();
    let second =
        project_fixture_with_runtime_requirements(SignatureSet::Complete, changed_requirements)
            .unwrap();
    assert_eq!(
        first.package_local_abi.local_abi_identity,
        second.package_local_abi.local_abi_identity
    );
    assert_ne!(first.package_build_id, second.package_build_id);

    let first_id = callable_id(&first, "run");
    let second_id = callable_id(&second, "run");
    let BoundaryCallableProjection::Available {
        operation_contract: first_contract,
        ..
    } = &first.boundary_projections[&first_id]
    else {
        panic!("run must be available");
    };
    let BoundaryCallableProjection::Available {
        operation_contract: second_contract,
        ..
    } = &second.boundary_projections[&second_id]
    else {
        panic!("run must be available");
    };
    assert_eq!(first_contract, second_contract);
}

#[test]
fn implementation_throw_facts_change_build_but_not_local_abi_or_service_protocol() {
    let base = project_fixture(SignatureSet::Complete).unwrap();
    let callable_id = callable_id(&base, "run");
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = &base.boundary_projections[&callable_id]
    else {
        panic!("run must be available")
    };
    let operation_id = contract_operation_id("example.service", "1.0.0", "run").unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: "example.service".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: std::collections::BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id,
                stable_key: "run".to_string(),
                contract: operation_contract.clone(),
            },
        )]),
        public_instances: std::collections::BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: "service".to_string(),
            operations: std::collections::BTreeMap::new(),
            types: std::collections::BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    let baseline_protocol = service_protocol_identity(&contract).unwrap();
    let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
    let baseline_build = package_artifact_build_identity(&base).unwrap();

    let mut changed = base.clone();
    let facts = changed
        .callable_semantic_facts
        .get_mut(&callable_id)
        .expect("run semantic facts");
    let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
        panic!("fixture effects must be analyzed")
    };
    effects.requires_same_heap_identity = true;
    let CallableProvenanceSummary::Analyzed { throw_origins, .. } = &mut facts.provenance else {
        panic!("fixture provenance must be analyzed")
    };
    *throw_origins = vec![ValueProvenance::CallerParameter { index: 0 }];
    changed.boundary_projections.insert(
        callable_id,
        BoundaryCallableProjection::Unavailable {
            reasons: vec![BoundaryUnavailableReason::RequiresSameHeapIdentity],
        },
    );

    assert_eq!(
        package_artifact_local_abi_identity(&changed).unwrap(),
        baseline_local
    );
    assert_ne!(
        package_artifact_build_identity(&changed).unwrap(),
        baseline_build,
        "open-error provenance remains an implementation/build fact"
    );
    assert_eq!(
        service_protocol_identity(&contract).unwrap(),
        baseline_protocol
    );
}

#[test]
fn caller_projection_path_changes_build_identity_but_not_local_abi() {
    let base = project_fixture(SignatureSet::Complete).unwrap();
    let base_local = package_artifact_local_abi_identity(&base).unwrap();
    let base_build = package_artifact_build_identity(&base).unwrap();

    let mut state_projection = base.clone();
    let state_origin = ValueProvenance::CallerParameterProjection {
        index: 0,
        path: ValueProjectionPath::field("state").unwrap(),
    };
    let facts = state_projection
        .callable_semantic_facts
        .values_mut()
        .next()
        .expect("fixture callable facts");
    let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
        panic!("fixture effects must be analyzed")
    };
    effects.requires_same_heap_identity = true;
    facts.provenance = CallableProvenanceSummary::Analyzed {
        return_origins: vec![state_origin.clone()],
        direct_return_origins: vec![state_origin],
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let callable_id = state_projection
        .callable_semantic_facts
        .keys()
        .next()
        .expect("fixture callable id")
        .clone();
    state_projection.boundary_projections.insert(
        callable_id,
        BoundaryCallableProjection::Unavailable {
            reasons: vec![BoundaryUnavailableReason::RequiresSameHeapIdentity],
        },
    );

    let mut status_projection = state_projection.clone();
    let CallableProvenanceSummary::Analyzed {
        return_origins,
        direct_return_origins,
        ..
    } = &mut status_projection
        .callable_semantic_facts
        .values_mut()
        .next()
        .expect("fixture callable facts")
        .provenance
    else {
        panic!("fixture provenance must be analyzed")
    };
    let status_origin = ValueProvenance::CallerParameterProjection {
        index: 0,
        path: ValueProjectionPath::field("status").unwrap(),
    };
    *return_origins = vec![status_origin.clone()];
    *direct_return_origins = vec![status_origin.clone()];

    let mut direct_only_projection = state_projection.clone();
    let CallableProvenanceSummary::Analyzed {
        direct_return_origins,
        ..
    } = &mut direct_only_projection
        .callable_semantic_facts
        .values_mut()
        .next()
        .expect("fixture callable facts")
        .provenance
    else {
        panic!("fixture provenance must be analyzed")
    };
    *direct_return_origins = vec![status_origin];

    for changed in [
        &state_projection,
        &status_projection,
        &direct_only_projection,
    ] {
        assert_eq!(
            package_artifact_local_abi_identity(changed).unwrap(),
            base_local
        );
        assert_ne!(
            package_artifact_build_identity(changed).unwrap(),
            base_build
        );
    }
    assert_ne!(
        package_artifact_build_identity(&state_projection).unwrap(),
        package_artifact_build_identity(&status_projection).unwrap()
    );
    assert_ne!(
        package_artifact_build_identity(&state_projection).unwrap(),
        package_artifact_build_identity(&direct_only_projection).unwrap(),
        "directReturnOrigins is part of package implementation identity"
    );
}
