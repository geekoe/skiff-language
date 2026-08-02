//! W-http real-socket probes: trusted selector validation, service-scoped
//! ingress, CORS preflight/service-managed/platform error and test-dispatch
//! correlation isolation.

mod http_common;

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use bytes::Bytes;
    use serde_json::Value;
    use skiff_router::http::fake::{FakeDispatchPlan, FakeHttpDispatcher};
    use skiff_router::http::{start_http_gateway, HttpGatewayServer, HttpGatewayServerOptions};

    use crate::http_common;
    use crate::http_common::{
        fixture_epoch, fixture_resolver, send_request, service_headers, CONTRACT_VERSION,
        SERVICE_ID,
    };

    async fn start_server(
        dispatcher: FakeHttpDispatcher,
    ) -> (HttpGatewayServer, FakeHttpDispatcher) {
        let dispatcher = Arc::new(dispatcher);
        let server = start_http_gateway(
            HttpGatewayServerOptions::new("127.0.0.1:0".parse().expect("bind"), 1024 * 1024, 4096),
            fixture_epoch(),
            fixture_resolver(),
            dispatcher.clone(),
        )
        .await
        .expect("start http gateway");
        (server, (*dispatcher).clone())
    }

    fn ok_plan() -> FakeDispatchPlan {
        FakeDispatchPlan::UnaryOk {
            status: 200,
            headers: vec![("x-ok".to_string(), "1".to_string())],
            payload: Bytes::new(),
        }
    }

    fn ok_dispatcher() -> FakeHttpDispatcher {
        FakeHttpDispatcher::new(vec![ok_plan()])
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_missing_service_selector_is_rejected() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let response = send_request(
            addr,
            "POST",
            "/items",
            &[("x-skiff-version", CONTRACT_VERSION)],
            b"{}",
        )
        .expect("missing service");
        assert_eq!(response.status, 400);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "ServiceSelectorRequired");
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_missing_version_selector_is_rejected() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let response = send_request(
            addr,
            "POST",
            "/items",
            &[("x-skiff-service", SERVICE_ID)],
            b"{}",
        )
        .expect("missing version");
        assert_eq!(response.status, 400);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "VersionSelectorRequired");
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_duplicate_selector_header_is_rejected() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-service", "example.com/other"),
            ("x-skiff-version", CONTRACT_VERSION),
        ];
        let response =
            send_request(addr, "POST", "/items", &headers, b"{}").expect("duplicate selector");
        assert_eq!(response.status, 400);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "ServiceSelectorInvalid");
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_version_release_conflict_is_rejected() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-version", CONTRACT_VERSION),
            ("x-skiff-release", "2.0.0"),
        ];
        let response =
            send_request(addr, "POST", "/items", &headers, b"{}").expect("version conflict");
        assert_eq!(response.status, 400);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "InvalidVersionHeader");
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_release_header_alias_selects_version() {
        let (server, mut fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-release", CONTRACT_VERSION),
        ];
        let response =
            send_request(addr, "POST", "/items", &headers, b"{}").expect("release alias");
        assert_eq!(response.status, 200);
        fake.wait_for_requests(1).await;
        assert_eq!(fake.recorded_requests().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unknown_ingress_path_is_404() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let response = send_request(addr, "POST", "/nope", &service_headers(), b"{}")
            .expect("unknown ingress");
        assert_eq!(response.status, 404);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "AssemblyIngressNotFound");
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_wrong_method_is_404() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let response =
            send_request(addr, "GET", "/items", &service_headers(), b"").expect("wrong method");
        assert_eq!(response.status, 404);
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_automatic_cors_preflight_returns_204() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-version", CONTRACT_VERSION),
            ("origin", "https://client.example"),
            ("access-control-request-method", "POST"),
            (
                "access-control-request-headers",
                "x-test, x-test, bad header",
            ),
        ];
        let response = send_request(addr, "OPTIONS", "/events", &headers, b"").expect("preflight");
        assert_eq!(response.status, 204);
        assert_eq!(
            response.header("access-control-allow-origin"),
            Some("https://client.example")
        );
        assert_eq!(
            response.header("access-control-allow-credentials"),
            Some("true")
        );
        let allow_headers = response
            .header("access-control-allow-headers")
            .expect("allow headers");
        assert!(allow_headers.contains("x-test"));
        assert!(!allow_headers.contains("bad header"));
        assert_eq!(response.header("access-control-max-age"), Some("600"));
        assert!(response
            .header("vary")
            .expect("vary")
            .to_ascii_lowercase()
            .contains("origin"));
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_explicit_options_ingress_is_service_managed() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryOk {
            status: 200,
            headers: vec![(
                "access-control-allow-origin".to_string(),
                "https://service.example".to_string(),
            )],
            payload: Bytes::from_static(b"managed"),
        }]);
        let (server, mut fake) = start_server(fake).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-version", CONTRACT_VERSION),
            ("origin", "https://client.example"),
            ("access-control-request-method", "POST"),
        ];
        let response = send_request(addr, "OPTIONS", "/items", &headers, b"")
            .expect("service-managed options");
        assert_eq!(response.status, 200);
        assert_eq!(response.body_text(), "managed");
        // The runtime CORS header is preserved; no automatic 204/headers added.
        assert_eq!(
            response.header("access-control-allow-origin"),
            Some("https://service.example")
        );
        fake.wait_for_requests(1).await;
        let recorded = fake.recorded_requests();
        assert_eq!(recorded[0].header.routing.ingress.method, "OPTIONS");
        assert_eq!(
            recorded[0].header.routing.gateway_entry_identity.as_str(),
            http_common::GATEWAY_ITEMS_OPTIONS_IDENTITY
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_platform_error_carries_automatic_cors_headers() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("origin", "https://client.example"),
        ];
        let response =
            send_request(addr, "POST", "/items", &headers, b"{}").expect("platform error");
        assert_eq!(response.status, 400);
        assert_eq!(
            response.header("access-control-allow-origin"),
            Some("https://client.example")
        );
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_test_case_correlation_is_isolated_from_http_metadata() {
        let fake = FakeHttpDispatcher::new(vec![ok_plan()]);
        let (server, mut fake) = start_server(fake).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-version", CONTRACT_VERSION),
            ("x-skiff-test-case-capability", "cap-1"),
            ("x-skiff-test-case-parent-request-id", "parent-1"),
            ("x-normal", "kept"),
        ];
        let response =
            send_request(addr, "POST", "/items", &headers, b"{}").expect("test correlation");
        assert_eq!(response.status, 200);
        fake.wait_for_requests(1).await;
        let recorded = fake.recorded_requests();
        assert_eq!(recorded.len(), 1);
        let header = &recorded[0].header;
        assert!(header.test_effects_enabled);
        assert_eq!(header.test_case_capability.as_deref(), Some("cap-1"));
        assert_eq!(
            header.test_case_parent_request_id.as_deref(),
            Some("parent-1")
        );
        assert!(!header
            .http_request
            .headers
            .iter()
            .any(|item| item.name == "x-skiff-test-case-capability"));
        assert!(!header
            .http_request
            .headers
            .iter()
            .any(|item| item.name == "x-skiff-test-case-parent-request-id"));
        assert!(header
            .http_request
            .headers
            .iter()
            .any(|item| item.name == "x-normal" && item.value == "kept"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_test_case_correlation_without_parent_is_rejected() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-version", CONTRACT_VERSION),
            ("x-skiff-test-case-capability", "cap-1"),
        ];
        let response =
            send_request(addr, "POST", "/items", &headers, b"{}").expect("invalid correlation");
        assert_eq!(response.status, 400);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "InvalidTestCaseCorrelation");
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_automatic_preflight_rejects_test_case_correlation() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let headers = vec![
            ("x-skiff-service", SERVICE_ID),
            ("x-skiff-version", CONTRACT_VERSION),
            ("origin", "https://client.example"),
            ("access-control-request-method", "POST"),
            ("x-skiff-test-case-capability", "cap-1"),
            ("x-skiff-test-case-parent-request-id", "parent-1"),
        ];
        let response =
            send_request(addr, "OPTIONS", "/events", &headers, b"").expect("preflight correlation");
        assert_eq!(response.status, 400);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "InvalidTestCaseCorrelation");
        assert_eq!(fake.recorded_requests().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_control_endpoint_is_not_served_by_public_gateway() {
        let (server, fake) = start_server(ok_dispatcher()).await;
        let addr = server.addr();
        let response = send_request(addr, "GET", "/__router/health", &service_headers(), b"")
            .expect("control endpoint");
        assert_eq!(response.status, 404);
        let body: Value = serde_json::from_slice(&response.body).expect("json");
        assert_eq!(body["error"]["code"], "ControlEndpointNotFound");
        assert_eq!(fake.recorded_requests().len(), 0);
    }
}
