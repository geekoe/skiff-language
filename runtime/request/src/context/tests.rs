use std::collections::HashMap;

use serde_json::{json, Map};

use super::*;

fn request_with_caller_kind(kind: Option<&str>) -> RequestEnvelope {
    let mut extra = Map::new();
    if let Some(kind) = kind {
        extra.insert("caller".to_string(), json!({ "kind": kind }));
    }
    RequestEnvelope {
        request_id: "request-1".to_string(),
        mode: "unary".to_string(),
        target: "function:target".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some("skiff.run/account".to_string()),
        build_id: "build-a".to_string(),
        service_protocol_identity: "protocol-a".to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: None,
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
        payload_bytes: Vec::new(),
        extra,
    }
}

#[test]
fn spawn_caller_marks_request_payload_as_recoverable_spawn() {
    let request = request_with_caller_kind(Some("spawn"));
    let context = request_payload_context_from_request(&request);

    assert_eq!(
        context.payload_encoding(),
        RequestPayloadEncoding::RecoverableSpawnPayload
    );
}

#[test]
fn non_spawn_request_payload_defaults_to_runtime_binary() {
    for kind in [None, Some("service"), Some("gateway")] {
        let request = request_with_caller_kind(kind);
        let context = request_payload_context_from_request(&request);

        assert_eq!(
            context.payload_encoding(),
            RequestPayloadEncoding::RuntimeBinary
        );
    }
}
