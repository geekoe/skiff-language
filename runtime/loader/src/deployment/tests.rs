use std::collections::BTreeMap;

use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentOperationBinding, DeploymentRevision,
    GatewayAdapterPlan, GatewayAdapterKind, GatewayEntryIdentity, GatewayEntryKey,
    GatewayEntryProtocolSurface, GatewayHttpProtocolSurface, GatewayAdapterSource,
    GatewayDispatchMode, GatewayExternalErrorProjection, GatewayExternalSchema,
    IngressProtocol, IngressSelector, PackageArtifactRef, PackageBuildId,
    PackageCallableId, PackageLocalAbiIdentity, PackageRequirementKey, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, ServiceProtocolIdentity, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_artifact_identity::{gateway_entry_identity, service_deployment_ref};

use super::compose_deployment_assembly;

fn package_ref(package_id: &str) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new(format!("build:{package_id}")),
        package_local_abi_identity: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
    }
}

fn http_gateway_entry() -> (GatewayEntryKey, DeploymentGatewayEntry) {
    let surface = GatewayEntryProtocolSurface {
        protocol: skiff_artifact_model::GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: Vec::new(),
            request_body_schema: None,
            response_schema: None,
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let identity = gateway_entry_identity(&surface).unwrap();
    (
        GatewayEntryKey::parse("fixture-http").unwrap(),
        DeploymentGatewayEntry {
            gateway_entry_identity: identity,
            protocol_surface: surface,
            handler: Some(PackageCallableId::new("pkg-callable:example:health")),
            pre: None,
            guard: None,
            adapter_plan: GatewayAdapterPlan {
                kind: GatewayAdapterKind::TypedJson,
                args: Vec::new(),
            },
        },
    )
}

fn deployment() -> ServiceDeployment {
    let contract = ServiceContractRef {
        service_id: "example.health".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
    };
    let (entry_key, entry) = http_gateway_entry();
    ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref("example.health-provider"),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::from([(entry_key.clone(), entry)]),
        ingress: vec![DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("GET".to_string()),
                path: "/health".to_string(),
            },
            gateway_entry_key: entry_key,
        }],
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Health deployment".to_string(),
            notes: BTreeMap::new(),
        },
    }
}

#[test]
fn compose_succeeds_for_self_contained_deployment() {
    let mut deployment = deployment();
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    let assembly = compose_deployment_assembly(&reference, &deployment).unwrap();
    assert_eq!(assembly.roots, vec![reference.clone()]);
    assert_eq!(assembly.resolved_deployments, vec![reference.clone()]);
    assert_eq!(assembly.resolved_contracts, vec![deployment.contract]);
    assert_eq!(
        assembly.activation_templates,
        vec![skiff_artifact_model::ActivationTemplate {
            deployment: reference.clone(),
            implementation_package_build_id: deployment.implementation.package_build_id.clone(),
        }]
    );
    assert_eq!(assembly.gateway_ingress.len(), 1);
    assert_eq!(assembly.gateway_ingress[0].deployment, reference);
    assert_eq!(
        assembly.gateway_ingress[0].gateway_entry_identity,
        deployment
            .gateway_entries
            .values()
            .next()
            .unwrap()
            .gateway_entry_identity
    );
    assert_eq!(assembly.service_binding_templates[0].bindings, Vec::new());
    assert_eq!(assembly.package_link_plan.code_slots.len(), 1);
    skiff_artifact_identity::validate_runtime_assembly_identity(&assembly).unwrap();
}

#[test]
fn compose_closes_package_bindings() {
    let mut deployment = deployment();
    deployment.package_bindings = vec![skiff_artifact_model::PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: deployment.implementation.package_build_id.clone(),
            package_requirement_alias: "lib".to_string(),
        },
        package: package_ref("example.lib"),
    }];
    deployment.gateway_entries.clear();
    deployment.ingress.clear();
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    let assembly = compose_deployment_assembly(&reference, &deployment).unwrap();
    let builds = assembly
        .package_link_plan
        .code_slots
        .iter()
        .map(|slot| slot.package.package_build_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(builds.len(), 2);
    assert!(builds.contains(&"build:example.health-provider"));
    assert!(builds.contains(&"build:example.lib"));
    assert_eq!(assembly.package_link_plan.package_links.len(), 1);
}

#[test]
fn compose_rejects_cross_service_dependencies() {
    let mut deployment = deployment();
    deployment.service_selectors = vec![skiff_artifact_model::ServiceSelectorBinding {
        key: skiff_artifact_model::ServiceRequirementKey {
            caller_package_build_id: deployment.implementation.package_build_id.clone(),
            service_requirement_slot: 0,
        },
        contract: deployment.contract.clone(),
    }];
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    let error = compose_deployment_assembly(&reference, &deployment)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cross-service dependencies"), "{error}");
}

#[test]
fn compose_rejects_unreachable_package_bindings() {
    let mut deployment = deployment();
    deployment.package_bindings = vec![skiff_artifact_model::PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: PackageBuildId::new("build:unknown"),
            package_requirement_alias: "lib".to_string(),
        },
        package: package_ref("example.lib"),
    }];
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = service_deployment_ref(&deployment);
    let error = compose_deployment_assembly(&reference, &deployment)
        .unwrap_err()
        .to_string();
    assert!(error.contains("outside its reachable closure"), "{error}");
}

#[test]
fn compose_rejects_exact_ref_mismatch() {
    let deployment = deployment();
    let mut other = deployment.clone();
    other.deployment_revision = DeploymentRevision::new("revision-2");
    let reference = service_deployment_ref(&other);
    let error = compose_deployment_assembly(&reference, &deployment)
        .unwrap_err()
        .to_string();
    assert!(error.contains("deployment"), "{error}");
}
