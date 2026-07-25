use skiff_artifact_identity::{
    assign_runtime_assembly_identity, assign_service_deployment_identity,
    runtime_assembly_identity, runtime_assembly_identity_projection, service_deployment_identity,
    service_deployment_identity_projection, validate_runtime_assembly_identity,
    validate_runtime_assembly_surface, validate_service_deployment_identity,
    validate_service_deployment_ref,
};
use skiff_artifact_model::{
    ConfigLiteralBinding, ContractOperationId, DeploymentIngressBinding,
    DeploymentOperationBinding, IngressProtocol, IngressSelector, MetadataValue,
    PackageArtifactRef, PackageBinding, PackageBuildId, PackageLocalAbiIdentity,
    PackageRequirementKey, ResolvedServiceBinding, ResourceBinding, RuntimeAssembly,
    RuntimeCapabilityBinding, SecretRefBinding, ServiceDeployment, ServiceRequirementKey,
    ServiceSelectorBinding, StateBinding, StateBindingKind,
};

use crate::fixtures::{
    empty_runtime_assembly_fixture, runtime_assembly_fixture, service_deployment_fixture,
};

fn additional_package() -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: "example.dependency".to_string(),
        package_version: "2.0.0".to_string(),
        package_build_id: PackageBuildId::new("dependency-build"),
        package_local_abi_identity: PackageLocalAbiIdentity::new("dependency-abi"),
    }
}

fn rich_deployment() -> ServiceDeployment {
    let mut deployment = service_deployment_fixture().expect("deployment fixture");
    let dependency = additional_package();
    deployment
        .operation_bindings
        .push(DeploymentOperationBinding {
            contract_operation_id: ContractOperationId::new("operation.health"),
            package_callable_id: skiff_artifact_model::PackageCallableId::new("callable.health"),
        });
    deployment.package_bindings.push(PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: deployment.implementation.package_build_id.clone(),
            package_requirement_alias: "dependency".to_string(),
        },
        package: dependency,
    });
    deployment.service_selectors.push(ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: deployment.implementation.package_build_id.clone(),
            service_requirement_slot: 0,
        },
        contract: deployment.contract.clone(),
    });
    deployment.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            host: "example.test".to_string(),
            method: Some("GET".to_string()),
            path: "/health".to_string(),
        },
        contract_operation_id: ContractOperationId::new("operation.health"),
    });
    deployment.config_literals.push(ConfigLiteralBinding {
        path: "message.suffix".to_string(),
        value: MetadataValue::String("!".to_string()),
    });
    deployment.secret_refs.push(SecretRefBinding {
        path: "auth.token".to_string(),
        secret_ref: "vault://echo/token".to_string(),
    });
    deployment.state_bindings.push(StateBinding {
        requirement_key: "messages".to_string(),
        kind: StateBindingKind::Database,
        namespace: "echo-messages".to_string(),
    });
    deployment.resource_bindings.push(ResourceBinding {
        requirement_key: "mailer".to_string(),
        capability: "smtp".to_string(),
        resource_ref: "resource://mailer/default".to_string(),
    });
    deployment
        .runtime_capability_bindings
        .push(RuntimeCapabilityBinding {
            capability: "clock".to_string(),
            version: "1".to_string(),
        });
    assign_service_deployment_identity(&mut deployment).expect("rich deployment identity");
    deployment
}

#[test]
fn strict_wire_rejects_unknown_and_missing_semantic_fields() {
    let deployment = service_deployment_fixture().expect("deployment fixture");
    let mut value = serde_json::to_value(&deployment).expect("serialize deployment");
    value
        .as_object_mut()
        .unwrap()
        .insert("serviceAssembly".to_string(), serde_json::json!({}));
    assert!(serde_json::from_value::<ServiceDeployment>(value).is_err());

    let mut missing = serde_json::to_value(&deployment).expect("serialize deployment");
    missing.as_object_mut().unwrap().remove("operationBindings");
    assert!(serde_json::from_value::<ServiceDeployment>(missing).is_err());

    let mut semantic_ref = serde_json::to_value(&deployment.implementation).unwrap();
    semantic_ref.as_object_mut().unwrap().insert(
        "artifactPath".to_string(),
        serde_json::json!("packages/provider.json"),
    );
    assert!(serde_json::from_value::<PackageArtifactRef>(semantic_ref).is_err());

    let mut selector = serde_json::to_value(&rich_deployment().service_selectors[0]).unwrap();
    selector.as_object_mut().unwrap().insert(
        "deploymentRevision".to_string(),
        serde_json::json!("forbidden"),
    );
    assert!(serde_json::from_value::<ServiceSelectorBinding>(selector).is_err());

    let mut secret = serde_json::to_value(&rich_deployment().secret_refs[0]).unwrap();
    secret
        .as_object_mut()
        .unwrap()
        .insert("resolvedBytes".to_string(), serde_json::json!("forbidden"));
    assert!(serde_json::from_value::<SecretRefBinding>(secret).is_err());

    let assembly = runtime_assembly_fixture().expect("assembly fixture");
    let mut assembly_value = serde_json::to_value(assembly).unwrap();
    assembly_value
        .as_object_mut()
        .unwrap()
        .insert("runtimeReplicaIds".to_string(), serde_json::json!([]));
    assert!(serde_json::from_value::<RuntimeAssembly>(assembly_value).is_err());
}

#[test]
fn deployment_identity_is_order_independent_and_excludes_diagnostics() {
    let deployment = rich_deployment();
    let expected = service_deployment_identity(&deployment).expect("identity");

    let mut reordered = deployment.clone();
    reordered.operation_bindings.reverse();
    reordered.package_bindings.reverse();
    reordered.service_selectors.reverse();
    reordered.ingress.reverse();
    reordered.config_literals.reverse();
    reordered.secret_refs.reverse();
    reordered.state_bindings.reverse();
    reordered.resource_bindings.reverse();
    reordered.runtime_capability_bindings.reverse();
    assert_eq!(
        service_deployment_identity(&reordered).expect("reordered identity"),
        expected
    );

    let mut diagnostic_only = deployment;
    diagnostic_only.diagnostic_text.display_name = "renamed for humans".to_string();
    diagnostic_only
        .diagnostic_text
        .notes
        .insert("source".to_string(), "not semantic".to_string());
    assert_eq!(
        service_deployment_identity(&diagnostic_only).expect("diagnostic identity"),
        expected
    );
}

#[test]
fn absent_and_null_timeout_normalize_to_the_same_policy_and_identity() {
    let deployment = service_deployment_fixture().expect("deployment fixture");
    let mut absent = serde_json::to_value(&deployment).unwrap();
    absent["policy"].as_object_mut().unwrap().remove("timeoutMs");
    let mut null = absent.clone();
    null["policy"]
        .as_object_mut()
        .unwrap()
        .insert("timeoutMs".to_string(), serde_json::Value::Null);

    let absent: ServiceDeployment = serde_json::from_value(absent).unwrap();
    let null: ServiceDeployment = serde_json::from_value(null).unwrap();
    assert_eq!(absent.policy.timeout_ms, None);
    assert_eq!(null.policy.timeout_ms, None);
    assert_eq!(
        service_deployment_identity(&absent).unwrap(),
        service_deployment_identity(&null).unwrap()
    );
}

#[test]
fn deployment_identity_mutation_matrix_covers_every_semantic_category() {
    let deployment = rich_deployment();
    let expected = service_deployment_identity(&deployment).unwrap();
    let cases: Vec<(&str, Box<dyn Fn(&mut ServiceDeployment)>)> = vec![
        (
            "revision",
            Box::new(|value| value.deployment_revision = "revision-2".into()),
        ),
        (
            "implementation build",
            Box::new(|value| {
                let old = value.implementation.package_build_id.clone();
                let new = PackageBuildId::new("package-build-2");
                value.implementation.package_build_id = new.clone();
                for binding in &mut value.package_bindings {
                    if binding.key.caller_package_build_id == old {
                        binding.key.caller_package_build_id = new.clone();
                    }
                }
                for selector in &mut value.service_selectors {
                    if selector.key.caller_package_build_id == old {
                        selector.key.caller_package_build_id = new.clone();
                    }
                }
            }),
        ),
        (
            "operation",
            Box::new(|value| {
                value.operation_bindings[0].package_callable_id = "callable.changed".into();
            }),
        ),
        (
            "package dependency",
            Box::new(|value| {
                value.package_bindings[0].package.package_build_id =
                    PackageBuildId::new("dependency-build-changed")
            }),
        ),
        (
            "service selector",
            Box::new(|value| {
                value.service_selectors[0]
                    .contract
                    .service_protocol_identity = "protocol.changed".into();
            }),
        ),
        (
            "ingress",
            Box::new(|value| value.ingress[0].selector.path = "/echo-v2".into()),
        ),
        (
            "config literal",
            Box::new(|value| {
                value.config_literals[0].value = MetadataValue::String("changed".into());
            }),
        ),
        (
            "secret ref",
            Box::new(|value| value.secret_refs[0].secret_ref = "vault://echo/next".into()),
        ),
        (
            "state",
            Box::new(|value| value.state_bindings[0].namespace = "echo-next".into()),
        ),
        (
            "resource",
            Box::new(|value| value.resource_bindings[0].resource_ref = "resource://next".into()),
        ),
        (
            "capability",
            Box::new(|value| value.runtime_capability_bindings[0].version = "2".into()),
        ),
        (
            "policy",
            Box::new(|value| {
                value.policy.timeout_ms = value.policy.timeout_ms.map(|timeout| timeout + 1);
            }),
        ),
    ];

    for (label, mutate) in cases {
        let mut changed = deployment.clone();
        mutate(&mut changed);
        assert_ne!(
            service_deployment_identity(&changed).unwrap(),
            expected,
            "{label} must enter deployment identity"
        );
    }
}

#[test]
fn human_version_labels_are_preserved_in_records_but_excluded_from_all_identity_preimages() {
    let deployment = rich_deployment();
    let deployment_identity = service_deployment_identity(&deployment).unwrap();
    let mut relabeled_deployment = serde_json::to_value(&deployment).unwrap();
    relabel_human_versions(&mut relabeled_deployment);
    let relabeled_deployment: ServiceDeployment =
        serde_json::from_value(relabeled_deployment).unwrap();
    assert_ne!(
        deployment.contract.contract_version,
        relabeled_deployment.contract.contract_version
    );
    assert_eq!(
        deployment_identity,
        service_deployment_identity(&relabeled_deployment).unwrap()
    );

    let assembly = runtime_assembly_fixture().unwrap();
    let assembly_identity = runtime_assembly_identity(&assembly).unwrap();
    let mut relabeled_assembly = serde_json::to_value(&assembly).unwrap();
    relabel_human_versions(&mut relabeled_assembly);
    let relabeled_assembly: RuntimeAssembly = serde_json::from_value(relabeled_assembly).unwrap();
    assert_ne!(
        assembly.resolved_packages[0].package_version,
        relabeled_assembly.resolved_packages[0].package_version
    );
    assert_eq!(
        assembly_identity,
        runtime_assembly_identity(&relabeled_assembly).unwrap()
    );

    assert_no_human_version_keys(
        &serde_json::to_value(service_deployment_identity_projection(&deployment).unwrap())
            .unwrap(),
    );
    assert_no_human_version_keys(
        &serde_json::to_value(runtime_assembly_identity_projection(&assembly).unwrap()).unwrap(),
    );
}

fn relabel_human_versions(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                relabel_human_versions(value);
            }
        }
        serde_json::Value::Object(fields) => {
            for key in ["packageVersion", "contractVersion", "exactVersion"] {
                if fields.contains_key(key) {
                    fields.insert(key.to_string(), serde_json::json!("99.99.99"));
                }
            }
            for value in fields.values_mut() {
                relabel_human_versions(value);
            }
        }
        _ => {}
    }
}

fn assert_no_human_version_keys(value: &serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                assert_no_human_version_keys(value);
            }
        }
        serde_json::Value::Object(fields) => {
            for key in fields.keys() {
                assert!(
                    !["packageVersion", "contractVersion", "exactVersion"].contains(&key.as_str()),
                    "identity preimage contains human version label key {key}"
                );
            }
            for value in fields.values() {
                assert_no_human_version_keys(value);
            }
        }
        _ => {}
    }
}

#[test]
fn deployment_validation_rejects_duplicates_dangling_refs_and_tamper() {
    let deployment = rich_deployment();

    let mut duplicate = deployment.clone();
    duplicate
        .package_bindings
        .push(duplicate.package_bindings[0].clone());
    assert!(service_deployment_identity(&duplicate).is_err());

    let mut dangling = deployment.clone();
    dangling.ingress[0].contract_operation_id = "operation.missing".into();
    assert!(service_deployment_identity(&dangling).is_err());

    let mut coordinate_mismatch = deployment.clone();
    coordinate_mismatch.package_bindings.push(PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: coordinate_mismatch.implementation.package_build_id.clone(),
            package_requirement_alias: "conflicting".to_string(),
        },
        package: PackageArtifactRef {
            package_id: "wrong.coordinate".to_string(),
            package_build_id: coordinate_mismatch.package_bindings[0]
                .package
                .package_build_id
                .clone(),
            ..coordinate_mismatch.package_bindings[0].package.clone()
        },
    });
    assert!(service_deployment_identity(&coordinate_mismatch).is_err());

    let mut tampered = deployment.clone();
    tampered.deployment_artifact_identity = "tampered".into();
    assert!(validate_service_deployment_identity(&tampered).is_err());

    let mut wrong_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
    wrong_ref.contract_version = "9.9.9".to_string();
    assert!(validate_service_deployment_ref(&wrong_ref, &deployment).is_err());
}

#[test]
fn empty_assembly_assign_validate_and_round_trip_are_stable() {
    let assembly = empty_runtime_assembly_fixture().expect("empty assembly");
    validate_runtime_assembly_identity(&assembly).expect("valid empty assembly");
    let encoded = serde_json::to_vec(&assembly).expect("serialize empty assembly");
    let decoded = serde_json::from_slice(&encoded).expect("deserialize empty assembly");
    assert_eq!(assembly, decoded);
    assert_eq!(
        runtime_assembly_identity(&decoded).expect("round-trip identity"),
        assembly.assembly_identity
    );
    assert_eq!(
        assembly.assembly_identity.as_str(),
        "skiff-runtime-assembly-v1:sha256:4176e39122928fcf47db987c34884f2f7ab4a1833c502a33bb6fd0c861a5acf6"
    );
}

#[test]
fn assembly_identity_includes_graph_link_plan_and_templates() {
    let assembly = runtime_assembly_fixture().expect("assembly fixture");
    let expected = runtime_assembly_identity(&assembly).unwrap();

    let mut graph = assembly.clone();
    let dependency = additional_package();
    graph.resolved_packages.push(dependency.clone());
    graph
        .package_link_plan
        .code_slots
        .push(skiff_artifact_model::PackageCodeSlot {
            package: dependency,
        });
    assert_ne!(runtime_assembly_identity(&graph).unwrap(), expected);

    let mut link_plan = graph.clone();
    link_plan
        .package_link_plan
        .package_links
        .push(PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: assembly.resolved_packages[0].package_build_id.clone(),
                package_requirement_alias: "dependency".to_string(),
            },
            package: link_plan.resolved_packages[1].clone(),
        });
    assert_ne!(
        runtime_assembly_identity(&link_plan).unwrap(),
        runtime_assembly_identity(&graph).unwrap()
    );

    let mut service_template = assembly.clone();
    service_template.service_binding_templates[0]
        .bindings
        .push(ResolvedServiceBinding {
            key: ServiceRequirementKey {
                caller_package_build_id: assembly.resolved_packages[0].package_build_id.clone(),
                service_requirement_slot: 0,
            },
            contract: assembly.resolved_contracts[0].clone(),
            provider: assembly.resolved_deployments[0].clone(),
            used_operations: vec![ContractOperationId::new("operation.echo")],
        });
    assert_ne!(
        runtime_assembly_identity(&service_template).unwrap(),
        expected
    );

    let mut activation = assembly.clone();
    activation.activation_templates[0].policy.timeout_ms = activation.activation_templates[0]
        .policy
        .timeout_ms
        .map(|timeout| timeout + 1);
    assert_ne!(runtime_assembly_identity(&activation).unwrap(), expected);

    let mut ingress = assembly.clone();
    ingress.global_ingress[0].selector.path = "/echo-v2".to_string();
    assert_ne!(runtime_assembly_identity(&ingress).unwrap(), expected);
}

#[test]
fn assembly_normalization_is_insertion_order_independent() {
    let mut assembly = runtime_assembly_fixture().expect("assembly fixture");
    let dependency = additional_package();
    assembly.resolved_packages.push(dependency.clone());
    assembly
        .package_link_plan
        .code_slots
        .push(skiff_artifact_model::PackageCodeSlot {
            package: dependency,
        });
    assign_runtime_assembly_identity(&mut assembly).unwrap();
    let expected = assembly.assembly_identity.clone();
    assembly.resolved_packages.reverse();
    assembly.package_link_plan.code_slots.reverse();
    assert_eq!(runtime_assembly_identity(&assembly).unwrap(), expected);
}

#[test]
fn assembly_validation_rejects_dangling_collision_and_tamper() {
    let assembly = runtime_assembly_fixture().expect("assembly fixture");

    let mut dangling = assembly.clone();
    dangling.package_link_plan.code_slots.clear();
    assert!(validate_runtime_assembly_surface(&dangling).is_err());

    let mut collision = assembly.clone();
    collision
        .global_ingress
        .push(collision.global_ingress[0].clone());
    assert!(validate_runtime_assembly_surface(&collision).is_err());

    let mut duplicate_slot = assembly.clone();
    let binding = ResolvedServiceBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: assembly.resolved_packages[0].package_build_id.clone(),
            service_requirement_slot: 0,
        },
        contract: assembly.resolved_contracts[0].clone(),
        provider: assembly.resolved_deployments[0].clone(),
        used_operations: vec![ContractOperationId::new("operation.echo")],
    };
    duplicate_slot.service_binding_templates[0].bindings = vec![binding.clone(), binding];
    assert!(validate_runtime_assembly_surface(&duplicate_slot).is_err());

    let mut tampered = assembly;
    tampered.assembly_identity = "tampered".into();
    assert!(validate_runtime_assembly_identity(&tampered).is_err());
}

#[test]
fn service_requirement_slot_is_scoped_by_caller_package_build() {
    let mut assembly = runtime_assembly_fixture().expect("assembly fixture");
    let dependency = additional_package();
    assembly.resolved_packages.push(dependency.clone());
    assembly
        .package_link_plan
        .code_slots
        .push(skiff_artifact_model::PackageCodeSlot {
            package: dependency.clone(),
        });
    let common = |caller_package_build_id| ResolvedServiceBinding {
        key: ServiceRequirementKey {
            caller_package_build_id,
            service_requirement_slot: 0,
        },
        contract: assembly.resolved_contracts[0].clone(),
        provider: assembly.resolved_deployments[0].clone(),
        used_operations: vec![ContractOperationId::new("operation.echo")],
    };
    assembly.service_binding_templates[0].bindings = vec![
        common(assembly.resolved_packages[0].package_build_id.clone()),
        common(dependency.package_build_id),
    ];
    validate_runtime_assembly_surface(&assembly)
        .expect("slot zero for two distinct caller builds must not collide");
}
