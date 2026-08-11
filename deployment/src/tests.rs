use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_service_deployment_identity, gateway_entry_identity, service_deployment_identity,
    service_deployment_identity_projection, validate_service_deployment_identity,
    validate_service_deployment_ref,
};
use skiff_artifact_model::{
    ContractOperationId, DeploymentIngressBinding, DeploymentOperationBinding, GatewayAdapterArg,
    GatewayAdapterSource, GatewayEntryIdentity, GatewayEntryKey, GatewayExternalSchema,
    GatewayProtocolSurface, IngressProtocol, IngressSelector, PackageArtifactRef, PackageBinding,
    PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, PackageRequirementKey,
    ServiceDeployment, ServiceRequirementKey, ServiceSelectorBinding,
};

use crate::fixtures::{gateway_entry_fixture, service_deployment_fixture};

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
    let health_key = GatewayEntryKey::parse("health").expect("gateway key");
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
    deployment.gateway_entries.insert(
        health_key.clone(),
        gateway_entry_fixture(PackageCallableId::new("callable.health")),
    );
    deployment.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("GET".to_string()),
            path: "/health".to_string(),
        },
        gateway_entry_key: health_key,
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

    let mut missing_gateway_entries =
        serde_json::to_value(&deployment).expect("serialize deployment");
    missing_gateway_entries
        .as_object_mut()
        .unwrap()
        .remove("gatewayEntries");
    assert!(serde_json::from_value::<ServiceDeployment>(missing_gateway_entries).is_err());

    let mut legacy_ingress = serde_json::to_value(&deployment).expect("serialize deployment");
    let ingress = legacy_ingress["ingress"][0].as_object_mut().unwrap();
    ingress.remove("gatewayEntryKey");
    ingress.insert(
        "contractOperationId".to_string(),
        serde_json::json!("operation.echo"),
    );
    assert!(serde_json::from_value::<ServiceDeployment>(legacy_ingress).is_err());

    let encoded = serde_json::to_string(&deployment).unwrap();
    let entry = serde_json::to_string(
        deployment
            .gateway_entries
            .get(&GatewayEntryKey::parse("echo").unwrap())
            .unwrap(),
    )
    .unwrap();
    let unique = format!(r#""gatewayEntries":{{"echo":{entry}}}"#);
    let duplicate = format!(r#""gatewayEntries":{{"echo":{entry},"echo":{entry}}}"#);
    assert!(encoded.contains(&unique));
    assert!(
        serde_json::from_str::<ServiceDeployment>(&encoded.replace(&unique, &duplicate)).is_err()
    );

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
    assert!(serde_json::from_value::<ServiceSelectorBinding>(selector).is_err());}

#[test]
fn deployment_identity_is_order_independent_and_excludes_diagnostics() {
    let deployment = rich_deployment();
    let expected = service_deployment_identity(&deployment).expect("identity");

    let mut reordered = deployment.clone();
    reordered.operation_bindings.reverse();
    reordered.package_bindings.reverse();
    reordered.service_selectors.reverse();
    reordered.ingress.reverse();
    let entries = reordered.gateway_entries.clone();
    reordered.gateway_entries = BTreeMap::new();
    for (key, entry) in entries.into_iter().rev() {
        reordered.gateway_entries.insert(key, entry);
    }
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
fn removed_collection_mapping_wire_is_rejected_by_deployment() {
    let mut removed_wire = serde_json::to_value(rich_deployment()).unwrap();
    removed_wire["packageBindings"][0]["collectionNameMapping"] = serde_json::json!({});
    assert!(serde_json::from_value::<ServiceDeployment>(removed_wire).is_err());
}

#[test]
fn deployment_identity_mutation_matrix_covers_every_semantic_category() {
    type DeploymentMutation = Box<dyn Fn(&mut ServiceDeployment)>;

    let deployment = rich_deployment();
    let expected = service_deployment_identity(&deployment).unwrap();
    let cases: Vec<(&str, DeploymentMutation)> = vec![
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
            "gateway key",
            Box::new(|value| {
                let old = value.ingress[0].gateway_entry_key.clone();
                let new = GatewayEntryKey::parse("echo-renamed").unwrap();
                let entry = value.gateway_entries.remove(&old).unwrap();
                value.gateway_entries.insert(new.clone(), entry);
                value.ingress[0].gateway_entry_key = new;
            }),
        ),
        (
            "gateway protocol surface and identity",
            Box::new(|value| {
                let entry = value
                    .gateway_entries
                    .get_mut(&GatewayEntryKey::parse("echo").unwrap())
                    .unwrap();
                let GatewayProtocolSurface::Http(http) = &mut entry.protocol_surface.protocol
                else {
                    panic!("HTTP fixture unexpectedly contains websocketConnect");
                };
                http.response_schema = Some(GatewayExternalSchema::Integer);
                entry.gateway_entry_identity =
                    gateway_entry_identity(&entry.protocol_surface).unwrap();
            }),
        ),
        (
            "gateway handler",
            Box::new(|value| {
                value.gateway_entries.values_mut().next().unwrap().handler =
                    Some(PackageCallableId::new("callable.changed-handler"));
            }),
        ),
        (
            "gateway pre",
            Box::new(|value| {
                value.gateway_entries.values_mut().next().unwrap().pre =
                    Some(PackageCallableId::new("callable.pre"));
            }),
        ),
        (
            "gateway guard",
            Box::new(|value| {
                value.gateway_entries.values_mut().next().unwrap().guard =
                    Some(PackageCallableId::new("callable.guard"));
            }),
        ),
        (
            "gateway adapter param",
            Box::new(|value| {
                value
                    .gateway_entries
                    .values_mut()
                    .next()
                    .unwrap()
                    .adapter_plan
                    .args[0]
                    .param = "payload".to_string();
            }),
        ),
        (
            "gateway adapter source",
            Box::new(|value| {
                let entry = value.gateway_entries.values_mut().next().unwrap();
                entry.adapter_plan.args.push(GatewayAdapterArg {
                    param: "request".to_string(),
                    source: GatewayAdapterSource::HttpRequest,
                });
                let GatewayProtocolSurface::Http(http) = &mut entry.protocol_surface.protocol
                else {
                    panic!("HTTP fixture unexpectedly contains websocketConnect");
                };
                http.external_sources = vec![
                    GatewayAdapterSource::HttpBody,
                    GatewayAdapterSource::HttpRequest,
                ];
                entry.gateway_entry_identity =
                    gateway_entry_identity(&entry.protocol_surface).unwrap();
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
    assert_no_human_version_keys(
        &serde_json::to_value(service_deployment_identity_projection(&deployment).unwrap())
            .unwrap(),
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
    dangling.ingress[0].gateway_entry_key = GatewayEntryKey::parse("missing").unwrap();
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
