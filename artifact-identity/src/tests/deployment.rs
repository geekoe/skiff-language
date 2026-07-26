use std::collections::BTreeMap;

use skiff_artifact_model::{
    ActivationPolicy, DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentPolicy, DeploymentRevision, GatewayAdapterArg,
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, IngressProtocol, IngressSelector, PackageArtifactRef, PackageBuildId,
    PackageCallableId, PackageLocalAbiIdentity, ResourcePolicy, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentInput, ServiceProtocolIdentity,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

use super::{
    assign_service_deployment_identity, gateway_entry_identity, service_deployment_identity,
    validate_service_deployment_input, validate_service_deployment_surface,
    DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
};

fn protocol_surface(kind: GatewayAdapterKind) -> GatewayEntryProtocolSurface {
    let http = match kind {
        GatewayAdapterKind::TypedJson => GatewayHttpProtocolSurface {
            adapter_kind: kind,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpBody],
            request_body_schema: Some(GatewayExternalSchema::String),
            response_schema: Some(GatewayExternalSchema::String),
            stream_item_schema: None,
        },
        GatewayAdapterKind::RawHttp => GatewayHttpProtocolSurface {
            adapter_kind: kind,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpRequest],
            request_body_schema: None,
            response_schema: None,
            stream_item_schema: None,
        },
    };
    GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(http),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    }
}

fn gateway_entry(kind: GatewayAdapterKind) -> DeploymentGatewayEntry {
    let protocol_surface = protocol_surface(kind);
    let source = match kind {
        GatewayAdapterKind::TypedJson => GatewayAdapterSource::HttpBody,
        GatewayAdapterKind::RawHttp => GatewayAdapterSource::HttpRequest,
    };
    DeploymentGatewayEntry {
        gateway_entry_identity: gateway_entry_identity(&protocol_surface).unwrap(),
        protocol_surface,
        handler: PackageCallableId::new("pkg-callable:example.provider:gateway"),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind,
            args: vec![GatewayAdapterArg {
                param: "input".to_string(),
                source,
            }],
        },
    }
}

fn selector(host: &str, key: GatewayEntryKey) -> DeploymentIngressBinding {
    DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            host: host.to_string(),
            method: Some("POST".to_string()),
            path: "/gateway".to_string(),
        },
        gateway_entry_key: key,
    }
}

fn deployment_with(kind: GatewayAdapterKind) -> ServiceDeployment {
    let key = GatewayEntryKey::parse("primary").unwrap();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: ServiceContractRef {
            service_id: "example.service".to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
        },
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: PackageArtifactRef {
            package_id: "example.provider".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("package-build"),
            package_local_abi_identity: PackageLocalAbiIdentity::new("package-abi"),
        },
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::from([(key.clone(), gateway_entry(kind))]),
        ingress: vec![selector("api.example.test", key)],
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: None,
            resources: ResourcePolicy {
                cpu_millis: 1,
                memory_bytes: 1,
            },
            activation: ActivationPolicy {
                max_concurrency: 1,
                idle_timeout_ms: None,
            },
            principal: "service:example.service".to_string(),
        },
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Gateway deployment".to_string(),
            notes: BTreeMap::new(),
        },
    };
    assign_service_deployment_identity(&mut deployment).unwrap();
    deployment
}

fn input_from(deployment: &ServiceDeployment) -> ServiceDeploymentInput {
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract: deployment.contract.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        implementation: deployment.implementation.clone(),
        operation_bindings: Vec::new(),
        package_bindings: deployment.package_bindings.clone(),
        service_selectors: deployment.service_selectors.clone(),
        gateway_entries: deployment.gateway_entries.clone(),
        ingress: deployment.ingress.clone(),
        config_literals: deployment.config_literals.clone(),
        secret_refs: deployment.secret_refs.clone(),
        state_bindings: deployment.state_bindings.clone(),
        resource_bindings: deployment.resource_bindings.clone(),
        runtime_capability_bindings: deployment.runtime_capability_bindings.clone(),
        policy: deployment.policy.clone(),
        diagnostic_text: deployment.diagnostic_text.clone(),
    }
}

#[test]
fn deployment_gateway_validation_accepts_typed_raw_multiple_selectors_and_zero() {
    for kind in [GatewayAdapterKind::TypedJson, GatewayAdapterKind::RawHttp] {
        let mut deployment = deployment_with(kind);
        let key = deployment.ingress[0].gateway_entry_key.clone();
        deployment.ingress.push(selector("alias.example.test", key));
        validate_service_deployment_surface(&deployment).unwrap();
        validate_service_deployment_input(&input_from(&deployment)).unwrap();
    }

    let mut empty = deployment_with(GatewayAdapterKind::TypedJson);
    empty.gateway_entries.clear();
    empty.ingress.clear();
    assign_service_deployment_identity(&mut empty).unwrap();
    validate_service_deployment_surface(&empty).unwrap();
    validate_service_deployment_input(&input_from(&empty)).unwrap();
}

#[test]
fn deployment_gateway_validation_rejects_cross_field_mismatches() {
    let baseline = deployment_with(GatewayAdapterKind::TypedJson);

    let mut identity = baseline.clone();
    identity
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .gateway_entry_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v1:sha256:{}", "f".repeat(64)))
            .unwrap();
    assert!(validate_service_deployment_surface(&identity).is_err());

    let mut kind = baseline.clone();
    kind.gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .adapter_plan
        .kind = GatewayAdapterKind::RawHttp;
    assert!(validate_service_deployment_surface(&kind).is_err());

    let mut sources = baseline.clone();
    sources
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .adapter_plan
        .args
        .push(GatewayAdapterArg {
            param: "request".to_string(),
            source: GatewayAdapterSource::HttpRequest,
        });
    assert!(validate_service_deployment_surface(&sources).is_err());

    let mut duplicate_param = baseline.clone();
    duplicate_param
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .adapter_plan
        .args
        .push(GatewayAdapterArg {
            param: "input".to_string(),
            source: GatewayAdapterSource::HttpBody,
        });
    assert!(validate_service_deployment_surface(&duplicate_param).is_err());

    let mut context = baseline.clone();
    context
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .adapter_plan
        .args
        .push(GatewayAdapterArg {
            param: "context".to_string(),
            source: GatewayAdapterSource::HttpContext,
        });
    assert!(validate_service_deployment_surface(&context).is_err());

    let mut empty_handler = baseline.clone();
    empty_handler
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .handler = PackageCallableId::new("");
    assert!(validate_service_deployment_surface(&empty_handler).is_err());

    let mut empty_pre = baseline.clone();
    empty_pre.gateway_entries.values_mut().next().unwrap().pre = Some(PackageCallableId::new(""));
    assert!(validate_service_deployment_surface(&empty_pre).is_err());

    let mut empty_guard = baseline.clone();
    empty_guard
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .guard = Some(PackageCallableId::new(""));
    assert!(validate_service_deployment_surface(&empty_guard).is_err());

    let mut missing = baseline.clone();
    missing.ingress[0].gateway_entry_key = GatewayEntryKey::parse("missing").unwrap();
    assert!(validate_service_deployment_surface(&missing).is_err());

    let mut orphan = baseline.clone();
    orphan.gateway_entries.insert(
        GatewayEntryKey::parse("orphan").unwrap(),
        gateway_entry(GatewayAdapterKind::RawHttp),
    );
    assert!(validate_service_deployment_surface(&orphan).is_err());

    let mut duplicate_selector = baseline.clone();
    duplicate_selector
        .ingress
        .push(duplicate_selector.ingress[0].clone());
    assert!(validate_service_deployment_surface(&duplicate_selector).is_err());

    let mut websocket = baseline;
    websocket.ingress[0].selector.protocol = IngressProtocol::WebSocket;
    websocket.ingress[0].selector.method = None;
    assert!(validate_service_deployment_surface(&websocket).is_err());
}

#[test]
fn deployment_identity_is_stable_under_reorder_and_rejects_stale_generation() {
    let mut deployment = deployment_with(GatewayAdapterKind::TypedJson);
    let key = deployment.ingress[0].gateway_entry_key.clone();
    deployment.ingress.push(selector("alias.example.test", key));
    let expected = service_deployment_identity(&deployment).unwrap();
    deployment.ingress.reverse();
    assert_eq!(service_deployment_identity(&deployment).unwrap(), expected);

    let mut stale_input = input_from(&deployment);
    stale_input.schema_version = "skiff-service-deployment-input-v1".to_string();
    assert!(validate_service_deployment_input(&stale_input).is_err());

    let mut stale_schema = deployment.clone();
    stale_schema.schema_version = "skiff-service-deployment-v1".to_string();
    assert!(service_deployment_identity(&stale_schema).is_err());

    let mut stale_identity = deployment;
    stale_identity.deployment_artifact_identity = DeploymentArtifactIdentity::new(format!(
        "skiff-deployment-artifact-v1:sha256:{}",
        "a".repeat(64)
    ));
    assert!(super::validate_service_deployment_identity(&stale_identity).is_err());
    assert_eq!(
        DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
        "skiff-deployment-artifact-v2:sha256"
    );
}
