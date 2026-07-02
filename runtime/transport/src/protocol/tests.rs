use std::collections::HashMap;

use serde_json::json;

use crate::protocol::{
    decode_binary_frame, decode_typed_binary_frame, encode_binary_frame, ConnectionSendEnvelope,
    ConnectionSendFrameHeader, PackageTestStartFrameHeader, RequestCancelFrameHeader,
    RequestStartFrameHeader, RequestTestEffectDouble, ResponseEndFrameHeader,
    ResponseErrorFrameHeader, ResponseStartFrameHeader, RouterControlEnvelope,
    RuntimeCallerFrameHeader, RuntimeCapabilitiesFrameHeader,
    RuntimeCapabilitiesFrameHeaderMetadata, RuntimeDeadlineFrameHeader,
    RuntimeDispatchModeCapability, RuntimeErrorFramePayload, RuntimeHttpAdapterArgFrameHeader,
    RuntimeHttpAdapterCallableFrameHeader, RuntimeHttpAdapterFrameHeader,
    RuntimeHttpAdapterKindFrameHeader, RuntimeHttpAdapterSourceFrameHeader,
    RuntimeHttpNameValueFrameHeader, RuntimeHttpResponseFrameHeader, RuntimeRegisterEnvelope,
    RuntimeRegisterFrameHeader, RuntimeTraceContextFrameHeader, TelemetryProtocol, TelemetryTopic,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

const SERVICE_PROTOCOL_A: &str =
    "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SERVICE_REVISION: &str = "1111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn runtime_register_frame_header_round_trips_empty_payload() {
    let envelope = RuntimeRegisterEnvelope {
        envelope_type: "runtime.register",
        runtime_id: "runtime-1".to_string(),
        service_id: "example.com/service-a".to_string(),
        version: "v1".to_string(),
        build_id:
            "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .to_string(),
        revision_id: SERVICE_REVISION.to_string(),
        activation_identity: Some("skiff-runtime-activation-v1:opaque:activation-fixture".to_string()),
        service_protocol_identity: SERVICE_PROTOCOL_A.to_string(),
        contract_identity: SERVICE_PROTOCOL_A.to_string(),
        targets: vec![
            "service.test.Api.alpha".to_string(),
            "service.test.Api.beta".to_string(),
        ],
        protocol_version: "skiff-protocol-v1".to_string(),
        runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        code_revision_id: SERVICE_REVISION.to_string(),
        implementation_identity:
            "skiff-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        artifact_identity:
            "skiff-service-assembly-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
        capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
            dispatch_modes: vec![RuntimeDispatchModeCapability::Unary],
            package_test_dispatch: true,
            request_cancel: true,
            runtime_program: true,
        },
        gateway_entry_identities: vec![
            "skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111"
                .to_string(),
        ],
    };
    let header = RuntimeRegisterFrameHeader::from(envelope);

    let frame = encode_binary_frame(&header, &[]).expect("runtime.register frame encodes");
    let (decoded, payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("runtime.register frame decodes");

    assert_eq!(decoded, header);
    assert!(payload.is_empty());
    assert_eq!(decoded.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(decoded.envelope_type, "runtime.register");
    assert_eq!(decoded.runtime_id, "runtime-1");
    assert_eq!(decoded.service_id, "example.com/service-a");
    assert_eq!(
        decoded.build_id,
        "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
    );
    assert_eq!(
        decoded.activation_identity.as_deref(),
        Some("skiff-runtime-activation-v1:opaque:activation-fixture")
    );
    assert_eq!(decoded.service_protocol_identity, SERVICE_PROTOCOL_A);
    assert_eq!(
        decoded.artifact_identity.as_deref(),
        Some(
            "skiff-service-assembly-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        )
    );
    assert_eq!(
        decoded.targets,
        vec![
            "service.test.Api.alpha".to_string(),
            "service.test.Api.beta".to_string()
        ]
    );
    assert_eq!(
        decoded.capabilities,
        Some(RuntimeCapabilitiesFrameHeaderMetadata {
            dispatch_modes: vec![RuntimeDispatchModeCapability::Unary],
            package_test_dispatch: true,
            request_cancel: true,
            runtime_program: true,
        })
    );
    assert_eq!(
        decoded.gateway_entry_identities,
        vec![
            "skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111"
                .to_string()
        ]
    );
}

#[test]
fn runtime_capabilities_frame_header_round_trips_empty_payload() {
    let header = RuntimeCapabilitiesFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.capabilities".to_string(),
        runtime_id: "runtime-base-1".to_string(),
        capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
            package_test_dispatch: true,
            request_cancel: true,
            ..RuntimeCapabilitiesFrameHeaderMetadata::default()
        },
    };

    let frame = encode_binary_frame(&header, &[]).expect("runtime.capabilities frame encodes");
    let (decoded, payload): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("runtime.capabilities frame decodes");

    assert_eq!(decoded, header);
    assert!(payload.is_empty());
    assert_eq!(decoded.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(decoded.envelope_type, "runtime.capabilities");
    assert_eq!(decoded.runtime_id, "runtime-base-1");
    assert_eq!(
        decoded.capabilities,
        RuntimeCapabilitiesFrameHeaderMetadata {
            package_test_dispatch: true,
            request_cancel: true,
            ..RuntimeCapabilitiesFrameHeaderMetadata::default()
        }
    );
}

#[test]
fn package_test_start_frame_header_round_trips_router_shape() {
    let header = PackageTestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "package-test.start".to_string(),
        request_id: "package-test-request-1".to_string(),
        caller: RuntimeCallerFrameHeader {
            kind: "gateway".to_string(),
            target: "__skiff.test-dispatch".to_string(),
        },
        package_id: "example.com/hello".to_string(),
        package_version: "0.1.0".to_string(),
        test_build_identity:
            "skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        entrypoint_id:
            "skiff-package-test-entrypoint-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
        activation_id: "skiff-package-test-run-v1:example.com~hello:aaaaaaaa:run-fixture:1"
            .to_string(),
        deadline: Some(RuntimeDeadlineFrameHeader {
            timeout_ms: 2000,
            expires_at: "2026-01-01T00:00:02.000Z".to_string(),
        }),
        trace: RuntimeTraceContextFrameHeader {
            trace_id: "trace-package-test-1".to_string(),
            span_id: "span-package-test-1".to_string(),
            parent_span_id: None,
            sampled: Some(true),
        },
        test_effects_enabled: true,
        test_effect_doubles: HashMap::from([(
            "std.http.fetch".to_string(),
            vec![RequestTestEffectDouble {
                expect_request: Some(json!({"url": "https://example.com"})),
                response: json!({"status": 200}),
            }],
        )]),
    };

    let frame = encode_binary_frame(&header, b"encoded package test payload")
        .expect("package-test.start frame encodes");
    let (decoded, payload): (PackageTestStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("package-test.start frame decodes");

    assert_eq!(decoded, header);
    assert_eq!(payload, b"encoded package test payload");
    assert_eq!(decoded.envelope_type, "package-test.start");
    assert_eq!(decoded.caller.kind, "gateway");
    assert_eq!(decoded.caller.target, "__skiff.test-dispatch");
}

#[test]
fn connection_send_envelope_serializes_router_formal_fields() {
    let value = serde_json::to_value(ConnectionSendEnvelope {
        envelope_type: "connection.send",
        service_id: "example.com/sample".to_string(),
        websocket_entry_id: Some("gateway.websocket.chat".to_string()),
        business_identity: Some("user-1".to_string()),
        connection_id: None,
        payload_kind: "text".to_string(),
    })
    .expect("connection.send envelope should serialize");

    assert_eq!(
        value,
        json!({
            "type": "connection.send",
            "serviceId": "example.com/sample",
            "websocketEntryId": "gateway.websocket.chat",
            "businessIdentity": "user-1",
            "payloadKind": "text"
        })
    );
}

#[test]
fn connection_send_frame_header_round_trips_payload_kind() {
    let header = ConnectionSendFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.send".to_string(),
        service_id: "example.com/sample".to_string(),
        websocket_entry_id: Some("gateway.websocket.chat".to_string()),
        business_identity: Some("user-1".to_string()),
        connection_id: None,
        payload_kind: Some("text".to_string()),
    };
    let payload = "hello typed text".as_bytes().to_vec();

    let frame = encode_binary_frame(&header, &payload).expect("connection.send frame encodes");
    let (decoded, decoded_payload): (ConnectionSendFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("connection.send frame decodes");

    assert_eq!(decoded, header);
    assert_eq!(decoded_payload, payload);
    let value = serde_json::to_value(decoded).expect("connection.send header serializes");
    assert_eq!(value["websocketEntryId"], "gateway.websocket.chat");
    assert_eq!(value["businessIdentity"], "user-1");
    assert!(value.get("identity").is_none());
    assert_eq!(value["payloadKind"], "text");
}

#[test]
fn runtime_http_adapter_frame_header_serializes_structured_adapter_args() {
    let header = RuntimeHttpAdapterFrameHeader {
        kind: RuntimeHttpAdapterKindFrameHeader::TypedJson,
        handler: RuntimeHttpAdapterCallableFrameHeader::ServiceFunction {
            module_path: "program.main".to_string(),
            symbol: "run".to_string(),
        },
        guard: None,
        pre: None,
        adapter_args: vec![RuntimeHttpAdapterArgFrameHeader {
            param: "input".to_string(),
            source: RuntimeHttpAdapterSourceFrameHeader::HttpBody,
        }],
    };

    let value = serde_json::to_value(&header).expect("HTTP adapter header should serialize");

    assert!(value.get("handlerArgs").is_none());
    assert_eq!(
        value["adapterArgs"],
        json!([{ "param": "input", "source": { "kind": "http.body" } }])
    );
    let decoded: RuntimeHttpAdapterFrameHeader =
        serde_json::from_value(value).expect("HTTP adapter header should deserialize");
    assert_eq!(decoded, header);

    let legacy = json!({
        "kind": "typedJson",
        "handler": {
            "kind": "serviceFunction",
            "modulePath": "program.main",
            "symbol": "run"
        },
        "handlerArgs": [{ "kind": "body" }]
    });
    assert!(serde_json::from_value::<RuntimeHttpAdapterFrameHeader>(legacy).is_err());
}

#[test]
fn router_control_envelope_deserializes_artifact_roots() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": ["/tmp/skiff-artifacts"],
        "devReload": true,
        "generation": "generation-1"
    }))
    .expect("router control envelope should deserialize");

    assert_eq!(
        value
            .ordered_artifact_roots()
            .expect("artifact roots should normalize"),
        vec![std::path::PathBuf::from("/tmp/skiff-artifacts")]
    );
    assert_eq!(value.dev_reload, Some(true));
    assert_eq!(value.generation.as_deref(), Some("generation-1"));
}

#[test]
fn router_control_envelope_deserializes_multiple_artifact_roots() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": [
            "/tmp/skiff-artifacts-default",
            "/tmp/skiff-artifacts-test"
        ],
        "devReload": true
    }))
    .expect("router control envelope should deserialize artifactRoots");

    assert_eq!(
        value
            .ordered_artifact_roots()
            .expect("artifact roots should normalize"),
        vec![
            std::path::PathBuf::from("/tmp/skiff-artifacts-default"),
            std::path::PathBuf::from("/tmp/skiff-artifacts-test")
        ]
    );
}

#[test]
fn router_control_envelope_rejects_legacy_artifact_root() {
    let error = serde_json::from_value::<RouterControlEnvelope>(json!({
        "artifactRoot": "/tmp/skiff-artifacts-default",
    }))
    .expect_err("legacy artifactRoot-only control should not deserialize");

    assert_eq!(
        error.classify(),
        serde_json::error::Category::Data,
        "unexpected error: {error}"
    );
}

#[test]
fn router_control_envelope_rejects_duplicate_artifact_roots() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": [
            "/tmp/skiff-artifacts",
            "/tmp/skiff-artifacts"
        ]
    }))
    .expect("router control envelope should deserialize before semantic validation");

    let error = value
        .reject_legacy_config_fields()
        .expect_err("duplicate roots should be rejected");

    assert!(
        error.contains("duplicate root"),
        "unexpected error: {error}"
    );
}

#[test]
fn router_control_envelope_deserializes_service_config_only() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": ["/tmp/skiff-artifacts"],
        "serviceConfig": [
            {
                "serviceId": "example.com/service-a",
                "buildId": "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "activationIdentity": "activation-a",
                "resolvedConfigIdentity": "skiff-config-resolved-v1:opaque:config-a",
                "resolvedConfig": { "app": { "apiKey": "secret" } },
                "redactedResolvedConfig": { "app": { "apiKey": "[REDACTED]" } },
                "redactionProjectionIdentity": "skiff-config-redaction-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "configShape": {
                    "schemaVersion": "skiff-config-shape-v1",
                    "entries": [
                        { "path": "app.apiKey", "type": "string", "required": true }
                    ]
                },
                "serviceDb": {
                    "mongoUrl": "mongodb://127.0.0.1:27017/?directConnection=true",
                    "storageServiceId": "example.com/service-a"
                },
                "packageConfigs": [
                    {
                        "packageId": "skiff.run/llm",
                        "alias": "llm",
                        "resolvedConfigIdentity": "skiff-config-resolved-v1:opaque:pkg-config-a",
                        "resolvedConfig": { "dashscope": { "apiKey": "secret" } },
                        "redactedResolvedConfig": { "dashscope": { "apiKey": "[REDACTED]" } },
                        "redactionProjectionIdentity": "skiff-config-redaction-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "configShape": {
                            "schemaVersion": "skiff-config-shape-v1",
                            "entries": [
                                { "path": "dashscope.apiKey", "type": "string", "required": true }
                            ]
                        }
                    }
                ]
            }
        ]
    }))
    .expect("router control serviceConfig should deserialize");

    assert_eq!(value.service_config.len(), 1);
    assert_eq!(value.service_config[0].service_id, "example.com/service-a");
    assert_eq!(
        value.service_config[0].build_id,
        "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(
        value.service_config[0].resolved_config_identity,
        "skiff-config-resolved-v1:opaque:config-a"
    );
    assert_eq!(
        value.service_config[0].resolved_config["app"]["apiKey"],
        "secret"
    );
    assert_eq!(
        value.service_config[0].redacted_resolved_config["app"]["apiKey"],
        "[REDACTED]"
    );
    assert_eq!(
        value.service_config[0]
            .service_db
            .as_ref()
            .expect("serviceDb should deserialize")
            .mongo_url,
        "mongodb://127.0.0.1:27017/?directConnection=true"
    );
    assert_eq!(
        value.service_config[0]
            .service_db
            .as_ref()
            .expect("serviceDb should deserialize")
            .storage_service_id,
        "example.com/service-a"
    );
    assert_eq!(value.service_config[0].package_configs.len(), 1);
    assert_eq!(
        value.service_config[0].package_configs[0].package_id,
        "skiff.run/llm"
    );
    assert_eq!(value.service_config[0].package_configs[0].alias, "llm");
    assert_eq!(
        value.service_config[0].package_configs[0].resolved_config["dashscope"]["apiKey"],
        "secret"
    );
    value
        .reject_legacy_config_fields()
        .expect("new serviceConfig fields should validate");
}

#[test]
fn router_control_envelope_rejects_legacy_service_values() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": ["/tmp/skiff-artifacts"],
        "serviceValues": [
            {
                "serviceId": "example.com/legacy",
                "buildId": "legacy",
                "activationIdentity": "legacy",
                "valuesSnapshotIdentity": "legacy"
            }
        ]
    }))
    .expect("legacy top-level field is captured for protocol validation");

    let error = value
        .reject_legacy_config_fields()
        .expect_err("serviceValues must be rejected");

    assert!(error.contains("serviceValues is no longer supported"));
}

#[test]
fn router_control_envelope_rejects_legacy_service_config_item_fields() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": ["/tmp/skiff-artifacts"],
        "serviceConfig": [
            {
                "serviceId": "example.com/service-a",
                "buildId": "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "activationIdentity": "activation-a",
                "resolvedConfigIdentity": "skiff-config-resolved-v1:opaque:config-a",
                "resolvedConfig": {},
                "redactedResolvedConfig": {},
                "redactionProjectionIdentity": "skiff-config-redaction-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "configShape": {
                    "schemaVersion": "skiff-config-shape-v1",
                    "entries": []
                },
                "valuesSnapshotIdentity": "legacy"
            }
        ]
    }))
    .expect("legacy serviceConfig item field is captured for protocol validation");

    let error = value
        .reject_legacy_config_fields()
        .expect_err("legacy serviceConfig item field must be rejected");

    assert!(error.contains("serviceConfig[0].valuesSnapshotIdentity is no longer supported"));
}

#[test]
fn router_control_envelope_deserializes_telemetry_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../doc/architecture/fixtures/observability-minimal.json"
    ))
    .expect("observability fixture should parse");
    let mut control = fixture["valid"]["routerControl"].clone();
    let object = control
        .as_object_mut()
        .expect("router control fixture should be an object");
    let artifact_root = object
        .remove("artifactRoot")
        .expect("legacy fixture should include artifactRoot");
    object.insert("artifactRoots".to_string(), json!([artifact_root]));
    let value: RouterControlEnvelope = serde_json::from_value(control)
        .expect("router control telemetry fixture should deserialize");

    let telemetry = value.telemetry.expect("telemetry config should be present");
    assert_eq!(telemetry.endpoint, "ws://127.0.0.1:4002/telemetry");
    assert_eq!(telemetry.protocol, TelemetryProtocol::SkiffTelemetryV1);
    assert_eq!(
        telemetry.topics,
        vec![
            TelemetryTopic::Log,
            TelemetryTopic::Trace,
            TelemetryTopic::Metric,
            TelemetryTopic::Health,
            TelemetryTopic::Debug
        ]
    );
    assert_eq!(telemetry.queue_max_events, 10_000);
    assert_eq!(telemetry.batch_max_events, 200);
    assert_eq!(telemetry.batch_max_bytes, 262_144);
    assert_eq!(telemetry.flush_interval_ms, 1000);
    assert!(telemetry.enabled);
}

#[test]
fn router_control_envelope_deserializes_file_backend() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": ["/tmp/skiff-artifacts"],
        "fileBackend": {
            "local": {
                "root": "/var/lib/skiff/file-blobs"
            },
            "oss": {
                "endpoint": "https://oss-cn-hangzhou.aliyuncs.com",
                "bucket": "skiff-files",
                "region": "cn-hangzhou",
                "accessKeyIdEnv": "SKIFF_OSS_ACCESS_KEY_ID",
                "accessKeySecretEnv": "SKIFF_OSS_ACCESS_KEY_SECRET"
            }
        }
    }))
    .expect("router control fileBackend should deserialize");

    let file_backend = value.file_backend.expect("fileBackend should be present");
    assert_eq!(
        file_backend.local.expect("local backend should exist").root,
        std::path::PathBuf::from("/var/lib/skiff/file-blobs")
    );
    let oss = file_backend.oss.expect("oss backend should exist");
    assert_eq!(oss.endpoint, "https://oss-cn-hangzhou.aliyuncs.com");
    assert_eq!(oss.bucket, "skiff-files");
    assert_eq!(oss.region.as_deref(), Some("cn-hangzhou"));
    assert_eq!(
        oss.access_key_id_env.as_deref(),
        Some("SKIFF_OSS_ACCESS_KEY_ID")
    );
    assert_eq!(
        oss.access_key_secret_env.as_deref(),
        Some("SKIFF_OSS_ACCESS_KEY_SECRET")
    );
}

#[test]
fn router_control_envelope_rejects_incomplete_file_backend() {
    let value: RouterControlEnvelope = serde_json::from_value(json!({
        "artifactRoots": ["/tmp/skiff-artifacts"],
        "fileBackend": {
            "oss": {
                "endpoint": "https://oss-cn-hangzhou.aliyuncs.com",
                "bucket": "skiff-files",
                "accessKeyIdEnv": "SKIFF_OSS_ACCESS_KEY_ID"
            }
        }
    }))
    .expect("router control fileBackend should deserialize before semantic validation");

    let error = value
        .reject_legacy_config_fields()
        .expect_err("missing secret credential source should be rejected");

    assert!(error.contains("requires accessKeySecretEnv or accessKeySecret"));
}

#[test]
fn runtime_binary_frame_round_trips_typed_header_and_payload_bytes() {
    let header = RequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "request.start".to_string(),
        request_id: "request-1".to_string(),
        mode: "unary".to_string(),
        caller: RuntimeCallerFrameHeader {
            kind: "gateway".to_string(),
            target: "gateway.hello.http.raw".to_string(),
        },
        target: "service.example~com~~service-a.Api.hello".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        version: None,
        build_id:
            "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .to_string(),
        service_protocol_identity:
            "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        activation_identity: None,
        gateway_entry_identity: None,
        business_identity: None,
        websocket_entry_id: None,
        client_session: None,
        deadline: Some(RuntimeDeadlineFrameHeader {
            timeout_ms: 2_000,
            expires_at: "2026-01-01T00:00:02.000Z".to_string(),
        }),
        trace: RuntimeTraceContextFrameHeader {
            trace_id: "trace-1".to_string(),
            span_id: "span-1".to_string(),
            parent_span_id: None,
            sampled: Some(true),
        },
        http_adapter: None,
        websocket_adapter: None,
        http_request: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
    };
    let payload = [0, 1, 2, 123, 34, 255];

    let frame = encode_binary_frame(&header, &payload).expect("binary frame should encode");
    let decoded = decode_binary_frame(&frame).expect("binary frame should decode");
    assert_eq!(decoded.payload_bytes, payload);

    let (typed_header, typed_payload): (RequestStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("typed frame should decode");
    assert_eq!(typed_header, header);
    assert_eq!(typed_payload, payload);
}

#[test]
fn runtime_binary_request_start_decodes_test_effect_fields() {
    let test_effect_doubles = HashMap::from([(
        "std.http.request".to_string(),
        vec![RequestTestEffectDouble {
            expect_request: Some(json!({
                "method": "GET",
                "url": "https://example.test/items"
            })),
            response: json!({
                "status": 200,
                "headers": [],
                "body": { "text": "ok" }
            }),
        }],
    )]);
    let header = RequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "request.start".to_string(),
        request_id: "request-test-1".to_string(),
        mode: "unary".to_string(),
        caller: RuntimeCallerFrameHeader {
            kind: "gateway".to_string(),
            target: "gateway.test".to_string(),
        },
        target: "service.example~com~~service-a.Api.test".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        version: None,
        build_id:
            "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .to_string(),
        service_protocol_identity: SERVICE_PROTOCOL_A.to_string(),
        activation_identity: None,
        gateway_entry_identity: None,
        business_identity: None,
        websocket_entry_id: None,
        client_session: None,
        deadline: None,
        trace: RuntimeTraceContextFrameHeader {
            trace_id: "trace-test".to_string(),
            span_id: "span-test".to_string(),
            parent_span_id: None,
            sampled: Some(true),
        },
        http_adapter: None,
        websocket_adapter: None,
        http_request: None,
        test_effects_enabled: true,
        test_effect_doubles: test_effect_doubles.clone(),
    };

    let frame = encode_binary_frame(&header, &[]).expect("binary frame should encode");
    let (decoded_header, payload): (RequestStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("typed frame should decode");

    assert!(payload.is_empty());
    assert!(decoded_header.test_effects_enabled);
    assert_eq!(decoded_header.test_effect_doubles, test_effect_doubles);
}

#[test]
fn runtime_binary_frame_allows_header_only_control_and_error_frames() {
    let cancel_header = RequestCancelFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "request.cancel".to_string(),
        request_id: "request-1".to_string(),
        reason: "timeout".to_string(),
    };

    let frame = encode_binary_frame(&cancel_header, &[]).expect("header-only frame should encode");
    let (typed_header, payload): (RequestCancelFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("header-only frame should decode");

    assert_eq!(typed_header, cancel_header);
    assert!(payload.is_empty());

    let start_header = ResponseStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "response.start".to_string(),
        request_id: "request-1".to_string(),
        http_response: RuntimeHttpResponseFrameHeader {
            status: 200,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "content-type".to_string(),
                value: "text/plain".to_string(),
            }],
        },
    };

    let frame = encode_binary_frame(&start_header, &[]).expect("start frame should encode");
    let (typed_header, payload): (ResponseStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("start frame should decode");

    assert_eq!(typed_header, start_header);
    assert!(payload.is_empty());

    let error_header = ResponseErrorFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "response.error".to_string(),
        request_id: "request-1".to_string(),
        error: RuntimeErrorFramePayload {
            code: "FixtureError".to_string(),
            message: "fixture runtime error".to_string(),
            status: None,
            details: None,
        },
    };

    let frame = encode_binary_frame(&error_header, &[]).expect("error frame should encode");
    let (typed_header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("error frame should decode");

    assert_eq!(typed_header, error_header);
    assert!(payload.is_empty());
}

#[test]
fn runtime_binary_frame_rejects_legacy_json_text_envelope() {
    let legacy = br#"{"type":"request.start","requestId":"request-1","args":{"name":"Ada"}}"#;

    let error = decode_binary_frame(legacy).expect_err("legacy JSON text must fail closed");

    assert!(error
        .to_string()
        .contains("expected skiff binary frame magic"));
}

#[test]
fn runtime_frame_header_rejects_legacy_payload_fields() {
    let request_error = serde_json::from_value::<RequestStartFrameHeader>(json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": "request.start",
        "requestId": "request-1",
        "mode": "unary",
        "caller": {
            "kind": "gateway",
            "target": "gateway.hello.http.raw"
        },
        "target": "service.example~com~~service-a.Api.hello",
        "buildId": "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "serviceProtocolIdentity": "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "trace": {
            "traceId": "trace-1",
            "spanId": "span-1"
        },
        "args": {
            "name": "Ada"
        }
    }))
    .expect_err("legacy args must not be accepted in frame headers");
    assert!(
        request_error.to_string().contains("unknown field `args`"),
        "unexpected error: {request_error}"
    );

    let response_error = serde_json::from_value::<ResponseEndFrameHeader>(json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": "response.end",
        "requestId": "request-1",
        "payloadPresent": true,
        "payload": {
            "message": "hello"
        }
    }))
    .expect_err("legacy response payload must not be accepted in frame headers");
    assert!(
        response_error
            .to_string()
            .contains("unknown field `payload`"),
        "unexpected error: {response_error}"
    );
}

#[test]
fn runtime_frame_header_rejects_invalid_test_effect_doubles() {
    let empty_sequence_error = serde_json::from_value::<RequestStartFrameHeader>(json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": "request.start",
        "requestId": "request-1",
        "mode": "unary",
        "caller": {
            "kind": "gateway",
            "target": "gateway.hello.http.raw"
        },
        "target": "service.example~com~~service-a.Api.hello",
        "buildId": "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "serviceProtocolIdentity": SERVICE_PROTOCOL_A,
        "trace": {
            "traceId": "trace-1",
            "spanId": "span-1"
        },
        "testEffectDoubles": {
            "std.http.request": []
        }
    }))
    .expect_err("empty test double sequence must not be accepted");
    assert!(
        empty_sequence_error
            .to_string()
            .contains("testEffectDoubles.std.http.request must be a non-empty array"),
        "unexpected error: {empty_sequence_error}"
    );

    let unknown_field_error = serde_json::from_value::<RequestStartFrameHeader>(json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": "request.start",
        "requestId": "request-1",
        "mode": "unary",
        "caller": {
            "kind": "gateway",
            "target": "gateway.hello.http.raw"
        },
        "target": "service.example~com~~service-a.Api.hello",
        "buildId": "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "serviceProtocolIdentity": SERVICE_PROTOCOL_A,
        "trace": {
            "traceId": "trace-1",
            "spanId": "span-1"
        },
        "testEffectDoubles": {
            "std.http.request": [{
                "response": null,
                "unexpected": true
            }]
        }
    }))
    .expect_err("unknown test double fields must not be accepted");
    assert!(
        unknown_field_error
            .to_string()
            .contains("unknown field `unexpected`"),
        "unexpected error: {unknown_field_error}"
    );
}

#[test]
fn runtime_http_binary_frame_headers_round_trip_metadata_without_body_base64() {
    let header = RequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "request.start".to_string(),
        request_id: "request-http-1".to_string(),
        mode: "unary".to_string(),
        caller: RuntimeCallerFrameHeader {
            kind: "gateway".to_string(),
            target: "gateway.http.raw".to_string(),
        },
        target: "service.http.handle".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        version: None,
        build_id:
            "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .to_string(),
        service_protocol_identity: SERVICE_PROTOCOL_A.to_string(),
        activation_identity: None,
        gateway_entry_identity: None,
        business_identity: None,
        websocket_entry_id: None,
        client_session: None,
        deadline: None,
        trace: RuntimeTraceContextFrameHeader {
            trace_id: "trace-http".to_string(),
            span_id: "span-http".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_adapter: None,
        websocket_adapter: None,
        http_request: Some(crate::protocol::RuntimeHttpRequestFrameHeader {
            method: "POST".to_string(),
            url: "https://example.test/items?x=1".to_string(),
            path: "/items".to_string(),
            query: vec![crate::protocol::RuntimeHttpNameValueFrameHeader {
                name: "x".to_string(),
                value: "1".to_string(),
            }],
            headers: vec![crate::protocol::RuntimeHttpNameValueFrameHeader {
                name: "content-type".to_string(),
                value: "application/octet-stream".to_string(),
            }],
        }),
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
    };
    let body = b"\x00raw body bytes".to_vec();

    let frame = encode_binary_frame(&header, &body).expect("HTTP frame should encode");
    let (decoded, decoded_body): (RequestStartFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("HTTP frame should decode");

    assert_eq!(decoded.http_request.as_ref().unwrap().method, "POST");
    assert_eq!(decoded_body, body);
    let header_json = serde_json::to_string(&decoded).expect("header serializes");
    assert!(!header_json.contains("__skiffBytesBase64"));
    assert!(!header_json.contains("raw body bytes"));
}
