use super::super::test_support::*;
use super::*;
use skiff_runtime_request_contract::{PlatformErrorProjectionPayload, StdFileFileErrorPayload};

#[test]
fn test_dispatch_response_requires_canonical_response_end_and_null_payload() {
    assert_eq!(
        decode_test_dispatch_response(&valid_test_dispatch_response().to_string()).unwrap(),
        TestDispatchOutcome::Passed
    );

    let mut mutations = Vec::new();
    let mut mutate = |name: &'static str, update: fn(&mut Value)| {
        let mut response = valid_test_dispatch_response();
        update(&mut response);
        mutations.push((name, response));
    };
    mutate("outer unknown field", |value| {
        value["legacy"] = Value::Bool(true);
    });
    mutate("outer ok false", |value| {
        value["ok"] = Value::Bool(false);
    });
    mutate("wrong frame type", |value| {
        value["header"]["type"] = Value::String("response.error".to_string());
    });
    mutate("empty request id", |value| {
        value["header"]["requestId"] = Value::String(String::new());
    });
    mutate("noncanonical request id", |value| {
        value["header"]["requestId"] = Value::String("request id".to_string());
    });
    mutate("payload flag false", |value| {
        value["header"]["payloadPresent"] = Value::Bool(false);
    });
    mutate("inner status failure", |value| {
        value["header"]["httpResponse"]["status"] = serde_json::json!(500);
    });
    mutate("wrong content type", |value| {
        value["header"]["httpResponse"]["headers"][0]["value"] =
            Value::String("application/json".to_string());
    });
    mutate("extra inner header", |value| {
        value["header"]["httpResponse"]["headers"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name": "x-extra", "value": "forbidden"}));
    });
    mutate("invalid base64", |value| {
        value["payloadBase64"] = Value::String("***".to_string());
    });
    mutate("non-null payload", |value| {
        value["payloadBase64"] = Value::String("e30=".to_string());
    });

    for (name, response) in mutations {
        assert!(
            decode_test_dispatch_response(&response.to_string()).is_err(),
            "response mutation {name} was accepted"
        );
    }
}

#[test]
fn test_dispatch_typed_control_error_is_a_business_failure() {
    let response = serde_json::json!({
        "ok": true,
        "header": {
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "response.error",
            "requestId": "package-test-request-error",
            "errorKind": "control",
            "error": {
                "code": "UnhandledServiceError",
                "message": "unhandled request-local user exception",
            },
        },
        "payloadBase64": "",
    });

    assert_eq!(
        decode_test_dispatch_response(&response.to_string()).unwrap(),
        TestDispatchOutcome::Failed(
            "UnhandledServiceError: unhandled request-local user exception".to_string()
        )
    );
}

#[test]
fn test_dispatch_typed_fixed_error_is_a_business_failure() {
    let payload = serde_json::to_vec(&serde_json::json!({
        "kind": "internalError",
        "payload": {
            "message": "Internal service error",
            "traceId": "trace-package-test",
            "errorId": "error-package-test",
        },
    }))
    .unwrap();
    let response = serde_json::json!({
        "ok": true,
        "header": {
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "response.error",
            "requestId": "package-test-request-fixed-error",
            "errorKind": "fixedService",
        },
        "payloadBase64": BASE64_STANDARD.encode(payload),
    });

    assert_eq!(
        decode_test_dispatch_response(&response.to_string()).unwrap(),
        TestDispatchOutcome::Failed("Internal service error".to_string())
    );
}

#[test]
fn test_dispatch_typed_platform_error_reports_the_projection_key() {
    let payload = PlatformErrorProjectionPayload::StdFileFileError(StdFileFileErrorPayload {
        message: "provider detail must stay opaque".to_string(),
    });
    let projection_key = payload.key();
    let error = OpaqueServiceError::platform_error(
        &payload,
        "trace-package-test-platform-error",
        "error-package-test-platform-error",
    )
    .unwrap();
    let response = serde_json::json!({
        "ok": true,
        "header": {
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "response.error",
            "requestId": "package-test-request-platform-error",
            "errorKind": "fixedService",
        },
        "payloadBase64": BASE64_STANDARD.encode(error.encoded_bytes()),
    });

    assert_eq!(
        decode_test_dispatch_response(&response.to_string()).unwrap(),
        TestDispatchOutcome::Failed(format!("fixed service error {projection_key}"))
    );
}

#[test]
fn test_dispatch_malformed_error_frame_is_a_wire_failure() {
    let response = serde_json::json!({
        "ok": true,
        "header": {
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "response.error",
            "requestId": "package-test-request-error",
            "errorKind": "control",
            "error": {
                "code": "",
                "message": "missing canonical code",
            },
        },
        "payloadBase64": "",
    });

    assert!(matches!(
        decode_test_dispatch_response(&response.to_string()),
        Err(CanonicalFixtureError::Wire { .. })
    ));
}

#[test]
fn health_decodes_the_release_projection_and_ignores_router_owned_surfaces() {
    let health =
        decode_health_snapshot(&health_body(PROFILE, vec![DEPLOYMENT_A, DEPLOYMENT_B])).unwrap();
    assert_eq!(health.active.profile, PROFILE);
    assert_eq!(health.active.release_count, 2);
    assert_eq!(
        health.active.build_ids,
        [DEPLOYMENT_A, DEPLOYMENT_B]
            .into_iter()
            .map(str::to_string)
            .collect()
    );

    let mut extended = valid_health();
    extended["routerOwnedSurface"] = serde_json::json!({ "nested": true });
    extended["counters"] = serde_json::json!({ "sessions": { "registeredSessions": 1 } });
    let health = decode_health_snapshot(&extended.to_string())
        .expect("router-owned fields must be tolerated");
    assert_eq!(health.active.release_count, 2);
}

#[test]
fn health_projection_mutations_fail_closed() {
    let valid = || {
        serde_json::from_str::<Value>(&health_body(PROFILE, vec![DEPLOYMENT_A, DEPLOYMENT_B]))
            .unwrap()
    };
    let mutate = |update: fn(&mut Value)| {
        let mut mutated = valid();
        update(&mut mutated);
        mutated
    };
    let cases = vec![
        (
            "unknown activeAssembly field",
            mutate(|value: &mut Value| {
                value["activeAssembly"]["legacy"] = Value::Bool(true);
            }),
        ),
        (
            "missing buildIds",
            mutate(|value: &mut Value| {
                value["activeAssembly"]
                    .as_object_mut()
                    .unwrap()
                    .remove("buildIds");
            }),
        ),
        (
            "missing releaseCount",
            mutate(|value: &mut Value| {
                value["activeAssembly"]
                    .as_object_mut()
                    .unwrap()
                    .remove("releaseCount");
            }),
        ),
        (
            "missing profile",
            mutate(|value: &mut Value| {
                value["activeAssembly"]
                    .as_object_mut()
                    .unwrap()
                    .remove("profile");
            }),
        ),
        (
            "non-array buildIds",
            mutate(|value: &mut Value| {
                value["activeAssembly"]["buildIds"] = Value::String("build".to_string());
            }),
        ),
        (
            "non-string buildId",
            mutate(|value: &mut Value| {
                value["activeAssembly"]["buildIds"][0] = Value::Bool(true);
            }),
        ),
        (
            "non-canonical buildId",
            mutate(|value: &mut Value| {
                value["activeAssembly"]["buildIds"][0] = Value::String("build id".to_string());
            }),
        ),
        (
            "fractional releaseCount",
            mutate(|value: &mut Value| {
                value["activeAssembly"]["releaseCount"] = serde_json::json!(2.5);
            }),
        ),
        (
            "negative releaseCount",
            mutate(|value: &mut Value| {
                value["activeAssembly"]["releaseCount"] = serde_json::json!(-1);
            }),
        ),
    ];

    for (name, value) in cases {
        assert!(
            decode_health_snapshot(&value.to_string()).is_err(),
            "mutation {name} was accepted"
        );
    }
}

#[test]
fn health_missing_ok_or_wrong_ok_fail_closed() {
    let mut missing = valid_health();
    missing.as_object_mut().unwrap().remove("ok");
    assert!(decode_health_snapshot(&missing.to_string()).is_err());

    let mut wrong = valid_health();
    wrong["ok"] = Value::Bool(false);
    assert!(decode_health_snapshot(&wrong.to_string()).is_err());

    let mut absent = valid_health();
    absent.as_object_mut().unwrap().remove("activeAssembly");
    assert!(decode_health_snapshot(&absent.to_string()).is_err());
}

fn valid_test_dispatch_response() -> Value {
    serde_json::json!({
        "ok": true,
        "header": {
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "response.end",
            "requestId": "package-test-request-1",
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
}
