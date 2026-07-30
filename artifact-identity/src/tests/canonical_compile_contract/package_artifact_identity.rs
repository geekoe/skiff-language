use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryValueLifetime, BoundaryValuePlan, CallableEffectSummary,
    PackageLocalAbiSymbol, PackageRuntimeCapabilityRequirement,
};

use super::*;

#[test]
fn package_artifact_assign_validate_and_golden_identities() {
    let artifact = package_artifact_fixture();
    validate_package_artifact_identities(&artifact).unwrap();
    assert_eq!(
        artifact.package_build_id.as_str(),
        "skiff-package-build-v4:sha256:8036e864ca8706c7006598924186e58cb70918909321b7aeb86b8604e28b25c6"
    );
    assert_eq!(
        artifact.package_local_abi.local_abi_identity.as_str(),
        "skiff-package-local-abi-v3:sha256:222c6357adf8a2dddc349d8fd771379aab6128a081545d4a1263a61daa3b9411"
    );
}

#[test]
fn human_package_and_dependency_version_labels_do_not_change_artifact_identities() {
    let base = package_artifact_fixture();
    let mut relabeled = base.clone();
    relabeled.package_version = "99.0.0".to_string();
    relabeled.package_requirements[0].exact_version = "88.0.0".to_string();
    relabeled.contract_requirements[0].contract_version = "77.0.0".to_string();
    relabeled.service_requirements[0]
        .contract_requirement
        .contract_version = "77.0.0".to_string();

    assert_eq!(
        package_artifact_local_abi_identity(&base).unwrap(),
        package_artifact_local_abi_identity(&relabeled).unwrap()
    );
    assert_eq!(
        package_artifact_build_identity(&base).unwrap(),
        package_artifact_build_identity(&relabeled).unwrap()
    );
    assert_ne!(base.package_version, relabeled.package_version);
}

#[test]
fn available_operation_contract_and_implementation_requirements_are_build_only() {
    let base = package_artifact_fixture();
    let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
    let baseline_build = package_artifact_build_identity(&base).unwrap();

    let projection = package_artifact_build_identity_projection(&base).unwrap();
    let wire = serde_json::to_value(projection).unwrap();
    let available = wire["boundaryProjections"]
        .as_object()
        .and_then(|projections| projections.values().next())
        .expect("available boundary projection");
    assert!(available.get("operationContract").is_some());
    assert!(available.get("implementationRequirements").is_some());
    for forbidden in ["descriptor", "operationId", "stableKey"] {
        assert!(
            available.get(forbidden).is_none(),
            "package build projection must exclude {forbidden}"
        );
    }

    let mut changed_contract = base.clone();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = changed_contract
        .boundary_projections
        .values_mut()
        .next()
        .unwrap()
    else {
        panic!("fixture available")
    };
    operation_contract.effect_guarantee.detached_return = false;
    assert_eq!(
        package_artifact_local_abi_identity(&changed_contract).unwrap(),
        baseline_local
    );
    assert_ne!(
        package_artifact_build_identity(&changed_contract).unwrap(),
        baseline_build
    );

    let mut changed_requirements = base.clone();
    let BoundaryCallableProjection::Available {
        implementation_requirements,
        ..
    } = changed_requirements
        .boundary_projections
        .values_mut()
        .next()
        .unwrap()
    else {
        panic!("fixture available")
    };
    implementation_requirements
        .runtime_capabilities
        .push("stream".to_string());
    assert_eq!(
        package_artifact_local_abi_identity(&changed_requirements).unwrap(),
        baseline_local
    );
    assert_ne!(
        package_artifact_build_identity(&changed_requirements).unwrap(),
        baseline_build
    );
}

#[test]
fn local_abi_and_build_identity_include_package_public_surface() {
    let base = package_artifact_fixture();
    let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
    let baseline_build = package_artifact_build_identity(&base).unwrap();
    let mut changed = base.clone();
    let PackageLocalAbiSymbol::Callable { signature, .. } = changed
        .package_local_abi
        .public_symbols
        .get_mut("handle")
        .unwrap()
    else {
        panic!("fixture callable")
    };
    signature.may_suspend = false;
    assert_ne!(
        package_artifact_local_abi_identity(&changed).unwrap(),
        baseline_local
    );
    assert_ne!(
        package_artifact_build_identity(&changed).unwrap(),
        baseline_build
    );
}

#[test]
fn build_identity_includes_every_canonical_package_artifact_fact() {
    let base = package_artifact_fixture();
    let baseline = package_artifact_build_identity(&base).unwrap();

    let mutations: Vec<fn(&mut PackageArtifact)> = vec![
        |artifact| artifact.files[0].file_ir_identity.push('1'),
        |artifact| artifact.static_resources[0].sha256.push('1'),
        |artifact| {
            artifact
                .implementation_links
                .functions
                .get_mut("handle")
                .unwrap()
                .executable_index = 7
        },
        |artifact| {
            artifact
                .callable_links
                .values_mut()
                .next()
                .unwrap()
                .target
                .executable_index = 7
        },
        |artifact| {
            artifact.runtime_requirements.runtime_capabilities.push(
                PackageRuntimeCapabilityRequirement {
                    capability: "stream".to_string(),
                    required_version: "1".to_string(),
                },
            )
        },
        |artifact| {
            let facts = artifact
                .callable_semantic_facts
                .values_mut()
                .next()
                .unwrap();
            let CallableEffectSummary::Analyzed { effects } = &mut facts.effects else {
                panic!("fixture analyzed effects")
            };
            effects.invokes_unknown_target = true;
        },
        |artifact| {
            let BoundaryCallableProjection::Available {
                implementation_requirements,
                ..
            } = artifact.boundary_projections.values_mut().next().unwrap()
            else {
                panic!("fixture available")
            };
            implementation_requirements.provenance = CallableProvenanceSummary::Unknown {
                reason: skiff_artifact_model::CallableProvenanceUnknownReason::UnknownCallTarget,
            };
        },
        |artifact| {
            let BoundaryCallableProjection::Available {
                operation_contract, ..
            } = artifact.boundary_projections.values_mut().next().unwrap()
            else {
                panic!("fixture available")
            };
            let BoundaryValuePlan::Linkable { lifetime, .. } =
                &mut operation_contract.return_value.value_plan
            else {
                panic!("fixture linkable")
            };
            *lifetime = BoundaryValueLifetime::Request;
        },
        |artifact| {
            let operation = contract_operation_id("example.echo", "1.0.0", "second").unwrap();
            artifact.service_requirements[0]
                .used_operations
                .insert(operation.clone());
            artifact.service_call_refs.push(ServiceCallRef {
                service_requirement_slot: 0,
                contract_operation_id: operation,
                expected_protocol_identity: artifact.contract_requirements[0]
                    .expected_protocol_identity
                    .clone(),
            });
        },
    ];

    for mutate in mutations {
        let mut changed = base.clone();
        mutate(&mut changed);
        assert_ne!(package_artifact_build_identity(&changed).unwrap(), baseline);
    }
}

#[test]
fn storage_provenance_declared_ids_and_map_order_are_excluded() {
    let base = package_artifact_fixture();
    let baseline_local = package_artifact_local_abi_identity(&base).unwrap();
    let baseline_build = package_artifact_build_identity(&base).unwrap();

    let mut storage = base.clone();
    storage.files[0].artifact_path = Some("different/storage/path.json".to_string());
    storage.files[0].source_ast_hash = Some("different-source-provenance".to_string());
    storage.static_resources[0].artifact_path = Some("different/resource/path".to_string());
    storage.package_build_id = PackageBuildId::new("declared-build-is-not-a-preimage");
    storage.package_local_abi.local_abi_identity =
        PackageLocalAbiIdentity::new("declared-abi-is-not-a-preimage");
    assert_eq!(
        package_artifact_local_abi_identity(&storage).unwrap(),
        baseline_local
    );
    assert_eq!(
        package_artifact_build_identity(&storage).unwrap(),
        baseline_build
    );

    let mut reordered = base.clone();
    let mut facts = reordered
        .callable_semantic_facts
        .into_iter()
        .collect::<Vec<_>>();
    facts.reverse();
    reordered.callable_semantic_facts = facts.into_iter().collect();
    assert_eq!(
        package_artifact_build_identity(&reordered).unwrap(),
        baseline_build
    );
}

#[test]
fn declared_package_identities_fail_closed() {
    let mut artifact = package_artifact_fixture();
    artifact.schema_version = "skiff-package-artifact-v1".to_string();
    assert!(matches!(
        validate_package_artifact_identities(&artifact),
        Err(ArtifactIdentityError::InvalidPackageArtifact { .. })
    ));

    let mut artifact = package_artifact_fixture();
    artifact.package_local_abi.local_abi_identity = PackageLocalAbiIdentity::new("tampered");
    assert!(matches!(
        validate_package_artifact_identities(&artifact),
        Err(ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch { .. })
    ));

    let mut artifact = package_artifact_fixture();
    artifact.package_build_id = PackageBuildId::new("tampered");
    assert!(matches!(
        validate_package_artifact_identities(&artifact),
        Err(ArtifactIdentityError::PackageArtifactBuildIdentityMismatch { .. })
    ));
}
