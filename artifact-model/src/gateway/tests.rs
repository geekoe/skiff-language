use serde_json::json;

use super::*;
use crate::{GatewayEntryIdentity, GatewayEntryKey, GATEWAY_ENTRY_IDENTITY_PREFIX};

#[test]
fn gateway_key_and_identity_are_distinct_validated_types() {
    let key = GatewayEntryKey::parse("chat.entry").expect("valid opaque key");
    let identity = GatewayEntryIdentity::parse(format!(
        "{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}",
        "a".repeat(64)
    ))
    .expect("valid content identity");
    assert_eq!(key.as_str(), "chat.entry");
    assert_ne!(key.as_str(), identity.as_str());

    for invalid in ["", " ", "two words", "line\nbreak", "nul\0byte"] {
        assert!(GatewayEntryKey::parse(invalid).is_err(), "{invalid:?}");
        assert!(
            serde_json::from_value::<GatewayEntryKey>(json!(invalid)).is_err(),
            "{invalid:?}"
        );
    }

    let digest = "a".repeat(64);
    for invalid in [
        String::new(),
        format!("skiff-gateway-v1:sha256:{digest}"),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "a".repeat(63)),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "A".repeat(64)),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "g".repeat(64)),
    ] {
        assert!(GatewayEntryIdentity::parse(&invalid).is_err(), "{invalid}");
        assert!(
            serde_json::from_value::<GatewayEntryIdentity>(json!(invalid)).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn websocket_entry_id_has_an_exact_independent_lexical_frame() {
    let digest = "b".repeat(64);
    let valid = format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{digest}");
    let parsed = WebSocketEntryId::parse(&valid).expect("valid WebSocket entry id");
    assert_eq!(parsed.as_str(), valid);
    assert_eq!(
        serde_json::from_value::<WebSocketEntryId>(json!(valid))
            .unwrap()
            .as_str(),
        parsed.as_str()
    );

    for invalid in [
        String::new(),
        format!("skiff-websocket-v1:sha256:{digest}"),
        format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "b".repeat(63)),
        format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "b".repeat(65)),
        format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "B".repeat(64)),
        format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{}", "g".repeat(64)),
        format!("{WEBSOCKET_ENTRY_ID_PREFIX}:{digest}:extra"),
    ] {
        assert!(WebSocketEntryId::parse(&invalid).is_err(), "{invalid}");
        assert!(
            serde_json::from_value::<WebSocketEntryId>(json!(invalid)).is_err(),
            "serde accepted {invalid}"
        );
    }
}

#[test]
fn gateway_adapter_source_vocabulary_and_args_are_strict() {
    let all_sources = [
        ("http.request", GatewayAdapterSource::HttpRequest),
        ("http.body", GatewayAdapterSource::HttpBody),
        ("http.context", GatewayAdapterSource::HttpContext),
        (
            "websocket.connectRequest",
            GatewayAdapterSource::WebSocketConnectRequest,
        ),
        (
            "websocket.jsonRpcParams",
            GatewayAdapterSource::WebSocketJsonRpcParams,
        ),
        (
            "websocket.connectionId",
            GatewayAdapterSource::WebSocketConnectionId,
        ),
        (
            "websocket.businessIdentity",
            GatewayAdapterSource::WebSocketBusinessIdentity,
        ),
    ];
    for (wire, source) in all_sources {
        let value = serde_json::to_value(source).expect("source serialization");
        assert_eq!(value, json!({ "kind": wire }));
        assert_eq!(
            serde_json::from_value::<GatewayAdapterSource>(value).expect("source parse"),
            source
        );
    }
    assert!(!GatewayAdapterSource::HttpContext.is_external_protocol_source());
    for source in [
        GatewayAdapterSource::WebSocketJsonRpcParams,
        GatewayAdapterSource::WebSocketConnectionId,
        GatewayAdapterSource::WebSocketBusinessIdentity,
    ] {
        assert!(source.is_external_protocol_source(), "{source:?}");
    }
    assert!(
        serde_json::from_value::<GatewayAdapterSource>(json!({ "kind": "http.query" })).is_err()
    );
    assert!(serde_json::from_value::<GatewayAdapterSource>(
        json!({ "kind": "http.body", "path": "payload" })
    )
    .is_err());
    assert_eq!(
        serde_json::from_value::<GatewayAdapterKind>(json!("websocketConnect")).unwrap(),
        GatewayAdapterKind::WebSocketConnect
    );
    assert_eq!(
        serde_json::from_value::<GatewayAdapterKind>(json!("websocketJsonRpc")).unwrap(),
        GatewayAdapterKind::WebSocketJsonRpc
    );
    for invalid in [
        "webSocketConnect",
        "websocket",
        "websocketReceive",
        "webSocketJsonRpc",
    ] {
        assert!(
            serde_json::from_value::<GatewayAdapterKind>(json!(invalid)).is_err(),
            "{invalid}"
        );
    }
    assert!(
        serde_json::from_value::<GatewayAdapterSource>(json!({ "kind": "websocket.message" }))
            .is_err()
    );

    let typed = [GatewayAdapterArg {
        param: "body".to_string(),
        source: GatewayAdapterSource::HttpBody,
    }];
    validate_gateway_adapter_args(GatewayAdapterKind::TypedJson, false, &typed)
        .expect("typed body source");
    assert!(validate_gateway_adapter_args(GatewayAdapterKind::RawHttp, false, &typed).is_err());
    assert!(validate_gateway_adapter_args(
        GatewayAdapterKind::TypedJson,
        false,
        &[GatewayAdapterArg {
            param: "context".to_string(),
            source: GatewayAdapterSource::HttpContext,
        }]
    )
    .is_err());
    validate_gateway_adapter_args(
        GatewayAdapterKind::WebSocketConnect,
        false,
        &[
            GatewayAdapterArg {
                param: "request".to_string(),
                source: GatewayAdapterSource::WebSocketConnectRequest,
            },
            GatewayAdapterArg {
                param: "connectionId".to_string(),
                source: GatewayAdapterSource::WebSocketConnectionId,
            },
        ],
    )
    .expect("WebSocket connect sources");
    validate_gateway_adapter_args(
        GatewayAdapterKind::WebSocketJsonRpc,
        false,
        &[
            GatewayAdapterArg {
                param: "params".to_string(),
                source: GatewayAdapterSource::WebSocketJsonRpcParams,
            },
            GatewayAdapterArg {
                param: "connectionId".to_string(),
                source: GatewayAdapterSource::WebSocketConnectionId,
            },
            GatewayAdapterArg {
                param: "businessIdentity".to_string(),
                source: GatewayAdapterSource::WebSocketBusinessIdentity,
            },
        ],
    )
    .expect("WebSocket JSON-RPC sources");
    assert!(validate_gateway_adapter_args(
        GatewayAdapterKind::WebSocketJsonRpc,
        false,
        &[GatewayAdapterArg {
            param: "request".to_string(),
            source: GatewayAdapterSource::WebSocketConnectRequest,
        }]
    )
    .is_err());
    assert!(validate_gateway_adapter_args(
        GatewayAdapterKind::WebSocketConnect,
        false,
        &[GatewayAdapterArg {
            param: "request".to_string(),
            source: GatewayAdapterSource::HttpRequest,
        }]
    )
    .is_err());
    assert!(validate_gateway_adapter_args(
        GatewayAdapterKind::TypedJson,
        false,
        &[
            GatewayAdapterArg {
                param: "body".to_string(),
                source: GatewayAdapterSource::HttpBody,
            },
            GatewayAdapterArg {
                param: "body".to_string(),
                source: GatewayAdapterSource::HttpRequest,
            },
        ]
    )
    .is_err());
    assert!(serde_json::from_value::<GatewayAdapterArg>(json!({
        "param": "body",
        "source": { "kind": "http.body" },
        "targetType": "PrivateRequest"
    }))
    .is_err());
}

#[test]
fn websocket_json_rpc_protocol_surface_has_one_closed_profile_and_schema_shape() {
    let surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketJsonRpc(
            GatewayWebSocketJsonRpcProtocolSurface {
                profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::WebSocketJsonRpcParams],
                params_schema: GatewayExternalSchema::Record {
                    fields: BTreeMap::from([("id".to_string(), GatewayExternalSchema::String)]),
                    required: vec!["id".to_string()],
                },
                result_schema: GatewayExternalSchema::Null,
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let value = serde_json::to_value(&surface).unwrap();
    assert_eq!(
        value["protocol"]["surface"]["profile"],
        json!("jsonrpc-2.0-text")
    );
    assert_eq!(
        serde_json::from_value::<GatewayEntryProtocolSurface>(value).unwrap(),
        surface
    );
    assert_eq!(
        GatewayWebSocketRpcProfile::JsonRpc2_0Text.wire_name(),
        WEBSOCKET_JSON_RPC_TEXT_PROFILE
    );
    assert!(
        serde_json::from_value::<GatewayWebSocketRpcProfile>(json!("jsonrpc-1.0-text")).is_err()
    );
}

#[test]
fn gateway_schema_has_no_nominal_or_untyped_escape_fields() {
    let strict_record = json!({
        "kind": "record",
        "fields": {
            "id": { "kind": "string" },
            "state": {
                "kind": "closedUnion",
                "branches": [
                    { "kind": "stringLiteral", "value": "open" },
                    { "kind": "stringLiteral", "value": "closed" }
                ]
            }
        },
        "required": ["id"]
    });
    serde_json::from_value::<GatewayExternalSchema>(strict_record.clone())
        .expect("closed schema vocabulary");

    for forbidden in [
        ("packageSchemaTypeId", json!("package-type")),
        ("typeRefIr", json!({ "kind": "builtin", "name": "User" })),
        ("publicPath", json!("types.User")),
        ("sourcePath", json!("internal.user")),
        ("nominalName", json!("User")),
        ("value", json!({ "arbitrary": true })),
    ] {
        let mut forged = strict_record.clone();
        forged
            .as_object_mut()
            .expect("record object")
            .insert(forbidden.0.to_string(), forbidden.1);
        assert!(
            serde_json::from_value::<GatewayExternalSchema>(forged).is_err(),
            "{} must be rejected",
            forbidden.0
        );
    }

    let vocabulary = [
        json!({ "kind": "null" }),
        json!({ "kind": "string" }),
        json!({ "kind": "number" }),
        json!({ "kind": "integer" }),
        json!({ "kind": "boolean" }),
        json!({ "kind": "bytes" }),
        json!({ "kind": "array", "items": { "kind": "string" } }),
        json!({
            "kind": "nullable",
            "inner": { "kind": "integer" }
        }),
        json!({
            "kind": "stringLiteral",
            "value": ""
        }),
    ];
    for schema in vocabulary {
        let parsed = serde_json::from_value::<GatewayExternalSchema>(schema.clone())
            .expect("supported external schema vocabulary");
        assert_eq!(
            serde_json::to_value(parsed).expect("schema serialization"),
            schema
        );
    }

    assert!(serde_json::from_value::<GatewayExternalSchema>(
        json!({ "kind": "string", "packageId": "example.com/private" })
    )
    .is_err());
    assert!(serde_json::from_value::<GatewayExternalSchema>(json!({
        "kind": "record",
        "fields": {},
        "required": [],
        "additionalProperties": true
    }))
    .is_err());
    assert!(
            serde_json::from_str::<GatewayExternalSchema>(
                r#"{"kind":"record","fields":{"id":{"kind":"string"},"id":{"kind":"integer"}},"required":["id"]}"#
            )
            .is_err(),
            "duplicate record field keys must not be silently overwritten"
        );
}

#[test]
fn gateway_surface_dto_rejects_unknown_fields_and_enum_values() {
    let surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpBody],
            request_body_schema: Some(GatewayExternalSchema::String),
            response_schema: Some(GatewayExternalSchema::Boolean),
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let mut wire = serde_json::to_value(&surface).expect("surface serialization");
    wire.as_object_mut()
        .expect("surface object")
        .insert("handler".to_string(), json!("internal.handle"));
    assert!(serde_json::from_value::<GatewayEntryProtocolSurface>(wire).is_err());

    let mut wire = serde_json::to_value(&surface).expect("surface serialization");
    wire["protocol"]["surface"]["adapterKind"] = json!("graphql");
    assert!(serde_json::from_value::<GatewayEntryProtocolSurface>(wire).is_err());

    let websocket = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketConnect(
            GatewayWebSocketConnectProtocolSurface {
                connect_request_shape: GatewayWebSocketShapeVersion::V1,
                connect_result_shape: GatewayWebSocketShapeVersion::V1,
                connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                external_sources: vec![
                    GatewayAdapterSource::WebSocketConnectRequest,
                    GatewayAdapterSource::WebSocketConnectionId,
                ],
                downlink_frames: vec![
                    GatewayWebSocketDownlinkFrame::Binary,
                    GatewayWebSocketDownlinkFrame::Text,
                ],
                rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    assert_eq!(
        serde_json::to_value(&websocket).unwrap(),
        json!({
            "protocol": {
                "kind": "websocketConnect",
                "surface": {
                    "connectRequestShape": "v1",
                    "connectResultShape": "v1",
                    "connectionPolicyShape": "v1",
                    "externalSources": [
                        { "kind": "websocket.connectRequest" },
                        { "kind": "websocket.connectionId" }
                    ],
                    "downlinkFrames": ["binary", "text"],
                    "rpcProfiles": ["jsonrpc-2.0-text"]
                }
            },
            "externalErrorProjection": {
                "kind": "fixed",
                "version": "v1"
            }
        })
    );
    let mut unknown = serde_json::to_value(websocket).unwrap();
    unknown["protocol"]["surface"]["receive"] = json!(true);
    assert!(serde_json::from_value::<GatewayEntryProtocolSurface>(unknown).is_err());

    for invalid in ["webSocketConnect", "websocket", "websocketReceive"] {
        let mut wrong_kind = serde_json::to_value(&surface).unwrap();
        wrong_kind["protocol"]["kind"] = json!(invalid);
        assert!(
            serde_json::from_value::<GatewayEntryProtocolSurface>(wrong_kind).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn canonical_websocket_v1_shapes_are_closed_and_exact() {
    let request = canonical_websocket_connect_schema(WEBSOCKET_CONNECT_REQUEST_V1_TYPE).unwrap();
    let GatewayExternalSchema::Record { fields, required } = request else {
        panic!("connect request must be a record");
    };
    assert_eq!(
        fields.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "connectionId",
            "cookies",
            "gatewayEntryIdentity",
            "headers",
            "query",
            "url",
            "version",
            "websocketEntryId"
        ]
    );
    assert!(!required.contains(&"version".to_string()));
    assert!(required.contains(&"websocketEntryId".to_string()));
    assert!(required.contains(&"gatewayEntryIdentity".to_string()));

    let result = canonical_websocket_connect_schema(WEBSOCKET_CONNECT_RESULT_V1_TYPE).unwrap();
    let GatewayExternalSchema::ClosedUnion { branches } = result else {
        panic!("connect result must be a closed union");
    };
    assert_eq!(branches.len(), 2);
    assert!(canonical_websocket_connect_schema("std.websocket.WebSocketIngressEvent").is_none());
}
