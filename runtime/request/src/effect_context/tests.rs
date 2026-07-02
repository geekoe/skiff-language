use std::collections::HashMap;

use serde_json::{json, Map, Value};
use time::{format_description::well_known::Rfc3339, Duration, OffsetDateTime};

use super::request_deadline_ms;
use crate::RequestEnvelope;

#[test]
fn request_deadline_uses_timeout_ms_when_expires_at_missing() {
    let request = request_with_deadline(json!({ "timeoutMs": 2500 }));

    assert_eq!(request_deadline_ms(&request), Some(2500));
}

#[test]
fn request_deadline_uses_minimum_of_timeout_ms_and_expires_at_remaining() {
    let expires_at = OffsetDateTime::now_utc() + Duration::seconds(60);
    let request = request_with_deadline(json!({
        "timeoutMs": 5,
        "expiresAt": expires_at.format(&Rfc3339).unwrap()
    }));

    assert_eq!(request_deadline_ms(&request), Some(5));
}

#[test]
fn request_deadline_uses_expired_expires_at_even_with_larger_timeout() {
    let expires_at = OffsetDateTime::now_utc() - Duration::seconds(1);
    let request = request_with_deadline(json!({
        "timeoutMs": 5000,
        "expiresAt": expires_at.format(&Rfc3339).unwrap()
    }));

    assert_eq!(request_deadline_ms(&request), Some(0));
}

#[test]
fn request_deadline_falls_back_to_timeout_ms_for_malformed_expires_at() {
    let request = request_with_deadline(json!({
        "timeoutMs": 5000,
        "expiresAt": "not-a-date"
    }));

    assert_eq!(request_deadline_ms(&request), Some(5000));
}

fn request_with_deadline(deadline: Value) -> RequestEnvelope {
    let mut extra = Map::new();
    extra.insert("deadline".to_string(), deadline);
    RequestEnvelope {
        request_id: "request-1".to_string(),
        mode: "unary".to_string(),
        target: "svc.operation".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        build_id: "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string(),
        service_protocol_identity:
            "skiff-protocol-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        contract_identity: None,
        activation_identity: None,
        binary_http: None,
        http_adapter: None,
        websocket_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
        payload_bytes: Vec::new(),
        extra,
    }
}
