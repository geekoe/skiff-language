use std::cell::Cell;

use super::*;

#[test]
fn activation_and_business_clients_have_independent_timeout_budgets() {
    let router_default_prepare_timeout = Duration::from_millis(120_000);

    assert_eq!(ACTIVATION_HTTP_TIMEOUT, Duration::from_millis(150_000));
    assert!(
        ACTIVATION_HTTP_TIMEOUT > router_default_prepare_timeout,
        "the Router must decide a default prepare timeout before the activation client disconnects"
    );
    assert_eq!(BUSINESS_HTTP_TIMEOUT, Duration::from_millis(30_000));
    assert_ne!(ACTIVATION_HTTP_TIMEOUT, BUSINESS_HTTP_TIMEOUT);
}

#[test]
fn activation_and_dispatch_call_sites_use_separate_budgets() {
    let source = include_str!("../../runtime_execution.rs");

    assert_eq!(
        source
            .matches("deadline_after(ACTIVATION_HTTP_TIMEOUT)?")
            .count(),
        1
    );
    assert_eq!(
        source
            .matches("deadline_after(BUSINESS_HTTP_TIMEOUT)?")
            .count(),
        1
    );
    assert!(!source.contains("deadline_after(HTTP_TIMEOUT)"));
}

#[test]
fn client_deadline_overflow_fails_closed() {
    let error = deadline_after_from(Instant::now(), Duration::MAX).unwrap_err();

    assert!(matches!(
        error,
        CanonicalFixtureError::InvalidInput(message) if message == "HTTP deadline overflow"
    ));
}

#[test]
fn activation_request_preserves_dev_target_environment() {
    let assembly = RuntimeAssemblyRef {
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(test_support::ASSEMBLY_B),
    };

    let body = activation_request_body("dev", "activation-dev", 7, &assembly).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        body,
        serde_json::json!({
            "schemaVersion": "skiff-assembly-activation-request-v1",
            "environment": "dev",
            "activationId": "activation-dev",
            "expectedGeneration": 7,
            "assembly": {
                "assemblyIdentity": test_support::ASSEMBLY_B,
            },
        })
    );
}

#[test]
fn package_test_control_body_is_the_exact_f385_http_request() {
    let entrypoint = package_test_entrypoint();
    let assembly = RuntimeAssemblyRef {
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(test_support::ASSEMBLY_B),
    };

    let body =
        package_test_dispatch_body("http://127.0.0.1:46123", &assembly, 7, &entrypoint).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        body,
        serde_json::json!({
            "kind": "test",
            "routing": {
                "kind": "runtimeAssembly",
                "assemblyIdentity": test_support::ASSEMBLY_B,
                "assemblyGeneration": 7,
                "deployment": {
                    "serviceId": "test.skiff/package/example",
                    "contractVersion": "1.0.0",
                    "deploymentRevision": "test-control",
                    "deploymentArtifactIdentity": concat!(
                        "skiff-deployment-artifact-v4:sha256:",
                        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                    ),
                },
                "gatewayEntryIdentity": concat!(
                    "skiff-gateway-entry-v2:sha256:",
                    "b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305"
                ),
                "ingress": {
                    "protocol": "http",
                    "method": "POST",
                    "path": "/__skiff/package-test/0",
                },
            },
            "mode": "unary",
            "httpRequest": {
                "method": "POST",
                "url": "http://127.0.0.1:46123/__skiff/package-test/0",
                "path": "/__skiff/package-test/0",
                "query": [],
                "headers": [{
                    "name": "content-type",
                    "value": "application/json",
                }],
            },
            "payloadBase64": "bnVsbA==",
            "timeoutMs": 30_000,
        })
    );
    let encoded = serde_json::to_string(&body).unwrap();
    for retired in [
        "contractOperationId",
        "gatewayEntryKey",
        "testEffectsEnabled",
        "testEffectDoubles",
    ] {
        assert!(
            !encoded.contains(retired),
            "control body retained {retired}"
        );
    }
}

#[test]
fn package_test_control_body_rejects_non_http_or_methodless_selectors() {
    let assembly = RuntimeAssemblyRef {
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(test_support::ASSEMBLY_B),
    };
    let mut entrypoint = package_test_entrypoint();
    entrypoint.selector.protocol = IngressProtocol::WebSocket;
    assert!(
        package_test_dispatch_body("http://127.0.0.1:46123", &assembly, 7, &entrypoint).is_err()
    );

    entrypoint.selector.protocol = IngressProtocol::Http;
    entrypoint.selector.method = None;
    assert!(
        package_test_dispatch_body("http://127.0.0.1:46123", &assembly, 7, &entrypoint).is_err()
    );
}

#[test]
fn business_success_is_sent_exactly_once() {
    let calls = Cell::new(0);
    let result = execute_business_request_once(|| {
        calls.set(calls.get() + 1);
        Ok(http::HttpResponse {
            status: 200,
            body: "ok".to_string(),
        })
    });

    assert_eq!(result, (true, None));
    assert_eq!(calls.get(), 1);
}

#[test]
fn business_503_is_sent_exactly_once() {
    let calls = Cell::new(0);
    let result = execute_business_request_once(|| {
        calls.set(calls.get() + 1);
        Ok(http::HttpResponse {
            status: 503,
            body: "runtime unavailable".to_string(),
        })
    });

    assert!(!result.0);
    assert!(result.1.unwrap().contains("HTTP 503"));
    assert_eq!(calls.get(), 1);
}

#[test]
fn business_timeout_and_transport_errors_are_each_sent_exactly_once() {
    for kind in [
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::ConnectionReset,
    ] {
        let calls = Cell::new(0);
        let result = execute_business_request_once(|| {
            calls.set(calls.get() + 1);
            Err(CanonicalFixtureError::Io {
                path: "http://127.0.0.1/test".to_string(),
                source: std::io::Error::new(kind, "scripted transport failure"),
            })
        });

        assert!(!result.0);
        assert_eq!(calls.get(), 1);
    }
}

fn package_test_entrypoint() -> CanonicalPackageTestEntrypoint {
    let source = "test \"control body\" { assert true }\n";
    CanonicalPackageTestEntrypoint {
        case: PackageTestCase {
            case_identity: "main::test[0]".to_string(),
            relative_path: "main.test.skiff".into(),
            module_path: "main".to_string(),
            name: "control body".to_string(),
            function_name: "skiffTestCase0".to_string(),
            test_index: 0,
            source_text: source.to_string(),
            source_ast: skiff_syntax::parser::parse_source(source).unwrap(),
        },
        selector: skiff_artifact_model::IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/__skiff/package-test/0".to_string(),
        },
        deployment: serde_json::from_value(serde_json::json!({
            "serviceId": "test.skiff/package/example",
            "contractVersion": "1.0.0",
            "deploymentRevision": "test-control",
            "deploymentArtifactIdentity": concat!(
                "skiff-deployment-artifact-v4:sha256:",
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            ),
        }))
        .unwrap(),
        gateway_entry_key: skiff_artifact_model::GatewayEntryKey::parse("run").unwrap(),
        gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity::parse(concat!(
            "skiff-gateway-entry-v2:sha256:",
            "b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305"
        ))
        .unwrap(),
        mode: skiff_artifact_model::GatewayDispatchMode::Unary,
    }
}
