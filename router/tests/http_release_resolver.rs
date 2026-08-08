//! Release-pointer HTTP ingress resolution: buildId projection, fail-closed
//! semantics (missing pointer / broken record / unmatched ingress) and
//! same-version pointer overwrite visibility.

mod http_common;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use skiff_artifact_identity::assign_service_deployment_identity;
    use skiff_artifact_model::{
        DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentGatewayEntry,
        DeploymentIngressBinding, DeploymentRevision, GatewayAdapterKind, GatewayAdapterPlan,
        GatewayAdapterSource, GatewayDispatchMode, GatewayEntryIdentity, GatewayEntryKey,
        GatewayEntryProtocolSurface, GatewayExternalErrorProjection, GatewayExternalSchema,
        GatewayHttpProtocolSurface,
        GatewayProtocolSurface, IngressProtocol, IngressSelector, PackageArtifactRef,
        PackageBuildId, PackageLocalAbiIdentity, ServiceContractRef, ServiceDeployment,
        ServiceProtocolIdentity, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
    };
    use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
    use skiff_router::http::fake::{FakeDispatchPlan, FakeHttpDispatcher};
    use skiff_router::http::{
        start_http_gateway, HttpGatewayServerOptions, StoreHttpIngressResolver,
        HttpIngressResolver,
    };
    use skiff_router::http::selector::ServiceDeploymentSelector;

    use crate::http_common::send_request;

    const SERVICE_ID: &str = "example.com/release-resolver";
    const CONTRACT_VERSION: &str = "2.0.0";
    const PROFILE: &str = "prod";

    struct Guard(PathBuf);

    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_root() -> (PathBuf, Guard) {
        static TEMP_ROOT_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "skiff-http-release-resolver-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
            TEMP_ROOT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        let guard = Guard(path.clone());
        (path, guard)
    }

    fn entry() -> DeploymentGatewayEntry {
        let protocol_surface = GatewayEntryProtocolSurface {
            protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
                adapter_kind: GatewayAdapterKind::TypedJson,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::HttpBody],
                request_body_schema: Some(GatewayExternalSchema::String),
                response_schema: Some(GatewayExternalSchema::String),
                stream_item_schema: None,
            }),
            external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
        };
        DeploymentGatewayEntry {
            gateway_entry_identity: GatewayEntryIdentity::parse(
                skiff_artifact_identity::gateway_entry_identity(&protocol_surface)
                    .expect("entry identity")
                    .as_str(),
            )
            .expect("identity"),
            protocol_surface,
            handler: Some(skiff_artifact_model::PackageCallableId::new(format!(
                "pkg-callable:{SERVICE_ID}:greet"
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
            close_handler: None,
            close_adapter_plan: None,
        }
    }

    fn deployment(revision: &str) -> ServiceDeployment {
        let mut deployment = ServiceDeployment {
            schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
            contract: ServiceContractRef {
                service_id: SERVICE_ID.to_string(),
                contract_version: CONTRACT_VERSION.to_string(),
                service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
            },
            deployment_revision: DeploymentRevision::new(revision.to_string()),
            deployment_artifact_identity: DeploymentArtifactIdentity::new("placeholder"),
            implementation: PackageArtifactRef {
                package_id: SERVICE_ID.to_string(),
                package_version: "0.1.0".to_string(),
                package_build_id: PackageBuildId::new("build"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            },
            operation_bindings: Vec::new(),
            package_bindings: Vec::new(),
            service_selectors: Vec::new(),
            gateway_entries: BTreeMap::from([(entry_key("greet"), entry())]),
            ingress: vec![DeploymentIngressBinding {
                selector: IngressSelector {
                    protocol: IngressProtocol::Http,
                    method: Some("POST".to_string()),
                    path: "/greet".to_string(),
                },
                gateway_entry_key: entry_key("greet"),
            }],
            diagnostic_text: DeploymentDiagnosticText {
                display_name: "release-resolver".to_string(),
                notes: BTreeMap::new(),
            },
        };
        assign_service_deployment_identity(&mut deployment).expect("assign deployment identity");
        deployment
    }

    fn entry_key(value: &str) -> GatewayEntryKey {
        GatewayEntryKey::parse(value).expect("gateway entry key")
    }

    fn selector() -> ServiceDeploymentSelector {
        ServiceDeploymentSelector {
            service_id: SERVICE_ID.to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
        }
    }

    fn store_with_pointer(deployment: &ServiceDeployment) -> (CanonicalArtifactStore, Guard) {
        let (root, guard) = temp_root();
        let store = CanonicalArtifactStore::create(&root).expect("create artifact store");
        store
            .write_service_deployment(deployment)
            .expect("write deployment record");
        let reference = skiff_artifact_identity::service_deployment_ref(deployment);
        let pointer = ReleasePointer::new(PROFILE, reference).expect("release pointer");
        store
            .write_release_pointer(&pointer)
            .expect("write release pointer");
        (store, guard)
    }

    fn resolver(store: &CanonicalArtifactStore) -> StoreHttpIngressResolver {
        StoreHttpIngressResolver::new_with_live_artifact_store(
            Arc::new(skiff_router::http::HttpGatewaySurfaceView::default()),
            store.clone(),
            PROFILE,
        )
    }

    fn release_error(error: &skiff_router::http::HttpError) -> (u16, &str) {
        (error.status, error.code.as_str())
    }

    #[test]
    fn resolved_binding_carries_release_build_id_and_surface() {
        let deployment = deployment("1");
        let (store, _guard) = store_with_pointer(&deployment);
        let resolver = resolver(&store);

        let binding = resolver
            .resolve(&selector(), "POST", "/greet")
            .expect("release ingress resolves");
        assert_eq!(binding.deployment.service_id, SERVICE_ID);
        assert_eq!(binding.deployment.contract_version, CONTRACT_VERSION);
        assert_eq!(
            binding.build_id,
            binding.deployment.deployment_artifact_identity.as_str()
        );
        assert_eq!(
            binding.build_id,
            deployment.deployment_artifact_identity.as_str()
        );
        assert_eq!(binding.gateway_entry_key.as_str(), "greet");
        assert_eq!(
            binding.gateway_entry_identity.as_str(),
            entry().gateway_entry_identity.as_str()
        );
        assert_eq!(binding.selector.method.as_deref(), Some("POST"));
        assert_eq!(binding.selector.path, "/greet");
        assert_eq!(
            binding.mode,
            skiff_router::http::HttpDispatchMode::Unary
        );
    }

    #[test]
    fn missing_release_pointer_fails_closed_with_release_not_found() {
        let deployment = deployment("1");
        let (root, _guard) = temp_root();
        let store = CanonicalArtifactStore::create(&root).expect("create artifact store");
        store
            .write_service_deployment(&deployment)
            .expect("write deployment record");
        let resolver = resolver(&store);

        let error = resolver
            .resolve(&selector(), "POST", "/greet")
            .expect_err("unset release pointer must fail closed");
        let (status, code) = release_error(&error);
        assert_eq!((status, code), (404, "ReleaseNotFound"));
    }

    #[test]
    fn broken_pointer_target_fails_closed_as_internal() {
        let deployment = deployment("1");
        let (store, guard) = store_with_pointer(&deployment);
        let record_path = skiff_artifact_identity::ServiceDeploymentRecordPath::new(
            &skiff_artifact_identity::service_deployment_ref(&deployment),
        )
        .expect("record path");
        std::fs::remove_file(store.root().join(record_path.as_relative_path().as_path()))
            .expect("remove deployment record");
        let resolver = resolver(&store);

        let error = resolver
            .resolve(&selector(), "POST", "/greet")
            .expect_err("broken pointer target must fail closed");
        let (status, code) = release_error(&error);
        assert_eq!((status, code), (500, "InternalGatewayError"));
        assert!(error.message.contains("release resolution failed"));
        drop(guard);
    }

    #[test]
    fn unmatched_method_or_path_is_assembly_ingress_not_found() {
        let deployment = deployment("1");
        let (store, _guard) = store_with_pointer(&deployment);
        let resolver = resolver(&store);

        let error = resolver
            .resolve(&selector(), "GET", "/greet")
            .expect_err("wrong method fails closed");
        assert_eq!(release_error(&error), (404, "AssemblyIngressNotFound"));
        let error = resolver
            .resolve(&selector(), "POST", "/nope")
            .expect_err("unknown path fails closed");
        assert_eq!(release_error(&error), (404, "AssemblyIngressNotFound"));
    }

    #[test]
    fn same_version_pointer_overwrite_resolves_new_build_id() {
        let v1 = deployment("1");
        let (store, _guard) = store_with_pointer(&v1);
        let resolver = resolver(&store);

        let first = resolver
            .resolve(&selector(), "POST", "/greet")
            .expect("v1 resolves");
        assert_eq!(first.build_id, v1.deployment_artifact_identity.as_str());

        let mut v2 = v1.clone();
        v2.deployment_revision = DeploymentRevision::new("2");
        assign_service_deployment_identity(&mut v2).expect("assign v2 identity");
        assert_ne!(
            v2.deployment_artifact_identity,
            v1.deployment_artifact_identity
        );
        store
            .write_service_deployment(&v2)
            .expect("write v2 deployment");
        let reference_v2 = skiff_artifact_identity::service_deployment_ref(&v2);
        let pointer_v2 = ReleasePointer::new(PROFILE, reference_v2).expect("v2 pointer");
        store
            .write_release_pointer(&pointer_v2)
            .expect("overwrite release pointer");

        let second = resolver
            .resolve(&selector(), "POST", "/greet")
            .expect("v2 resolves after pointer overwrite");
        assert_eq!(second.build_id, v2.deployment_artifact_identity.as_str());
        assert_ne!(second.build_id, first.build_id);
    }

    #[test]
    fn has_ingress_path_follows_release_resolution() {
        let deployment = deployment("1");
        let (store, _guard) = store_with_pointer(&deployment);
        let resolver = resolver(&store);

        assert!(resolver.has_ingress_path(&selector(), "/greet"));
        assert!(!resolver.has_ingress_path(&selector(), "/other"));
        let unknown_selector = ServiceDeploymentSelector {
            service_id: "example.com/unknown".to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
        };
        assert!(!resolver.has_ingress_path(&unknown_selector, "/greet"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn socket_gateway_returns_404_release_not_found() {
        let deployment = deployment("1");
        let (store, _guard) = store_with_pointer(&deployment);
        let server = start_http_gateway(
            HttpGatewayServerOptions::new("127.0.0.1:0".parse().expect("bind"), 1024 * 1024, 4096),
            Arc::new(resolver(&store)),
            Arc::new(FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryOk {
                status: 200,
                headers: vec![],
                payload: bytes::Bytes::new(),
            }])),
        )
        .await
        .expect("start http gateway");
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", "example.com/never-released"),
            ("x-skiff-version", CONTRACT_VERSION),
        ];
        let response = send_request(addr, "POST", "/greet", &headers, b"{}")
            .expect("missing release roundtrip");
        assert_eq!(response.status, 404);
        let body: serde_json::Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "ReleaseNotFound");
        server.shutdown().await.expect("shutdown");
    }
}
