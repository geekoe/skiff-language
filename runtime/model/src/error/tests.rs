use std::borrow::Cow;

use super::*;

#[test]
fn model_error_diagnostics_cover_all_codes_and_message_ownership() {
    let decode = RuntimeModelError::Decode("invalid runtime type".to_string());
    assert_eq!(
        RuntimeDiagnostic::diagnostic_code(&decode).as_str(),
        "InternalError"
    );
    assert_eq!(
        RuntimeDiagnostic::diagnostic_message(&decode),
        Cow::Borrowed("invalid runtime type")
    );

    let limit = RuntimeModelError::ResourceLimitExceeded {
        resource: "heap".to_string(),
        reason: "request heap limit exceeded".to_string(),
        limit: 1024,
        current: 900,
        requested_delta: 200,
    };
    assert_eq!(
        RuntimeDiagnostic::diagnostic_code(&limit).as_str(),
        "ResourceLimitExceeded"
    );
    let limit_message = RuntimeDiagnostic::diagnostic_message(&limit);
    assert_eq!(
        limit_message,
        "resource limit exceeded for heap: request heap limit exceeded"
    );
    assert!(matches!(limit_message, Cow::Owned(_)));

    let json_error: serde_json::Error =
        serde_json::from_str::<serde_json::Value>("{").expect_err("json should fail");
    let expected_json_message = json_error.to_string();
    let json = RuntimeModelError::Json(json_error);
    assert_eq!(
        RuntimeDiagnostic::diagnostic_code(&json).as_str(),
        "JsonError"
    );
    let json_message = RuntimeDiagnostic::diagnostic_message(&json);
    assert_eq!(json_message, expected_json_message);
    assert!(matches!(json_message, Cow::Owned(_)));
}

#[test]
fn model_error_records_only_bounded_resource_limit_attributes() {
    fn attributes(error: &dyn RuntimeDiagnostic) -> DiagnosticAttributes {
        let mut attributes = DiagnosticAttributes::new();
        error.record_diagnostic_attributes(&mut attributes);
        attributes
    }

    let decode = RuntimeModelError::Decode("private decode text".to_string());
    assert!(attributes(&decode).is_empty());

    let limit = RuntimeModelError::ResourceLimitExceeded {
        resource: "private-resource".to_string(),
        reason: "private reason text".to_string(),
        limit: 1024,
        current: 900,
        requested_delta: 200,
    };
    let limit_attributes = attributes(&limit);
    assert_eq!(
        limit_attributes
            .iter()
            .map(|(key, value)| (key.as_str(), *value))
            .collect::<Vec<_>>(),
        vec![
            ("limit", DiagnosticFieldValue::U64(1024)),
            ("current", DiagnosticFieldValue::U64(900)),
            ("requested_delta", DiagnosticFieldValue::U64(200)),
        ]
    );
    assert!(!limit_attributes.was_truncated());
    let rendered_attributes = format!("{limit_attributes:?}");
    assert!(!rendered_attributes.contains("private-resource"));
    assert!(!rendered_attributes.contains("private reason text"));

    let json_error: serde_json::Error =
        serde_json::from_str::<serde_json::Value>("{").expect_err("json should fail");
    assert!(attributes(&RuntimeModelError::Json(json_error)).is_empty());
}

#[test]
fn model_error_supports_dyn_runtime_diagnostic() {
    let error = RuntimeModelError::Decode("invalid runtime type".to_string());
    let diagnostic: &dyn RuntimeDiagnostic = &error;

    assert_eq!(diagnostic.diagnostic_code().as_str(), "InternalError");
    assert_eq!(
        diagnostic.diagnostic_message(),
        Cow::Borrowed("invalid runtime type")
    );
    let mut attributes = DiagnosticAttributes::new();
    diagnostic.record_diagnostic_attributes(&mut attributes);
    assert!(attributes.is_empty());
}

#[test]
fn transitional_wire_payload_output_is_unchanged() {
    let decode = RuntimeModelError::Decode("invalid runtime type".to_string());
    assert_eq!(
        WirePayload::payload(&decode),
        RuntimeErrorPayload {
            code: "InternalError".to_string(),
            message: "invalid runtime type".to_string(),
            status: None,
            details: None,
        }
    );

    let limit = RuntimeModelError::ResourceLimitExceeded {
        resource: "heap".to_string(),
        reason: "request heap limit exceeded".to_string(),
        limit: 1024,
        current: 900,
        requested_delta: 200,
    };
    assert_eq!(
        WirePayload::payload(&limit),
        RuntimeErrorPayload {
            code: "ResourceLimitExceeded".to_string(),
            message: "resource limit exceeded for heap: request heap limit exceeded".to_string(),
            status: None,
            details: Some(serde_json::json!({
                "resource": "heap",
                "reason": "request heap limit exceeded",
                "limit": 1024,
                "current": 900,
                "requestedDelta": 200,
            })),
        }
    );

    let json_error: serde_json::Error =
        serde_json::from_str::<serde_json::Value>("{").expect_err("json should fail");
    let expected_json_message = json_error.to_string();
    let json = RuntimeModelError::Json(json_error);
    assert_eq!(
        WirePayload::payload(&json),
        RuntimeErrorPayload {
            code: "JsonError".to_string(),
            message: expected_json_message,
            status: None,
            details: None,
        }
    );
}

#[test]
fn matching_diagnostic_and_wire_codes_do_not_grant_catch_projection() {
    let json_error: serde_json::Error =
        serde_json::from_str::<serde_json::Value>("{").expect_err("json should fail");
    let errors = [
        RuntimeModelError::Decode("invalid runtime type".to_string()),
        RuntimeModelError::ResourceLimitExceeded {
            resource: "heap".to_string(),
            reason: "request heap limit exceeded".to_string(),
            limit: 1024,
            current: 900,
            requested_delta: 200,
        },
        RuntimeModelError::Json(json_error),
    ];

    for error in &errors {
        assert_eq!(
            WirePayload::payload(error).code,
            RuntimeDiagnostic::diagnostic_code(error).as_str()
        );
        assert_eq!(WirePayload::catch_projection(error), None);
        assert!(WirePayload::as_any(error).is::<RuntimeModelError>());
    }
}
