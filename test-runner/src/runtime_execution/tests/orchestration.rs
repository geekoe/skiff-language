use std::cell::{Cell, RefCell};
use std::os::unix::fs::PermissionsExt;

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
fn activation_request_preserves_dev_target_profile() {
    let assembly = RuntimeAssemblyRef {
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(test_support::ASSEMBLY_B),
    };

    let config_snapshot = test_support::snapshot_ref(test_support::SNAPSHOT_B);
    let body =
        activation_request_body("dev", "activation-dev", 7, &assembly, &config_snapshot).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        body,
        serde_json::json!({
            "schemaVersion": "skiff-assembly-activation-request-v3",
            "profile": "dev",
            "activationId": "activation-dev",
            "expectedGeneration": 7,
            "assembly": {
                "assemblyIdentity": test_support::ASSEMBLY_B,
            },
            "configSnapshot": {
                "snapshotId": test_support::SNAPSHOT_B,
            },
        })
    );
}

#[test]
fn base_snapshot_profile_is_used_to_align_non_live_test_environment() {
    let root = std::env::temp_dir().join(format!(
        "skiff-test-runner-profile-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("runtime-config")).unwrap();
    let permissions = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(&root, permissions.clone()).unwrap();
    std::fs::set_permissions(root.join("runtime-config"), permissions).unwrap();
    let store = skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
        root.join("runtime-config"),
    )
    .unwrap();
    let snapshot_ref = test_support::snapshot_ref(test_support::SNAPSHOT_B);
    let snapshot =
        skiff_runtime_config_snapshot::RuntimeConfigSnapshot::new("dev", snapshot_ref.clone(), Vec::new())
            .unwrap();
    store.publish(&snapshot).unwrap();

    let profile = base_snapshot_profile(&root, test_support::SNAPSHOT_B).unwrap();
    assert_eq!(profile, "dev");

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn shared_executor_activates_and_becomes_ready_once_then_dispatches_every_case() {
    let timeline = RefCell::new(Vec::new());
    let activation_calls = Cell::new(0);
    let readiness_calls = Cell::new(0);
    let dispatch_coordinates = RefCell::new(Vec::new());

    let summary = execute_shared_assembly_with(
        three_entrypoints(),
        7,
        |expected_generation, candidate_generation| {
            activation_calls.set(activation_calls.get() + 1);
            timeline.borrow_mut().push("activate".to_string());
            assert_eq!((expected_generation, candidate_generation), (7, 8));
            Ok(test_active_assembly(candidate_generation))
        },
        |active| {
            readiness_calls.set(readiness_calls.get() + 1);
            timeline.borrow_mut().push("readiness".to_string());
            assert_eq!(active.generation, 8);
            Ok(())
        },
        |active, entrypoint| {
            timeline
                .borrow_mut()
                .push(format!("dispatch:{}", entrypoint.case.name));
            dispatch_coordinates.borrow_mut().push((
                active.assembly.assembly_identity.as_str().to_string(),
                active.generation,
                entrypoint.deployment.clone(),
                entrypoint.gateway_entry_identity.clone(),
            ));
            if entrypoint.case.name == "case 2" {
                Ok(DispatchOutcome::Failed("assertion failed".to_string()))
            } else {
                Ok(DispatchOutcome::Passed)
            }
        },
    )
    .unwrap();

    assert_eq!(activation_calls.get(), 1);
    assert_eq!(readiness_calls.get(), 1);
    assert_eq!(
        &*timeline.borrow(),
        &[
            "activate",
            "readiness",
            "dispatch:case 1",
            "dispatch:case 2",
            "dispatch:case 3",
        ]
    );
    assert_eq!((summary.passed, summary.failed), (2, 1));
    let dispatch_coordinates = dispatch_coordinates.borrow();
    assert_eq!(dispatch_coordinates.len(), 3);
    assert!(dispatch_coordinates
        .iter()
        .all(
            |(identity, generation, _, _)| identity == test_support::ASSEMBLY_B && *generation == 8
        ));
    assert_eq!(
        dispatch_coordinates
            .iter()
            .map(|(_, _, deployment, _)| deployment.service_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "test.skiff/package/example-1",
            "test.skiff/package/example-2",
            "test.skiff/package/example-3",
        ]
    );
    assert_eq!(
        dispatch_coordinates
            .iter()
            .map(|(_, _, _, gateway)| gateway.as_str())
            .collect::<Vec<_>>(),
        vec![
            concat!(
                "skiff-gateway-entry-v2:sha256:",
                "0000000000000000000000000000000000000000000000000000000000000001"
            ),
            concat!(
                "skiff-gateway-entry-v2:sha256:",
                "0000000000000000000000000000000000000000000000000000000000000002"
            ),
            concat!(
                "skiff-gateway-entry-v2:sha256:",
                "0000000000000000000000000000000000000000000000000000000000000003"
            ),
        ]
    );
}

#[test]
fn shared_executor_activation_failure_has_an_empty_ledger_and_zero_dispatches() {
    let readiness_calls = Cell::new(0);
    let dispatch_calls = Cell::new(0);
    let error = execute_shared_assembly_with(
        three_entrypoints(),
        11,
        |expected_generation, candidate_generation| {
            assert_eq!((expected_generation, candidate_generation), (11, 12));
            Err::<ActivatedAssembly<()>, _>(CanonicalFixtureError::RemoteControl {
                status: 409,
                code: "ActivationGenerationMismatch".to_string(),
                message: "expected generation did not match".to_string(),
            })
        },
        |_| {
            readiness_calls.set(readiness_calls.get() + 1);
            Ok(())
        },
        |_, _| {
            dispatch_calls.set(dispatch_calls.get() + 1);
            Ok(DispatchOutcome::Passed)
        },
    )
    .unwrap_err();

    assert_eq!(readiness_calls.get(), 0);
    assert_eq!(dispatch_calls.get(), 0);
    assert_empty_first_case_ledger(error, "ActivationGenerationMismatch");
}

#[test]
fn shared_executor_readiness_failure_has_an_empty_ledger_and_zero_dispatches() {
    let activation_calls = Cell::new(0);
    let readiness_calls = Cell::new(0);
    let dispatch_calls = Cell::new(0);
    let error = execute_shared_assembly_with(
        three_entrypoints(),
        3,
        |expected_generation, candidate_generation| {
            activation_calls.set(activation_calls.get() + 1);
            assert_eq!((expected_generation, candidate_generation), (3, 4));
            Ok(test_active_assembly(candidate_generation))
        },
        |_| {
            readiness_calls.set(readiness_calls.get() + 1);
            Err(CanonicalFixtureError::InvalidInput(
                "runtime readiness failed: no healthy participant".to_string(),
            ))
        },
        |_, _| {
            dispatch_calls.set(dispatch_calls.get() + 1);
            Ok(DispatchOutcome::Passed)
        },
    )
    .unwrap_err();

    assert_eq!(activation_calls.get(), 1);
    assert_eq!(readiness_calls.get(), 1);
    assert_eq!(dispatch_calls.get(), 0);
    let CanonicalFixtureError::SuiteExecution {
        completed,
        module_path,
        name,
        source,
    } = error
    else {
        panic!("readiness failure did not become a suite-level error");
    };
    assert!(completed.is_empty());
    assert_eq!((module_path.as_str(), name.as_str()), ("main", "case 1"));
    assert!(matches!(
        *source,
        CanonicalFixtureError::InvalidInput(ref message)
            if message == "runtime readiness failed: no healthy participant"
    ));
}

#[test]
fn shared_executor_generation_overflow_fails_before_activation() {
    let activation_calls = Cell::new(0);
    let error = execute_shared_assembly_with(
        three_entrypoints(),
        u64::MAX,
        |_, _| {
            activation_calls.set(activation_calls.get() + 1);
            Ok(test_active_assembly(0))
        },
        |_| Ok(()),
        |_, _| Ok(DispatchOutcome::Passed),
    )
    .unwrap_err();

    assert_eq!(activation_calls.get(), 0);
    assert!(matches!(
        error,
        CanonicalFixtureError::SuiteExecution { source, .. }
            if matches!(*source, CanonicalFixtureError::InvalidInput(ref message)
                if message == "assembly activation expected generation cannot advance")
    ));
}

#[test]
fn test_service_control_body_is_the_exact_http_request() {
    let entrypoint = test_service_entrypoint();
    let assembly = RuntimeAssemblyRef {
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(test_support::ASSEMBLY_B),
    };

    let body = test_dispatch_body("http://127.0.0.1:46123", &assembly, 7, &entrypoint).unwrap();
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
                    "path": "/__skiff/test/0",
                },
            },
            "mode": "unary",
            "httpRequest": {
                "method": "POST",
                "url": "http://127.0.0.1:46123/__skiff/test/0",
                "path": "/__skiff/test/0",
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
fn test_service_control_body_rejects_non_http_or_methodless_selectors() {
    let assembly = RuntimeAssemblyRef {
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(test_support::ASSEMBLY_B),
    };
    let mut entrypoint = test_service_entrypoint();
    entrypoint.selector.protocol = IngressProtocol::WebSocket;
    assert!(test_dispatch_body("http://127.0.0.1:46123", &assembly, 7, &entrypoint).is_err());

    entrypoint.selector.protocol = IngressProtocol::Http;
    entrypoint.selector.method = None;
    assert!(test_dispatch_body("http://127.0.0.1:46123", &assembly, 7, &entrypoint).is_err());
}

#[test]
fn business_success_is_sent_exactly_once() {
    let calls = Cell::new(0);
    let result = execute_business_request_once(|| {
        calls.set(calls.get() + 1);
        Ok(http::HttpResponse {
            status: 200,
            body: valid_business_success_response(),
        })
    });

    assert_eq!(result.unwrap(), DispatchOutcome::Passed);
    assert_eq!(calls.get(), 1);
}

#[test]
fn typed_business_failure_is_sent_exactly_once() {
    let calls = Cell::new(0);
    let result = execute_business_request_once(|| {
        calls.set(calls.get() + 1);
        Ok(http::HttpResponse {
            status: 200,
            body: business_failure_response("UnhandledServiceError", "assertion failed"),
        })
    });

    assert_eq!(
        result.unwrap(),
        DispatchOutcome::Failed("UnhandledServiceError: assertion failed".to_string())
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn transport_errors_are_returned_to_the_suite_exactly_once() {
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

        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn harness_failure_stops_remaining_entrypoints_and_preserves_completed_ledger() {
    let entrypoints = three_entrypoints();
    let calls = RefCell::new(Vec::new());
    let error = execute_entrypoints_with(entrypoints, |entrypoint| {
        calls.borrow_mut().push(entrypoint.case.name.clone());
        match entrypoint.case.name.as_str() {
            "case 1" => Ok(DispatchOutcome::Passed),
            "case 2" => Err(CanonicalFixtureError::InvalidInput(
                "primary harness failure".to_string(),
            )),
            _ => panic!("entrypoint after a harness failure was dispatched"),
        }
    })
    .unwrap_err();

    assert_eq!(&*calls.borrow(), &["case 1", "case 2"]);
    let CanonicalFixtureError::SuiteExecution {
        completed,
        module_path,
        name,
        source,
    } = error
    else {
        panic!("harness failure did not become a suite-level error");
    };
    assert_eq!(completed.len(), 1);
    assert!(completed[0].passed);
    assert_eq!(module_path, "main");
    assert_eq!(name, "case 2");
    assert!(matches!(
        *source,
        CanonicalFixtureError::InvalidInput(ref message) if message == "primary harness failure"
    ));
}

#[test]
fn first_runtime_loss_prevents_the_other_two_entrypoints_from_dispatching() {
    let entrypoints = three_entrypoints();
    let calls = Cell::new(0);
    let error = execute_entrypoints_with(entrypoints, |_| {
        calls.set(calls.get() + 1);
        Err(CanonicalFixtureError::RemoteControl {
            status: 503,
            code: "AssemblyParticipantsUnavailable".to_string(),
            message: "runtime participant disconnected".to_string(),
        })
    })
    .unwrap_err();

    assert_eq!(calls.get(), 1);
    let CanonicalFixtureError::SuiteExecution {
        completed, source, ..
    } = error
    else {
        panic!("runtime loss did not become a suite-level error");
    };
    assert!(completed.is_empty());
    assert!(matches!(
        *source,
        CanonicalFixtureError::RemoteControl {
            status: 503,
            ref code,
            ..
        } if code == "AssemblyParticipantsUnavailable"
    ));
}

#[test]
fn business_failure_records_fail_and_continues_all_entrypoints() {
    let entrypoints = three_entrypoints();
    let calls = RefCell::new(Vec::new());
    let summary = execute_entrypoints_with(entrypoints, |entrypoint| {
        calls.borrow_mut().push(entrypoint.case.name.clone());
        match entrypoint.case.name.as_str() {
            "case 1" => Ok(DispatchOutcome::Failed("assertion failed".to_string())),
            _ => Ok(DispatchOutcome::Passed),
        }
    })
    .unwrap();

    assert_eq!(&*calls.borrow(), &["case 1", "case 2", "case 3"]);
    assert_eq!((summary.passed, summary.failed), (2, 1));
    assert_eq!(summary.results.len(), 3);
    assert_eq!(
        summary.results[0].message.as_deref(),
        Some("assertion failed")
    );
}

#[test]
fn non_success_control_response_requires_and_preserves_typed_error_identity() {
    let unavailable = execute_business_request_once(|| {
        Ok(http::HttpResponse {
            status: 503,
            body: serde_json::json!({
                "error": {
                    "code": "AssemblyParticipantsUnavailable",
                    "message": "No healthy RuntimeAssembly participant is connected",
                },
            })
            .to_string(),
        })
    })
    .unwrap_err();
    assert!(matches!(
        unavailable,
        CanonicalFixtureError::RemoteControl {
            status: 503,
            ref code,
            ..
        } if code == "AssemblyParticipantsUnavailable"
    ));

    let malformed = execute_business_request_once(|| {
        Ok(http::HttpResponse {
            status: 503,
            body: "runtime unavailable".to_string(),
        })
    })
    .unwrap_err();
    assert!(matches!(malformed, CanonicalFixtureError::Wire { .. }));
}

fn three_entrypoints() -> Vec<CanonicalTestServiceEntrypoint> {
    (1..=3)
        .map(|index| {
            let mut entrypoint = test_service_entrypoint();
            entrypoint.case.name = format!("case {index}");
            entrypoint.selector.path = format!("/__skiff/test/{index}");
            entrypoint.deployment = serde_json::from_value(serde_json::json!({
                "serviceId": format!("test.skiff/package/example-{index}"),
                "contractVersion": "1.0.0",
                "deploymentRevision": format!("test-control-{index}"),
                "deploymentArtifactIdentity": format!(
                    "skiff-deployment-artifact-v4:sha256:{index:064x}"
                ),
            }))
            .unwrap();
            entrypoint.gateway_entry_identity = skiff_artifact_model::GatewayEntryIdentity::parse(
                format!("skiff-gateway-entry-v2:sha256:{index:064x}"),
            )
            .unwrap();
            entrypoint
        })
        .collect()
}

fn test_active_assembly(generation: u64) -> ActivatedAssembly<()> {
    ActivatedAssembly {
        assembly: RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                test_support::ASSEMBLY_B,
            ),
        },
        generation,
        readiness: (),
    }
}

fn assert_empty_first_case_ledger(error: CanonicalFixtureError, expected_code: &str) {
    let CanonicalFixtureError::SuiteExecution {
        completed,
        module_path,
        name,
        source,
    } = error
    else {
        panic!("pre-dispatch failure did not become a suite-level error");
    };
    assert!(completed.is_empty());
    assert_eq!((module_path.as_str(), name.as_str()), ("main", "case 1"));
    assert!(matches!(
        *source,
        CanonicalFixtureError::RemoteControl { ref code, .. } if code == expected_code
    ));
}

fn business_failure_response(code: &str, message: &str) -> String {
    serde_json::json!({
        "ok": true,
        "header": {
            "schemaVersion": "skiff-runtime-frame-v4",
            "type": "response.error",
            "requestId": "package-test-failure",
            "errorKind": "control",
            "error": {
                "code": code,
                "message": message,
            },
        },
        "payloadBase64": "",
    })
    .to_string()
}

fn valid_business_success_response() -> String {
    serde_json::json!({
        "ok": true,
        "header": {
            "schemaVersion": "skiff-runtime-frame-v4",
            "type": "response.end",
            "requestId": "package-test-success",
            "payloadPresent": true,
            "httpResponse": {
                "status": 200,
                "headers": [{
                    "name": "content-type",
                    "value": "application/json; charset=utf-8",
                }],
            },
        },
        "payloadBase64": "bnVsbA==",
    })
    .to_string()
}

pub(super) fn test_service_entrypoint() -> CanonicalTestServiceEntrypoint {
    let source = "test \"control body\" { assert true }\n";
    CanonicalTestServiceEntrypoint {
        case: TestServiceCase {
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
            path: "/__skiff/test/0".to_string(),
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
