//! E-ws gate Gap 3 regression (`router-live:ws`):
//!
//! The HTTP surface view must project only `GatewayProtocolSurface::Http`
//! entries from a mixed HTTP+WebSocket deployment (WebSocket entries belong
//! to the WS surface view), so a real WS-only/WS-mixed service deployment no
//! longer fails the Router composition. The WS surface view must keep
//! projecting only WebSocket entries from the same record.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use skiff_artifact_identity::assign_service_deployment_identity;
use skiff_artifact_model::{
    DeploymentDiagnosticText, DeploymentGatewayEntry, DeploymentIngressBinding,
    DeploymentRevision, GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource,
    GatewayDispatchMode, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalSchema, GatewayWebSocketDownlinkFrame,
    GatewayExternalErrorProjection, GatewayHttpProtocolSurface, GatewayProtocolSurface,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketJsonRpcProtocolSurface,
    GatewayWebSocketRpcProfile, GatewayWebSocketShapeVersion, IngressProtocol, IngressSelector,
    PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity, ServiceContractRef,
    ServiceDeployment, ServiceProtocolIdentity, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::RoutingEpoch;
use skiff_router::supervisor::http::load_http_surface_view;
use skiff_router::supervisor::ws::load_ws_surface_view;
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot as RuntimeConfigSnapshotValue;

const SERVICE_ID: &str = "test.skiff/router-rust-ws-live";
const CONTRACT_VERSION: &str = "0.1.0";
const WS_PATH: &str = "/chat";
const HTTP_PATH: &str = "/health";

fn with_gateway_entry_identity(mut entry: DeploymentGatewayEntry) -> DeploymentGatewayEntry {
    entry.gateway_entry_identity =
        skiff_artifact_identity::gateway_entry_identity(&entry.protocol_surface)
            .expect("computed gateway entry identity");
    entry
}

fn entry_key(value: &str) -> GatewayEntryKey {
    GatewayEntryKey::parse(value).expect("gateway entry key")
}

fn http_entry() -> DeploymentGatewayEntry {
    with_gateway_entry_identity(DeploymentGatewayEntry {
        gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity::parse(
            "skiff-gateway-entry-v2:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("placeholder gateway entry identity"),
        protocol_surface: GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::TypedJson,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::HttpBody],
                request_body_schema: Some(GatewayExternalSchema::Record {
                    fields: BTreeMap::new(),
                    required: Vec::new(),
                }),
                response_schema: Some(GatewayExternalSchema::Boolean),
                stream_item_schema: None,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        },
        handler: Some(skiff_artifact_model::PackageCallableId::new(format!(
            "pkg-callable:{SERVICE_ID}:health"
        ))),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::TypedJson,
            args: vec![skiff_artifact_model::GatewayAdapterArg {
                param: "body".to_string(),
                source: GatewayAdapterSource::HttpBody,
            }],
        },
    })
}

fn websocket_connect_entry() -> DeploymentGatewayEntry {
    with_gateway_entry_identity(DeploymentGatewayEntry {
        gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity::parse(
            "skiff-gateway-entry-v2:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("placeholder gateway entry identity"),
        protocol_surface: GatewayEntryProtocolSurface {
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
        },
        handler: Some(skiff_artifact_model::PackageCallableId::new(format!(
            "pkg-callable:{SERVICE_ID}:onConnect"
        ))),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketConnect,
            args: vec![
                skiff_artifact_model::GatewayAdapterArg {
                    param: "request".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectRequest,
                },
                skiff_artifact_model::GatewayAdapterArg {
                    param: "connectionId".to_string(),
                    source: GatewayAdapterSource::WebSocketConnectionId,
                },
            ],
        },
    })
}

fn websocket_jsonrpc_entry() -> DeploymentGatewayEntry {
    with_gateway_entry_identity(DeploymentGatewayEntry {
        gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity::parse(
            "skiff-gateway-entry-v2:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("placeholder gateway entry identity"),
        protocol_surface: GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::WebSocketJsonRpc(
                GatewayWebSocketJsonRpcProtocolSurface {
                    profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                    dispatch_mode: GatewayDispatchMode::Unary,
                    external_sources: vec![GatewayAdapterSource::WebSocketJsonRpcParams],
                    params_schema: GatewayExternalSchema::Array {
                        items: Box::new(GatewayExternalSchema::String),
                    },
                    result_schema: GatewayExternalSchema::Boolean,
                },
            ),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        },
        handler: Some(skiff_artifact_model::PackageCallableId::new(format!(
            "pkg-callable:{SERVICE_ID}:status"
        ))),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketJsonRpc,
            args: vec![skiff_artifact_model::GatewayAdapterArg {
                param: "params".to_string(),
                source: GatewayAdapterSource::WebSocketJsonRpcParams,
            }],
        },
    })
}

fn deployment() -> ServiceDeployment {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: ServiceContractRef {
            service_id: SERVICE_ID.to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
        },
        deployment_revision: DeploymentRevision::new("1"),
        deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
            "placeholder",
        ),
        implementation: PackageArtifactRef {
            package_id: SERVICE_ID.to_string(),
            package_version: "0.1.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
        },
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::from([
            (entry_key("health"), http_entry()),
            (entry_key("websocket"), websocket_connect_entry()),
            (entry_key("chat.send"), websocket_jsonrpc_entry()),
        ]),
        ingress: vec![
            DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("POST".to_string()),
                    path: HTTP_PATH.to_string(),
                },
                gateway_entry_key: entry_key("health"),
            },
            DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::WebSocket,
                    method: None,
                    path: WS_PATH.to_string(),
                },
                gateway_entry_key: entry_key("websocket"),
            },
            DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::WebSocket,
                    method: Some("chat.send".to_string()),
                    path: WS_PATH.to_string(),
                },
                gateway_entry_key: entry_key("chat.send"),
            },
        ],
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "ws-live".to_string(),
            notes: BTreeMap::new(),
        },
    };
    assign_service_deployment_identity(&mut deployment).expect("assign deployment identity");
    deployment
}

fn epoch_with_deployment(
    deployment_ref: skiff_artifact_model::ServiceDeploymentRef,
) -> Arc<RoutingEpoch> {
    let mut assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    assembly.resolved_deployments = vec![deployment_ref];
    let snapshot = RuntimeConfigSnapshotValue::new(
        "ws-live",
        skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("snapshot id"),
        },
        Vec::new(),
    )
    .expect("snapshot fixture");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new(
            "ws-live",
            1,
            Arc::new(assembly),
            Arc::new(snapshot),
            catalog,
        )
        .expect("epoch fixture"),
    )
}

fn temp_artifact_root() -> (std::path::PathBuf, temp_guard::TempGuard) {
    let path = std::env::temp_dir().join(format!(
        "skiff-ws-live-surface-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let guard = temp_guard::TempGuard(path.clone());
    (path, guard)
}

mod temp_guard {
    pub struct TempGuard(pub std::path::PathBuf);

    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn mixed_http_websocket_deployment_loads_both_surfaces() {
    let deployment = deployment();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let (artifact_root, guard) = temp_artifact_root();
    let store = CanonicalArtifactStore::create(&artifact_root).expect("create artifact store");
    store
        .write_service_deployment(&deployment)
        .expect("write mixed deployment");
    drop(store);

    let epoch = epoch_with_deployment(reference);

    // HTTP surface: only the HTTP entry, no fail-closed on WS entries.
    let http = load_http_surface_view(&artifact_root, &epoch).expect("HTTP surface loads");
    assert_eq!(http.len(), 1, "HTTP view contains only the HTTP entry");
    assert!(
        http.get(&entry_key("health")).is_some(),
        "HTTP view keeps the HTTP entry"
    );
    assert!(
        http.get(&entry_key("websocket")).is_none(),
        "HTTP view must not contain the websocketConnect entry"
    );
    assert!(
        http.get(&entry_key("chat.send")).is_none(),
        "HTTP view must not contain the websocketJsonRpc entry"
    );

    // WS surface: connect binding + method table, no HTTP entry.
    let ws = load_ws_surface_view(&artifact_root, &epoch).expect("WS surface loads");
    let binding = ws
        .resolve(SERVICE_ID, WS_PATH)
        .expect("WS connect binding resolves");
    assert_eq!(binding.methods.len(), 1, "WS view keeps the JSON-RPC method");
    assert!(
        binding.methods.contains_key("chat.send"),
        "WS view keeps the chat.send method"
    );
    assert!(
        ws.resolve(SERVICE_ID, HTTP_PATH).is_none(),
        "WS view must not resolve the HTTP path"
    );
    assert_eq!(ws.len(), 1, "WS view contains only the WS connect binding");

    drop(epoch);
    drop(guard);
}

#[test]
fn http_only_deployment_keeps_previous_http_surface_behavior() {
    let mut deployment = deployment();
    deployment.gateway_entries = BTreeMap::from([(entry_key("health"), http_entry())]);
    deployment.ingress = vec![DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: HTTP_PATH.to_string(),
        },
        gateway_entry_key: entry_key("health"),
    }];
    assign_service_deployment_identity(&mut deployment).expect("assign deployment identity");
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let (artifact_root, guard) = temp_artifact_root();
    let store = CanonicalArtifactStore::create(&artifact_root).expect("create artifact store");
    store
        .write_service_deployment(&deployment)
        .expect("write http-only deployment");
    drop(store);

    let epoch = epoch_with_deployment(reference);
    let http = load_http_surface_view(&artifact_root, &epoch).expect("HTTP surface loads");
    assert_eq!(http.len(), 1);
    assert!(http.get(&entry_key("health")).is_some());
    let ws = load_ws_surface_view(&artifact_root, &epoch).expect("empty WS surface loads");
    assert_eq!(ws.len(), 0, "HTTP-only deployment has no WS binding");

    drop(epoch);
    drop(guard);
}
