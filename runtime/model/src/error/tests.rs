use super::*;

#[test]
fn model_error_payload_covers_domain_variants() {
    let decode = RuntimeModelError::Decode("invalid runtime type".to_string()).payload();
    assert_eq!(decode.code, "InternalError");
    assert_eq!(decode.message, "invalid runtime type");
    assert_eq!(decode.details, None);

    let limit = RuntimeModelError::ResourceLimitExceeded {
        resource: "heap".to_string(),
        reason: "request heap limit exceeded".to_string(),
        limit: 1024,
        current: 900,
        requested_delta: 200,
    }
    .payload();
    assert_eq!(limit.code, "ResourceLimitExceeded");
    assert_eq!(
        limit.details,
        Some(serde_json::json!({
            "resource": "heap",
            "reason": "request heap limit exceeded",
            "limit": 1024,
            "current": 900,
            "requestedDelta": 200,
        }))
    );

    let json_error: serde_json::Error =
        serde_json::from_str::<serde_json::Value>("{").expect_err("json should fail");
    let json = RuntimeModelError::Json(json_error).payload();
    assert_eq!(json.code, "JsonError");
    assert_eq!(json.details, None);
}

#[test]
fn model_error_is_not_catchable() {
    let error = RuntimeModelError::Decode("invalid runtime type".to_string());

    assert_eq!(WirePayload::catch_projection(&error), None);
    assert!(WirePayload::as_any(&error).is::<RuntimeModelError>());
}
