use std::collections::BTreeMap;

use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
    DeploymentIngressBinding, DeploymentPolicy, DeploymentRevision, GatewayAdapterArg,
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketJsonRpcProtocolSurface, GatewayWebSocketRpcProfile,
    GatewayWebSocketShapeVersion, IngressProtocol, IngressSelector, PackageArtifactRef,
    PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, ResourcePolicy, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentInput, ServiceProtocolIdentity,
    SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
    WEBSOCKET_GATEWAY_ENTRY_KEY,
};

use super::{
    assign_service_deployment_identity, gateway_entry_identity, service_deployment_identity,
    validate_service_deployment_input, validate_service_deployment_surface,
    DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX, DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER,
    GATEWAY_ENTRY_IDENTITY_PREFIX, GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
    SERVICE_PROTOCOL_IDENTITY_PREFIX,
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
        GatewayAdapterKind::WebSocketConnect => {
            panic!("HTTP deployment fixture does not accept websocketConnect")
        }
        GatewayAdapterKind::WebSocketJsonRpc => {
            panic!("HTTP deployment fixture does not accept websocketJsonRpc")
        }
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
        GatewayAdapterKind::WebSocketConnect => {
            panic!("HTTP deployment fixture does not accept websocketConnect")
        }
        GatewayAdapterKind::WebSocketJsonRpc => {
            panic!("HTTP deployment fixture does not accept websocketJsonRpc")
        }
    };
    DeploymentGatewayEntry {
        gateway_entry_identity: gateway_entry_identity(&protocol_surface).unwrap(),
        protocol_surface,
        handler: Some(PackageCallableId::new(
            "pkg-callable:example.provider:gateway",
        )),
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

fn websocket_gateway_entry(
    handler: Option<PackageCallableId>,
    args: Vec<GatewayAdapterArg>,
) -> DeploymentGatewayEntry {
    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketConnect(
            GatewayWebSocketConnectProtocolSurface {
                connect_request_shape: GatewayWebSocketShapeVersion::V1,
                connect_result_shape: GatewayWebSocketShapeVersion::V1,
                connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                external_sources: vec![
                    GatewayAdapterSource::WebSocketConnectRequest,
                    GatewayAdapterSource::WebSocketConnectionId,
                ],
                downlink_frames: vec![
                    GatewayWebSocketDownlinkFrame::Binary,
                    GatewayWebSocketDownlinkFrame::Text,
                ],
                rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    DeploymentGatewayEntry {
        gateway_entry_identity: gateway_entry_identity(&protocol_surface).unwrap(),
        protocol_surface,
        handler,
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketConnect,
            args,
        },
    }
}

fn websocket_json_rpc_gateway_entry(
    handler: Option<PackageCallableId>,
    args: Vec<GatewayAdapterArg>,
    params_schema: GatewayExternalSchema,
    result_schema: GatewayExternalSchema,
) -> DeploymentGatewayEntry {
    let mut external_sources = args
        .iter()
        .map(|argument| argument.source)
        .collect::<Vec<_>>();
    external_sources.sort_by_key(|source| source.wire_name());
    external_sources.dedup();
    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketJsonRpc(
            GatewayWebSocketJsonRpcProtocolSurface {
                profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources,
                params_schema,
                result_schema,
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    DeploymentGatewayEntry {
        gateway_entry_identity: gateway_entry_identity(&protocol_surface).unwrap(),
        protocol_surface,
        handler,
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketJsonRpc,
            args,
        },
    }
}

fn selector(path: &str, key: GatewayEntryKey) -> DeploymentIngressBinding {
    DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: path.to_string(),
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
        ingress: vec![selector("/gateway", key)],
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

fn websocket_deployment(
    handler: Option<PackageCallableId>,
    args: Vec<GatewayAdapterArg>,
) -> ServiceDeployment {
    let mut deployment = deployment_with(GatewayAdapterKind::TypedJson);
    deployment.gateway_entries.clear();
    deployment.ingress.clear();
    let key = GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap();
    deployment
        .gateway_entries
        .insert(key.clone(), websocket_gateway_entry(handler, args));
    deployment.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: None,
            path: "/chat".to_string(),
        },
        gateway_entry_key: key,
    });
    assign_service_deployment_identity(&mut deployment).unwrap();
    deployment
}

fn websocket_json_rpc_deployment() -> ServiceDeployment {
    let mut deployment = websocket_deployment(None, Vec::new());
    let key = GatewayEntryKey::parse("status").unwrap();
    deployment.gateway_entries.insert(
        key.clone(),
        websocket_json_rpc_gateway_entry(
            Some(PackageCallableId::new(
                "pkg-callable:example.provider:status",
            )),
            vec![
                GatewayAdapterArg {
                    param: "params".to_string(),
                    source: GatewayAdapterSource::WebSocketJsonRpcParams,
                },
                GatewayAdapterArg {
                    param: "connectionId".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectionId,
                },
            ],
            GatewayExternalSchema::Record {
                fields: BTreeMap::from([("requestId".to_string(), GatewayExternalSchema::String)]),
                required: vec!["requestId".to_string()],
            },
            GatewayExternalSchema::Record {
                fields: BTreeMap::from([("status".to_string(), GatewayExternalSchema::String)]),
                required: vec!["status".to_string()],
            },
        ),
    );
    deployment.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: Some("status.get".to_string()),
            path: "/chat".to_string(),
        },
        gateway_entry_key: key,
    });
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
        deployment.ingress.push(selector("/gateway-alias", key));
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
fn deployment_gateway_validation_accepts_websocket_with_or_without_connect_handler() {
    let no_handler = websocket_deployment(None, Vec::new());
    validate_service_deployment_surface(&no_handler).unwrap();
    validate_service_deployment_input(&input_from(&no_handler)).unwrap();

    let with_handler = websocket_deployment(
        Some(PackageCallableId::new(
            "pkg-callable:example.provider:connect",
        )),
        vec![
            GatewayAdapterArg {
                param: "request".to_string(),
                source: GatewayAdapterSource::WebSocketConnectRequest,
            },
            GatewayAdapterArg {
                param: "connectionId".to_string(),
                source: GatewayAdapterSource::WebSocketConnectionId,
            },
        ],
    );
    validate_service_deployment_surface(&with_handler).unwrap();

    let mut aliased = no_handler;
    aliased.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: None,
            path: "/chat-alias".to_string(),
        },
        gateway_entry_key: GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap(),
    });
    validate_service_deployment_surface(&aliased).unwrap();
}

#[test]
fn deployment_gateway_validation_accepts_linked_websocket_json_rpc_methods() {
    let deployment = websocket_json_rpc_deployment();
    validate_service_deployment_surface(&deployment).unwrap();
    validate_service_deployment_input(&input_from(&deployment)).unwrap();

    let physical = deployment
        .ingress
        .iter()
        .find(|binding| binding.gateway_entry_key.as_str() == WEBSOCKET_GATEWAY_ENTRY_KEY)
        .unwrap();
    let method = deployment
        .ingress
        .iter()
        .find(|binding| binding.gateway_entry_key.as_str() == "status")
        .unwrap();
    assert_eq!(physical.selector.method, None);
    assert_eq!(method.selector.method.as_deref(), Some("status.get"));
    assert_eq!(method.selector.path, physical.selector.path);
}

#[test]
fn deployment_websocket_json_rpc_identity_boundaries_are_exact() {
    let baseline = websocket_json_rpc_deployment();
    let baseline_gateway = baseline.gateway_entries[&GatewayEntryKey::parse("status").unwrap()]
        .gateway_entry_identity
        .clone();
    let baseline_deployment = service_deployment_identity(&baseline).unwrap();

    let mut renamed_method = baseline.clone();
    renamed_method
        .ingress
        .iter_mut()
        .find(|binding| binding.gateway_entry_key.as_str() == "status")
        .unwrap()
        .selector
        .method = Some("status.read".to_string());
    assert_eq!(
        renamed_method.gateway_entries[&GatewayEntryKey::parse("status").unwrap()]
            .gateway_entry_identity,
        baseline_gateway
    );
    assert_ne!(
        service_deployment_identity(&renamed_method).unwrap(),
        baseline_deployment
    );

    let mut shape_changed = baseline.clone();
    let entry = shape_changed
        .gateway_entries
        .get_mut(&GatewayEntryKey::parse("status").unwrap())
        .unwrap();
    let GatewayProtocolSurface::WebSocketJsonRpc(surface) = &mut entry.protocol_surface.protocol
    else {
        panic!("status must be websocketJsonRpc")
    };
    surface.params_schema = GatewayExternalSchema::Array {
        items: Box::new(GatewayExternalSchema::String),
    };
    entry.gateway_entry_identity = gateway_entry_identity(&entry.protocol_surface).unwrap();
    assert_ne!(entry.gateway_entry_identity, baseline_gateway);
    assert_ne!(
        service_deployment_identity(&shape_changed).unwrap(),
        baseline_deployment
    );

    let mut handler_changed = baseline.clone();
    handler_changed
        .gateway_entries
        .get_mut(&GatewayEntryKey::parse("status").unwrap())
        .unwrap()
        .handler = Some(PackageCallableId::new(
        "pkg-callable:example.provider:replacementStatus",
    ));
    assert_eq!(
        handler_changed.gateway_entries[&GatewayEntryKey::parse("status").unwrap()]
            .gateway_entry_identity,
        baseline_gateway
    );
    assert_ne!(
        service_deployment_identity(&handler_changed).unwrap(),
        baseline_deployment
    );

    let mut key_changed = baseline.clone();
    let old_key = GatewayEntryKey::parse("status").unwrap();
    let new_key = GatewayEntryKey::parse("renamed-status").unwrap();
    let entry = key_changed.gateway_entries.remove(&old_key).unwrap();
    key_changed.gateway_entries.insert(new_key.clone(), entry);
    key_changed
        .ingress
        .iter_mut()
        .find(|binding| binding.gateway_entry_key == old_key)
        .unwrap()
        .gateway_entry_key = new_key.clone();
    assert_eq!(
        key_changed.gateway_entries[&new_key].gateway_entry_identity,
        baseline_gateway
    );
    assert_ne!(
        service_deployment_identity(&key_changed).unwrap(),
        baseline_deployment
    );

    let mut args_reordered = baseline.clone();
    args_reordered
        .gateway_entries
        .get_mut(&GatewayEntryKey::parse("status").unwrap())
        .unwrap()
        .adapter_plan
        .args
        .reverse();
    assert_eq!(
        args_reordered.gateway_entries[&GatewayEntryKey::parse("status").unwrap()]
            .gateway_entry_identity,
        baseline_gateway
    );
    assert_ne!(
        service_deployment_identity(&args_reordered).unwrap(),
        baseline_deployment
    );
}

#[test]
fn deployment_websocket_json_rpc_association_and_tamper_fail_closed() {
    let baseline = websocket_json_rpc_deployment();
    let assert_invalid = |deployment: &ServiceDeployment| {
        assert!(
            validate_service_deployment_surface(deployment).is_err(),
            "tampered deployment unexpectedly validated"
        );
    };

    let mut missing_physical = baseline.clone();
    missing_physical
        .gateway_entries
        .remove(&GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap());
    missing_physical
        .ingress
        .retain(|binding| binding.gateway_entry_key.as_str() != WEBSOCKET_GATEWAY_ENTRY_KEY);
    assert_invalid(&missing_physical);

    let mut method_without_selector = baseline.clone();
    method_without_selector
        .ingress
        .iter_mut()
        .find(|binding| binding.gateway_entry_key.as_str() == "status")
        .unwrap()
        .selector
        .method = None;
    assert_invalid(&method_without_selector);

    let mut physical_with_method = baseline.clone();
    physical_with_method
        .ingress
        .iter_mut()
        .find(|binding| binding.gateway_entry_key.as_str() == WEBSOCKET_GATEWAY_ENTRY_KEY)
        .unwrap()
        .selector
        .method = Some("status.get".to_string());
    assert_invalid(&physical_with_method);

    let mutations: [fn(&mut IngressSelector); 1] =
        [|selector: &mut IngressSelector| selector.path = "/other".to_string()];
    for mutate in mutations {
        let mut mismatched = baseline.clone();
        mutate(
            &mut mismatched
                .ingress
                .iter_mut()
                .find(|binding| binding.gateway_entry_key.as_str() == "status")
                .unwrap()
                .selector,
        );
        assert_invalid(&mismatched);
    }

    let mut missing_handler = baseline.clone();
    missing_handler
        .gateway_entries
        .get_mut(&GatewayEntryKey::parse("status").unwrap())
        .unwrap()
        .handler = None;
    assert_invalid(&missing_handler);

    let mut wrong_kind = baseline.clone();
    wrong_kind
        .gateway_entries
        .get_mut(&GatewayEntryKey::parse("status").unwrap())
        .unwrap()
        .adapter_plan
        .kind = GatewayAdapterKind::WebSocketConnect;
    assert_invalid(&wrong_kind);

    let mut wrong_source = baseline.clone();
    wrong_source
        .gateway_entries
        .get_mut(&GatewayEntryKey::parse("status").unwrap())
        .unwrap()
        .adapter_plan
        .args[0]
        .source = GatewayAdapterSource::WebSocketConnectRequest;
    assert_invalid(&wrong_source);

    let mut source_set_mismatch = baseline.clone();
    source_set_mismatch
        .gateway_entries
        .get_mut(&GatewayEntryKey::parse("status").unwrap())
        .unwrap()
        .adapter_plan
        .args
        .pop();
    assert_invalid(&source_set_mismatch);

    let mut reserved_http = deployment_with(GatewayAdapterKind::TypedJson);
    let old_key = reserved_http.gateway_entries.keys().next().unwrap().clone();
    let entry = reserved_http.gateway_entries.remove(&old_key).unwrap();
    let reserved = GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap();
    reserved_http
        .gateway_entries
        .insert(reserved.clone(), entry);
    reserved_http.ingress[0].gateway_entry_key = reserved;
    assert_invalid(&reserved_http);

    let mut duplicate_method = baseline;
    let duplicate_key = GatewayEntryKey::parse("duplicate-status").unwrap();
    duplicate_method.gateway_entries.insert(
        duplicate_key.clone(),
        duplicate_method.gateway_entries[&GatewayEntryKey::parse("status").unwrap()].clone(),
    );
    duplicate_method.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: Some("status.get".to_string()),
            path: "/chat".to_string(),
        },
        gateway_entry_key: duplicate_key,
    });
    assert_invalid(&duplicate_method);
}

#[test]
fn deployment_gateway_validation_rejects_invalid_websocket_cross_fields() {
    let assert_invalid = |deployment: &ServiceDeployment| {
        assert!(validate_service_deployment_surface(deployment).is_err());
    };

    let mut no_handler_args = websocket_deployment(None, Vec::new());
    no_handler_args
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .adapter_plan
        .args
        .push(GatewayAdapterArg {
            param: "request".to_string(),
            source: GatewayAdapterSource::WebSocketConnectRequest,
        });
    assert_invalid(&no_handler_args);

    let mut pre = websocket_deployment(None, Vec::new());
    pre.gateway_entries.values_mut().next().unwrap().pre =
        Some(PackageCallableId::new("pkg-callable:pre"));
    assert_invalid(&pre);

    let mut guard = websocket_deployment(None, Vec::new());
    guard.gateway_entries.values_mut().next().unwrap().guard =
        Some(PackageCallableId::new("pkg-callable:guard"));
    assert_invalid(&guard);

    let mut wrong_kind = websocket_deployment(None, Vec::new());
    wrong_kind
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .adapter_plan
        .kind = GatewayAdapterKind::RawHttp;
    assert_invalid(&wrong_kind);

    let mut wrong_key = websocket_deployment(None, Vec::new());
    let entry = wrong_key
        .gateway_entries
        .remove(&GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap())
        .unwrap();
    let other = GatewayEntryKey::parse("other").unwrap();
    wrong_key.gateway_entries.insert(other.clone(), entry);
    wrong_key.ingress[0].gateway_entry_key = other;
    assert_invalid(&wrong_key);

    let mut method = websocket_deployment(None, Vec::new());
    method.ingress[0].selector.method = Some("GET".to_string());
    assert_invalid(&method);

    let mut selector_mismatch = websocket_deployment(None, Vec::new());
    selector_mismatch.ingress[0].selector.protocol = IngressProtocol::Http;
    selector_mismatch.ingress[0].selector.method = Some("GET".to_string());
    assert_invalid(&selector_mismatch);

    let mut http_without_handler = deployment_with(GatewayAdapterKind::TypedJson);
    http_without_handler
        .gateway_entries
        .values_mut()
        .next()
        .unwrap()
        .handler = None;
    assert_invalid(&http_without_handler);
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
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "f".repeat(64)))
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
        .handler = Some(PackageCallableId::new(""));
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
    deployment.ingress.push(selector("/gateway-alias", key));
    let expected = service_deployment_identity(&deployment).unwrap();
    deployment.ingress.reverse();
    assert_eq!(service_deployment_identity(&deployment).unwrap(), expected);

    let mut stale_input = input_from(&deployment);
    stale_input.schema_version = "skiff-service-deployment-input-v4".to_string();
    assert!(validate_service_deployment_input(&stale_input).is_err());

    let mut stale_schema = deployment.clone();
    stale_schema.schema_version = "skiff-service-deployment-v3".to_string();
    assert!(service_deployment_identity(&stale_schema).is_err());

    let mut stale_identity = deployment;
    stale_identity.deployment_artifact_identity = DeploymentArtifactIdentity::new(format!(
        "skiff-deployment-artifact-v3:sha256:{}",
        "a".repeat(64)
    ));
    assert!(super::validate_service_deployment_identity(&stale_identity).is_err());
    assert_eq!(
        SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
        "skiff-service-deployment-input-v5"
    );
    assert_eq!(
        SERVICE_DEPLOYMENT_SCHEMA_VERSION,
        "skiff-service-deployment-v4"
    );
    assert_eq!(
        DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
        "skiff-deployment-artifact-v4:sha256"
    );
    assert_eq!(
        DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER,
        "skiff-deployment-artifact-identity-v4"
    );
    assert_eq!(
        GATEWAY_ENTRY_IDENTITY_PREFIX,
        "skiff-gateway-entry-v2:sha256"
    );
    assert_eq!(
        GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER,
        "skiff-gateway-entry-identity-v2"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        "skiff-package-build-v10:sha256"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
        "skiff-package-local-abi-v7:sha256"
    );
    assert_eq!(
        SERVICE_PROTOCOL_IDENTITY_PREFIX,
        "skiff-service-protocol-v5:sha256"
    );
    assert!(
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v1:sha256:{}", "a".repeat(64)))
            .is_err(),
        "stale gateway identity generation must fail at the typed reader"
    );
}
