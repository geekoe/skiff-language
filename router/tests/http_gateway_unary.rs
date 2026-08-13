//! W-http real-socket probes: unary HTTP → fake dispatcher mapping.

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
        fixture_deployment_identity, fixture_resolver, send_request, service_headers,
        CONTRACT_VERSION, DEPLOYMENT_REVISION, SERVICE_ID,
    };

    async fn start_server(
        dispatcher: FakeHttpDispatcher,
        max_response_bytes: usize,
    ) -> (HttpGatewayServer, FakeHttpDispatcher) {
        let dispatcher = Arc::new(dispatcher);
        let server = start_http_gateway(
            HttpGatewayServerOptions::new(
                "127.0.0.1:0".parse().expect("bind"),
                1024 * 1024,
                max_response_bytes,
            ),
            fixture_resolver(),
            dispatcher.clone(),
        )
        .await
        .expect("start http gateway");
        (server, (*dispatcher).clone())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unary_roundtrip_carries_trusted_selector_and_opaque_body() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryOk {
            status: 200,
            headers: vec![
                ("content-type".to_string(), "text/plain".to_string()),
                ("x-service".to_string(), "echo".to_string()),
            ],
            payload: Bytes::from_static(b"hello"),
        }]);
        let (server, mut fake) = start_server(fake, 4096).await;
        let addr = server.addr();
        let response = send_request(
            addr,
            "POST",
            "/items?q=1",
            &service_headers(),
            b"{\"msg\":\"hi\"}",
        )
        .expect("unary roundtrip");
        assert_eq!(response.status, 200);
        assert_eq!(response.body_text(), "hello");
        assert_eq!(response.header("content-type"), Some("text/plain"));
        assert_eq!(response.header("x-service"), Some("echo"));

        fake.wait_for_requests(1).await;
        let recorded = fake.recorded_requests();
        assert_eq!(recorded.len(), 1);
        let header = &recorded[0].header;
        assert_eq!(header.mode, "unary");
        assert_eq!(header.request_id, header.request_id);
        assert_eq!(header.routing.kind, "runtimeAssembly");
        assert!(
            header.routing.assembly_identity.is_none(),
            "M4: router does not fill the assembly identity tuple"
        );
        assert!(
            header.routing.assembly_generation.is_none(),
            "M4: router does not fill the generation tuple"
        );
        assert_eq!(header.routing.deployment.service_id, SERVICE_ID);
        assert_eq!(header.routing.deployment.contract_version, CONTRACT_VERSION);
        assert_eq!(
            header.routing.deployment.deployment_revision.as_str(),
            DEPLOYMENT_REVISION
        );
        assert_eq!(
            header
                .routing
                .deployment
                .deployment_artifact_identity
                .as_str(),
            fixture_deployment_identity()
        );
        assert_eq!(
            header.routing.build_id.as_deref(),
            Some(fixture_deployment_identity().as_str()),
            "request.start routing buildId must be the release-resolved ref identity"
        );
        assert_eq!(
            header.routing.gateway_entry_identity.as_str(),
            http_common::fixture_entry_identity("items").as_str()
        );
        assert!(matches!(
            header.routing.ingress.protocol,
            skiff_runtime_transport::protocol::BytecodeRequestIngressProtocol::Http
        ));
        assert_eq!(header.routing.ingress.method, "POST");
        assert_eq!(header.routing.ingress.path, "/items");
        assert_eq!(recorded[0].payload_bytes.as_ref(), b"{\"msg\":\"hi\"}");
        let deadline = header.deadline.as_ref().expect("deadline present");
        assert_eq!(deadline.timeout_ms, 120_000);
        assert!(deadline.expires_at.ends_with('Z'));
        assert_eq!(header.http_request.method, "POST");
        assert!(header.http_request.url.starts_with("http://127.0.0.1:"));
        assert_eq!(header.http_request.path, "/items");
        assert_eq!(header.http_request.query.len(), 1);
        assert_eq!(header.http_request.query[0].name, "q");
        assert_eq!(header.http_request.query[0].value, "1");
        assert!(header
            .http_request
            .headers
            .iter()
            .any(|item| item.name == "x-skiff-service"));
        assert!(!header.test_effects_enabled);
        assert!(header.test_case_capability.is_none());
        assert_eq!(fake.recorded_cancels().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unary_control_error_maps_to_platform_json() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryControlError {
            code: "std.service.ProviderUnavailableError".to_string(),
            message: "runtime unavailable".to_string(),
            status: Some(400),
            details: Some(serde_json::json!({ "hint": "retry" })),
        }]);
        let (server, fake) = start_server(fake, 4096).await;
        let addr = server.addr();
        let response = send_request(addr, "POST", "/items", &service_headers(), b"{}")
            .expect("control error roundtrip");
        assert_eq!(response.status, 400);
        let body: Value = serde_json::from_slice(&response.body).expect("json body");
        assert_eq!(
            body["error"]["code"],
            "std.service.ProviderUnavailableError"
        );
        assert_eq!(body["error"]["message"], "runtime unavailable");
        assert_eq!(body["error"]["details"]["hint"], "retry");
        assert_eq!(fake.recorded_cancels().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unary_fixed_service_error_projects_trace_ids() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryFixedServiceError {
            trace_id: "trace-err".to_string(),
            error_id: "error-1".to_string(),
        }]);
        let (server, fake) = start_server(fake, 4096).await;
        let addr = server.addr();
        let response = send_request(addr, "POST", "/items", &service_headers(), b"{}")
            .expect("fixed service error roundtrip");
        assert_eq!(response.status, 500);
        let body: Value = serde_json::from_slice(&response.body).expect("json body");
        assert_eq!(body["error"]["code"], "FixedServiceError");
        assert_eq!(body["error"]["message"], "Service request failed");
        assert_eq!(body["error"]["details"]["traceId"], "trace-err");
        assert_eq!(body["error"]["details"]["errorId"], "error-1");
        assert_eq!(fake.recorded_cancels().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unary_response_above_ceiling_is_rejected() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryOk {
            status: 200,
            headers: vec![],
            payload: Bytes::from_static(b"payload-is-too-large"),
        }]);
        let (server, fake) = start_server(fake, 8).await;
        let addr = server.addr();
        let response = send_request(addr, "POST", "/items", &service_headers(), b"{}")
            .expect("ceiling rejection");
        assert_eq!(response.status, 502);
        let body: Value = serde_json::from_slice(&response.body).expect("json body");
        assert_eq!(body["error"]["code"], "ResponseTooLarge");
        assert_eq!(fake.recorded_cancels().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_unary_runtime_cancel_is_provider_unavailable() {
        let fake = FakeHttpDispatcher::new(vec![FakeDispatchPlan::UnaryRuntimeCancel]);
        let (server, fake) = start_server(fake, 4096).await;
        let addr = server.addr();
        let response = send_request(addr, "POST", "/items", &service_headers(), b"{}")
            .expect("runtime cancel roundtrip");
        assert_eq!(response.status, 503);
        let body: Value = serde_json::from_slice(&response.body).expect("json body");
        assert_eq!(
            body["error"]["code"],
            "std.service.ProviderUnavailableError"
        );
        // Runtime-initiated cancel sends no Router→Runtime cancel frame.
        assert_eq!(fake.recorded_cancels().len(), 0);
    }
}
