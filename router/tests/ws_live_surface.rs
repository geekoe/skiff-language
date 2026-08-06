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
    DeploymentDiagnosticText, DeploymentGatewayEntry, DeploymentIngressBinding, DeploymentRevision,
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayExternalErrorProjection,
    GatewayExternalSchema, GatewayHttpProtocolSurface, GatewayIngressBinding,
    GatewayProtocolSurface, GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketJsonRpcProtocolSurface, GatewayWebSocketRpcProfile,
    GatewayWebSocketShapeVersion, IngressProtocol, IngressSelector, PackageArtifactRef,
    PackageBuildId, PackageLocalAbiIdentity, ServiceContractRef, ServiceDeployment,
    ServiceProtocolIdentity, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_router::http::HttpIngressResolver;
use skiff_router::supervisor::http::load_http_surface_view;
use skiff_router::supervisor::ws::load_ws_surface_view;

const SERVICE_ID: &str = "test.skiff/router-rust-ws-live";
const CONTRACT_VERSION: &str = "0.1.0";
const WS_PATH: &str = "/chat";
const HTTP_PATH: &str = "/health";

#[cfg(test)]
mod tests {
    use super::*;

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

    fn write_deployment_with_pointer(
        root: &std::path::Path,
        deployment: &ServiceDeployment,
        profile: &str,
    ) -> skiff_artifact_model::ServiceDeploymentRef {
        let store = CanonicalArtifactStore::create(root).expect("create artifact store");
        store
            .write_service_deployment(deployment)
            .expect("write deployment");
        let reference = skiff_artifact_identity::service_deployment_ref(deployment);
        let pointer = ReleasePointer::new(profile, reference.clone()).expect("release pointer");
        store
            .write_release_pointer(&pointer)
            .expect("write release pointer");
        reference
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
        let (artifact_root, guard) = temp_artifact_root();
        let reference = write_deployment_with_pointer(&artifact_root, &deployment, "ws-live");

        // HTTP surface: only the HTTP entry, no fail-closed on WS entries.
        let http = load_http_surface_view(&artifact_root, "ws-live").expect("HTTP surface loads");
        assert_eq!(http.len(), 1, "HTTP view contains only the HTTP entry");
        assert!(
            http.get(&reference, &entry_key("health")).is_some(),
            "HTTP view keeps the HTTP entry"
        );
        assert!(
            http.get(&reference, &entry_key("websocket")).is_none(),
            "HTTP view must not contain the websocketConnect entry"
        );
        assert!(
            http.get(&reference, &entry_key("chat.send")).is_none(),
            "HTTP view must not contain the websocketJsonRpc entry"
        );

        // WS surface: connect binding + method table, no HTTP entry.
        let ws = load_ws_surface_view(&artifact_root, "ws-live").expect("WS surface loads");
        let binding = ws
            .resolve(SERVICE_ID, WS_PATH)
            .expect("WS connect binding resolves");
        assert_eq!(
            binding.methods.len(),
            1,
            "WS view keeps the JSON-RPC method"
        );
        assert!(
            binding.methods.contains_key("chat.send"),
            "WS view keeps the chat.send method"
        );
        assert!(
            ws.resolve(SERVICE_ID, HTTP_PATH).is_none(),
            "WS view must not resolve the HTTP path"
        );
        assert_eq!(ws.len(), 1, "WS view contains only the WS connect binding");

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
        let (artifact_root, guard) = temp_artifact_root();
        let reference = write_deployment_with_pointer(&artifact_root, &deployment, "ws-live");

        let http = load_http_surface_view(&artifact_root, "ws-live").expect("HTTP surface loads");
        assert_eq!(http.len(), 1);
        assert!(http.get(&reference, &entry_key("health")).is_some());
        let ws = load_ws_surface_view(&artifact_root, "ws-live").expect("empty WS surface loads");
        assert_eq!(ws.len(), 0, "HTTP-only deployment has no WS binding");

        drop(guard);
    }

    #[test]
    fn release_pointer_switch_rebuilds_ws_surface_with_current_deployment() {
        // F9 regression (M4 form): the WS connect admission resolves its
        // binding from the live release pointer table. A pointer switch
        // publishes a new deployment revision; a startup-loaded static
        // surface would keep the stale revision and the connect candidate
        // query would fail with no eligible runtime (WS connects 503 until
        // router restart).

        let deployment_v1 = deployment();
        let (artifact_root, guard) = temp_artifact_root();
        let ref_v1 = write_deployment_with_pointer(&artifact_root, &deployment_v1, "ws-live");
        let startup_surface =
            load_ws_surface_view(&artifact_root, "ws-live").expect("startup surface");

        // Pointer switch: same service/path, new deployment revision.
        let mut deployment_v2 = deployment_v1.clone();
        deployment_v2.deployment_revision = DeploymentRevision::new("2");
        assign_service_deployment_identity(&mut deployment_v2)
            .expect("assign v2 deployment identity");
        let ref_v2 = write_deployment_with_pointer(&artifact_root, &deployment_v2, "ws-live");

        // The startup surface keeps the stale revision.
        let stale_binding = startup_surface
            .resolve(SERVICE_ID, WS_PATH)
            .expect("startup binding resolves");
        assert_eq!(
            stale_binding.deployment.deployment_revision, ref_v1.deployment_revision,
            "startup surface is pinned to the old deployment"
        );

        // The live rebuild resolves the current deployment revision.
        let live_surface =
            load_ws_surface_view(&artifact_root, "ws-live").expect("live WS surface rebuilds");
        let current_binding = live_surface
            .resolve(SERVICE_ID, WS_PATH)
            .expect("current binding resolves");
        assert_eq!(
            current_binding.deployment.deployment_revision, ref_v2.deployment_revision,
            "pointer switch must not leave the connect binding on a stale deployment revision"
        );
        assert_eq!(current_binding.methods.len(), 1);
        assert!(current_binding.connect_handler);

        drop(guard);
    }

    #[test]
    fn duplicate_gateway_entry_key_across_deployments_is_deployment_scoped() {
        // Real agine stack regression (E-chat gate): agine.ai/aihub and
        // agine.ai/codex-relay both publish `v1ModelsGet` (GET /v1/models).
        // The HTTP surface must be keyed by (deployment, gateway entry key)
        // so Router composition succeeds and each service selector resolves
        // its own binding.
        const AIHUB_SERVICE: &str = "test.skiff/aihub";
        const RELAY_SERVICE: &str = "test.skiff/codex-relay";

        let aihub = models_deployment(AIHUB_SERVICE);
        let relay = models_deployment(RELAY_SERVICE);
        let aihub_ref = skiff_artifact_identity::service_deployment_ref(&aihub);
        let relay_ref = skiff_artifact_identity::service_deployment_ref(&relay);
        let (artifact_root, guard) = temp_artifact_root();
        let store = CanonicalArtifactStore::create(&artifact_root).expect("create artifact store");
        store
            .write_service_deployment(&aihub)
            .expect("write aihub deployment");
        store
            .write_service_deployment(&relay)
            .expect("write relay deployment");
        let aihub_pointer =
            skiff_deployment::storage::ReleasePointer::new("surface-dup", aihub_ref.clone())
                .expect("aihub release pointer");
        let relay_pointer =
            skiff_deployment::storage::ReleasePointer::new("surface-dup", relay_ref.clone())
                .expect("relay release pointer");
        store
            .write_release_pointer(&aihub_pointer)
            .expect("write aihub release pointer");
        store
            .write_release_pointer(&relay_pointer)
            .expect("write relay release pointer");

        let http = load_http_surface_view(&artifact_root, "surface-dup")
            .expect("HTTP surface loads with duplicate gateway entry keys");
        assert_eq!(
            http.len(),
            2,
            "HTTP view keeps both deployments for the shared key"
        );
        assert!(
            http.get(&aihub_ref, &entry_key("v1ModelsGet")).is_some(),
            "aihub surface exists"
        );
        assert!(
            http.get(&relay_ref, &entry_key("v1ModelsGet")).is_some(),
            "relay surface exists"
        );

        let resolver =
            skiff_router::http::ingress::StoreHttpIngressResolver::new_with_live_artifact_store(
                Arc::new(http),
                store.clone(),
                "surface-dup",
            );
        let aihub_selector = skiff_router::http::selector::ServiceDeploymentSelector {
            service_id: AIHUB_SERVICE.to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
        };
        let relay_selector = skiff_router::http::selector::ServiceDeploymentSelector {
            service_id: RELAY_SERVICE.to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
        };
        let aihub_binding = resolver
            .resolve(&aihub_selector, "GET", "/v1/models")
            .expect("aihub selector resolves the shared key");
        assert_eq!(aihub_binding.deployment.service_id, AIHUB_SERVICE);
        assert_eq!(aihub_binding.gateway_entry_key.as_str(), "v1ModelsGet");

        let relay_binding = resolver
            .resolve(&relay_selector, "GET", "/v1/models")
            .expect("relay selector resolves the shared key");
        assert_eq!(relay_binding.deployment.service_id, RELAY_SERVICE);
        assert_eq!(relay_binding.gateway_entry_key.as_str(), "v1ModelsGet");

        assert!(
            resolver
                .resolve(&aihub_selector, "GET", "/v1/other")
                .is_err(),
            "unmatched path still fails closed"
        );

        drop(guard);
    }

    fn models_deployment(service_id: &str) -> ServiceDeployment {
        let mut deployment = ServiceDeployment {
            schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
            contract: ServiceContractRef {
                service_id: service_id.to_string(),
                contract_version: CONTRACT_VERSION.to_string(),
                service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
            },
            deployment_revision: DeploymentRevision::new("1"),
            deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
                "placeholder",
            ),
            implementation: PackageArtifactRef {
                package_id: service_id.to_string(),
                package_version: "0.1.0".to_string(),
                package_build_id: PackageBuildId::new("build"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            },
            operation_bindings: Vec::new(),
            package_bindings: Vec::new(),
            service_selectors: Vec::new(),
            gateway_entries: BTreeMap::from([(entry_key("v1ModelsGet"), http_entry())]),
            ingress: vec![DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("GET".to_string()),
                    path: "/v1/models".to_string(),
                },
                gateway_entry_key: entry_key("v1ModelsGet"),
            }],
            diagnostic_text: DeploymentDiagnosticText {
                display_name: "duplicate-v1ModelsGet".to_string(),
                notes: BTreeMap::new(),
            },
        };
        assign_service_deployment_identity(&mut deployment).expect("assign deployment identity");
        deployment
    }
}
