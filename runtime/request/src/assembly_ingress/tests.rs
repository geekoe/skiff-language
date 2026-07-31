use std::collections::HashMap;

use skiff_artifact_model::{IngressProtocol, IngressSelector};

use super::validate_assembly_ingress_request;
use crate::RequestEnvelope;

#[test]
fn assembly_ingress_ignores_legacy_target_fields_but_requires_canonical_selector() {
    let mut request = request();
    request.build_id = "mutated-build".to_string();
    request.operation_abi_id = Some("mutated-operation-abi".to_string());
    request.selector = Some("mutated-display-selector".to_string());
    request.target = "mutated-display-target".to_string();
    assert!(validate_assembly_ingress_request(&request).is_ok());

    request.ingress_selector = None;
    let error = validate_assembly_ingress_request(&request)
        .expect_err("missing canonical selector must fail closed");
    assert!(error.to_string().contains("canonical ingress selector"));
}

#[test]
fn assembly_ingress_rejects_legacy_callable_adapter_before_dispatch() {
    let mut request = request();
    request.mode = "serverStream".to_string();
    assert!(validate_assembly_ingress_request(&request).is_err());
}

fn request() -> RequestEnvelope {
    RequestEnvelope {
        request_id: "assembly-ingress-request".to_string(),
        mode: "unary".to_string(),
        target: "display-only".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        build_id: "legacy-build".to_string(),
        service_protocol_identity: "legacy-protocol".to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/entry".to_string(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
        payload_bytes: Vec::new(),
        extra: serde_json::Map::new(),
    }
}
