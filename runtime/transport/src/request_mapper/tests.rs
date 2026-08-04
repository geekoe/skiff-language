use std::collections::HashMap;

use serde_json::json;
use skiff_artifact_model::IngressProtocol;

use super::{request_cancel_from_frame_header, request_envelope_from_start_frame};
use crate::protocol::{
    RequestCancelFrameHeader, RequestStartFrameHeader, RequestTestEffectDouble,
    RuntimeCallerFrameHeader, RuntimeGatewayAdapterArgFrameHeader,
    RuntimeGatewayAdapterSourceFrameHeader, RuntimeHttpAdapterCallableFrameHeader,
    RuntimeHttpAdapterFrameHeader, RuntimeHttpAdapterKindFrameHeader,
    RuntimeHttpNameValueFrameHeader, RuntimeHttpRequestFrameHeader, RuntimeTraceContextFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_request_contract::{
    GatewayAdapterSource, HttpAdapterCallable, HttpAdapterKind, RuntimeClientSessionControl,
};

#[test]
fn request_start_frame_maps_to_request_envelope() {
    let payload = b"opaque request body".to_vec();
    let request = request_envelope_from_start_frame(
        RequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.start".to_string(),
            request_id: "request-1".to_string(),
            mode: "unary".to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "gateway".to_string(),
                target: "gateway.http.raw".to_string(),
            },
            target: "service.target".to_string(),
            operation_abi_id: Some("operation-abi".to_string()),
            selector: Some("operation:operation-abi".to_string()),
            service_id: Some("skiff.run/service".to_string()),
            version: Some("0.1.0".to_string()),
            build_id: "build-1".to_string(),
            service_protocol_identity: "protocol-1".to_string(),
            activation_identity: Some("activation-1".to_string()),
            gateway_entry_identity: Some("gateway-entry-1".to_string()),
            client_session: Some(RuntimeClientSessionControl {
                id: "client".to_string(),
            }),
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: "trace-1".to_string(),
                span_id: "span-1".to_string(),
                parent_span_id: None,
                sampled: Some(true),
            },
            http_request: Some(RuntimeHttpRequestFrameHeader {
                method: "POST".to_string(),
                url: "https://example.com/path?q=1".to_string(),
                path: "/path".to_string(),
                query: vec![RuntimeHttpNameValueFrameHeader {
                    name: "q".to_string(),
                    value: "1".to_string(),
                }],
                headers: vec![RuntimeHttpNameValueFrameHeader {
                    name: "content-type".to_string(),
                    value: "application/octet-stream".to_string(),
                }],
            }),
            http_adapter: Some(RuntimeHttpAdapterFrameHeader {
                kind: RuntimeHttpAdapterKindFrameHeader::RawHttp,
                handler: RuntimeHttpAdapterCallableFrameHeader::ServiceFunction {
                    module_path: "api".to_string(),
                    symbol: "handle".to_string(),
                },
                guard: Some(RuntimeHttpAdapterCallableFrameHeader::PackageFunction {
                    package_id: "std".to_string(),
                    symbol_path: "http.guard".to_string(),
                }),
                pre: None,
                adapter_args: vec![RuntimeGatewayAdapterArgFrameHeader {
                    param: "request".to_string(),
                    source: RuntimeGatewayAdapterSourceFrameHeader::HttpRequest,
                }],
            }),
            test_effects_enabled: true,
            test_effect_doubles: [(
                "effect.target".to_string(),
                vec![RequestTestEffectDouble {
                    expect_request: Some(json!({"arg": 1})),
                    response: json!({"ok": true}),
                }],
            )]
            .into_iter()
            .collect(),
        },
        payload.clone(),
    )
    .expect("request.start should map");

    assert_eq!(request.request_id, "request-1");
    assert_eq!(request.mode, "unary");
    assert_eq!(request.target, "service.target");
    assert_eq!(request.operation_abi_id.as_deref(), Some("operation-abi"));
    assert_eq!(request.selector.as_deref(), Some("operation:operation-abi"));
    assert_eq!(request.service_id.as_deref(), Some("skiff.run/service"));
    assert_eq!(request.build_id, "build-1");
    assert_eq!(request.service_protocol_identity, "protocol-1");
    assert_eq!(request.activation_identity.as_deref(), Some("activation-1"));
    assert_eq!(request.payload_bytes, payload);
    assert!(request.contract_identity.is_none());
    let ingress = request
        .ingress_selector
        .as_ref()
        .expect("HTTP metadata should project a canonical ingress selector");
    assert_eq!(ingress.protocol, IngressProtocol::Http);
    assert_eq!(ingress.method.as_deref(), Some("POST"));
    assert_eq!(ingress.path, "/path");

    let binary_http = request.binary_http.expect("binary HTTP request should map");
    assert_eq!(binary_http.metadata.method, "POST");
    assert_eq!(binary_http.metadata.query[0].name, "q");
    assert_eq!(binary_http.body, b"opaque request body".to_vec());

    let http_adapter = request.http_adapter.expect("HTTP adapter should map");
    assert_eq!(http_adapter.kind, HttpAdapterKind::RawHttp);
    assert_eq!(
        http_adapter.handler,
        HttpAdapterCallable::ServiceFunction {
            module_path: "api".to_string(),
            symbol: "handle".to_string(),
        }
    );
    assert_eq!(
        http_adapter.adapter_args[0].source,
        GatewayAdapterSource::HttpRequest
    );

    let doubles = request
        .test_effect_doubles
        .get("effect.target")
        .expect("test effect doubles should map");
    assert_eq!(doubles[0].expect_request, Some(json!({"arg": 1})));
    assert_eq!(doubles[0].response, json!({"ok": true}));

    assert_eq!(
        request.extra.get("gatewayEntryIdentity"),
        Some(&json!("gateway-entry-1"))
    );
    assert_eq!(request.extra.get("trace.trace_id"), None);
    assert_eq!(
        request
            .extra
            .get("trace")
            .and_then(|value| value.get("traceId")),
        Some(&json!("trace-1"))
    );
}

#[test]
fn request_start_frame_rejects_wrong_schema_version() {
    let error = request_envelope_from_start_frame(
        minimal_request_start_header("old-schema", "request.start", "build-1"),
        Vec::new(),
    )
    .expect_err("wrong schema should fail");

    assert!(error.contains("request.start schemaVersion must be skiff-runtime-frame-v4"));
}

#[test]
fn request_start_frame_rejects_wrong_envelope_type() {
    let error = request_envelope_from_start_frame(
        minimal_request_start_header(RUNTIME_FRAME_SCHEMA_VERSION, "response.start", "build-1"),
        Vec::new(),
    )
    .expect_err("wrong frame type should fail");

    assert!(error.contains("binary frame type must be request.start"));
}

#[test]
fn request_start_frame_rejects_empty_build_id() {
    let error = request_envelope_from_start_frame(
        minimal_request_start_header(RUNTIME_FRAME_SCHEMA_VERSION, "request.start", ""),
        Vec::new(),
    )
    .expect_err("empty build id should fail");

    assert_eq!(error, "request.start buildId must be a non-empty string");
}

#[test]
fn request_cancel_frame_maps_to_request_cancel() {
    let cancel = request_cancel_from_frame_header(RequestCancelFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "request.cancel".to_string(),
        request_id: "request-1".to_string(),
        reason: "caller_cancelled".to_string(),
    });

    assert_eq!(cancel.request_id, "request-1");
    assert_eq!(cancel.reason.as_deref(), Some("caller_cancelled"));
}

fn minimal_request_start_header(
    schema_version: &str,
    envelope_type: &str,
    build_id: &str,
) -> RequestStartFrameHeader {
    RequestStartFrameHeader {
        schema_version: schema_version.to_string(),
        envelope_type: envelope_type.to_string(),
        request_id: "request-1".to_string(),
        mode: "unary".to_string(),
        caller: RuntimeCallerFrameHeader {
            kind: "gateway".to_string(),
            target: "gateway".to_string(),
        },
        target: "service.target".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        version: None,
        build_id: build_id.to_string(),
        service_protocol_identity: "protocol-1".to_string(),
        activation_identity: None,
        gateway_entry_identity: None,
        client_session: None,
        deadline: None,
        trace: RuntimeTraceContextFrameHeader {
            trace_id: "trace-1".to_string(),
            span_id: "span-1".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: None,
        http_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
    }
}
