use std::collections::BTreeMap;

use serde_json::json;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    GatewayAdapterArg, GatewayAdapterKind, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayExternalErrorProjection,
    GatewayExternalSchema, GatewayHttpProtocolSurface, GatewayProtocolSurface,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketJsonRpcProtocolSurface, GatewayWebSocketRpcProfile,
    GatewayWebSocketShapeVersion, WEBSOCKET_GATEWAY_ENTRY_KEY,
};

use super::*;

fn string_record(field_order: &[&str], required: Vec<String>) -> GatewayExternalSchema {
    let mut fields = BTreeMap::new();
    for name in field_order {
        fields.insert((*name).to_string(), GatewayExternalSchema::String);
    }
    GatewayExternalSchema::Record { fields, required }
}

fn typed_http_surface() -> GatewayEntryProtocolSurface {
    normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![
                GatewayAdapterSource::HttpRequest,
                GatewayAdapterSource::HttpBody,
            ],
            request_body_schema: Some(string_record(
                &["query", "requestId"],
                vec!["requestId".to_string(), "query".to_string()],
            )),
            response_schema: Some(GatewayExternalSchema::ClosedUnion {
                branches: vec![
                    GatewayExternalSchema::StringLiteral {
                        value: "ok".to_string(),
                    },
                    GatewayExternalSchema::StringLiteral {
                        value: "accepted".to_string(),
                    },
                ],
            }),
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .expect("canonical typed HTTP surface")
}

fn raw_http_surface(mode: GatewayDispatchMode) -> GatewayEntryProtocolSurface {
    normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::RawHttp,
            dispatch_mode: mode,
            external_sources: vec![GatewayAdapterSource::HttpRequest],
            request_body_schema: None,
            response_schema: None,
            stream_item_schema: (mode == GatewayDispatchMode::ServerStream).then_some(
                GatewayExternalSchema::Record {
                    fields: BTreeMap::from([
                        ("body".to_string(), GatewayExternalSchema::Bytes),
                        ("event".to_string(), GatewayExternalSchema::String),
                    ]),
                    required: vec!["body".to_string(), "event".to_string()],
                },
            ),
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .expect("canonical raw HTTP surface")
}

fn websocket_surface() -> GatewayEntryProtocolSurface {
    normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketConnect(
            GatewayWebSocketConnectProtocolSurface {
                connect_request_shape: GatewayWebSocketShapeVersion::V1,
                connect_result_shape: GatewayWebSocketShapeVersion::V1,
                connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                external_sources: vec![
                    GatewayAdapterSource::WebSocketConnectionId,
                    GatewayAdapterSource::WebSocketConnectRequest,
                ],
                downlink_frames: vec![
                    GatewayWebSocketDownlinkFrame::Text,
                    GatewayWebSocketDownlinkFrame::Binary,
                ],
                rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
                connection_close_shape: GatewayWebSocketShapeVersion::V1,
                close_external_sources: vec![],
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .expect("canonical WebSocket connect surface")
}

fn http_surface(surface: &GatewayEntryProtocolSurface) -> &GatewayHttpProtocolSurface {
    match &surface.protocol {
        GatewayProtocolSurface::Http(http) => http,
        GatewayProtocolSurface::WebSocketConnect(_)
        | GatewayProtocolSurface::WebSocketJsonRpc(_) => {
            panic!("HTTP surface helper received websocketConnect")
        }
    }
}

fn http_surface_mut(surface: &mut GatewayEntryProtocolSurface) -> &mut GatewayHttpProtocolSurface {
    match &mut surface.protocol {
        GatewayProtocolSurface::Http(http) => http,
        GatewayProtocolSurface::WebSocketConnect(_)
        | GatewayProtocolSurface::WebSocketJsonRpc(_) => {
            panic!("HTTP surface helper received websocketConnect")
        }
    }
}

fn websocket_json_rpc_surface(
    external_sources: Vec<GatewayAdapterSource>,
    params_schema: GatewayExternalSchema,
    result_schema: GatewayExternalSchema,
) -> GatewayEntryProtocolSurface {
    normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketJsonRpc(
            GatewayWebSocketJsonRpcProtocolSurface {
                profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources,
                params_schema,
                result_schema,
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .expect("canonical WebSocket JSON-RPC surface")
}

fn websocket_json_rpc_surface_mut(
    surface: &mut GatewayEntryProtocolSurface,
) -> &mut GatewayWebSocketJsonRpcProtocolSurface {
    match &mut surface.protocol {
        GatewayProtocolSurface::WebSocketJsonRpc(json_rpc) => json_rpc,
        GatewayProtocolSurface::Http(_) | GatewayProtocolSurface::WebSocketConnect(_) => {
            panic!("JSON-RPC surface helper received another protocol")
        }
    }
}

#[test]
fn gateway_identity_marker_parser_and_preimage_match_exact_golden() {
    assert_eq!(
        GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER,
        "skiff-gateway-entry-identity-v2"
    );
    assert_eq!(
        GATEWAY_ENTRY_IDENTITY_PREFIX,
        "skiff-gateway-entry-v2:sha256"
    );

    let surface = typed_http_surface();
    let bytes =
        canonical_gateway_entry_identity_bytes(&surface).expect("canonical gateway preimage");
    let preimage = String::from_utf8(bytes).expect("JSON UTF-8");
    assert_eq!(
        preimage,
        r#"{"schema":"skiff-gateway-entry-identity-v2","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"http","surface":{"adapterKind":"typedJson","dispatchMode":"unary","externalSources":[{"kind":"http.body"},{"kind":"http.request"}],"requestBodySchema":{"fields":{"query":{"kind":"string"},"requestId":{"kind":"string"}},"kind":"record","required":["query","requestId"]},"responseSchema":{"branches":[{"kind":"stringLiteral","value":"accepted"},{"kind":"stringLiteral","value":"ok"}],"kind":"closedUnion"},"streamItemSchema":null}}}}"#
    );
    let identity = gateway_entry_identity(&surface).expect("gateway identity");
    assert_eq!(
        identity.as_str(),
        "skiff-gateway-entry-v2:sha256:1ce33a44e725ea8fdea02caa1cc874567007967e3639e9c98ddeed04de5d4f5c"
    );
    assert_eq!(
        gateway_entry_identity_hash(identity.as_str()).expect("identity hash"),
        identity.as_str().rsplit_once(':').expect("framed hash").1
    );

    let digest = "a".repeat(64);
    for invalid in [
        String::new(),
        format!("skiff-gateway-v1:sha256:{digest}"),
        format!("skiff-gateway-entry-v1:sha256:{digest}"),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "a".repeat(63)),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "A".repeat(64)),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "g".repeat(64)),
    ] {
        assert!(
            gateway_entry_identity_hash(&invalid).is_err(),
            "{invalid} must be rejected"
        );
    }

    let diagnostic =
        gateway_entry_identity_hash(&format!("skiff-gateway-entry-v1:sha256:{digest}"))
            .unwrap_err()
            .to_string();
    assert!(
        diagnostic.contains(GATEWAY_ENTRY_IDENTITY_PREFIX),
        "diagnostic must reuse the canonical prefix: {diagnostic}"
    );
}

#[test]
fn websocket_gateway_and_internal_entry_id_match_language_neutral_goldens() {
    let surface = websocket_surface();
    let bytes =
        canonical_gateway_entry_identity_bytes(&surface).expect("canonical gateway preimage");
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        r#"{"schema":"skiff-gateway-entry-identity-v2","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"websocketConnect","surface":{"closeExternalSources":[],"connectRequestShape":"v1","connectResultShape":"v1","connectionCloseShape":"v1","connectionPolicyShape":"v1","downlinkFrames":["binary","text"],"externalSources":[{"kind":"websocket.connectRequest"},{"kind":"websocket.connectionId"}],"rpcProfiles":["jsonrpc-2.0-text"]}}}}"#
    );
    assert_eq!(
        gateway_entry_identity(&surface).unwrap().as_str(),
        "skiff-gateway-entry-v2:sha256:6ea166c14c3980ee9fab97561a99e4725cf3f841513e7a5b3a071611acac2319"
    );

    let key = GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap();
    let bytes = canonical_websocket_entry_id_bytes("example.com/chat", &key).unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        r#"{"gatewayEntryKey":"websocket","schema":"skiff-websocket-entry-identity-v1","serviceId":"example.com/chat"}"#
    );
    assert_eq!(
        websocket_entry_id("example.com/chat", &key).unwrap().as_str(),
        "skiff-websocket-entry-v1:sha256:3a0f9b39b684e0c324ff3f729395273987f86ed648e6c0ddd0cb35b67b1aa616"
    );
    assert_ne!(
        websocket_entry_id("example.com/chat", &key).unwrap(),
        websocket_entry_id("example.com/other-chat", &key).unwrap()
    );
    assert_ne!(
        websocket_entry_id("example.com/chat", &key).unwrap(),
        websocket_entry_id(
            "example.com/chat",
            &GatewayEntryKey::parse("other").unwrap()
        )
        .unwrap()
    );
    for service_id in ["", " ", "\n"] {
        assert!(websocket_entry_id(service_id, &key).is_err());
    }
}

#[test]
fn websocket_connect_profiles_normalize_and_loaded_sequences_are_strict() {
    let canonical = websocket_surface();
    let GatewayProtocolSurface::WebSocketConnect(connect) = &canonical.protocol else {
        panic!("expected websocketConnect")
    };
    assert_eq!(
        connect.rpc_profiles,
        vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text]
    );

    let mut duplicate = canonical.clone();
    let GatewayProtocolSurface::WebSocketConnect(connect) = &mut duplicate.protocol else {
        panic!("expected websocketConnect")
    };
    connect
        .rpc_profiles
        .push(GatewayWebSocketRpcProfile::JsonRpc2_0Text);
    assert!(
        validate_gateway_entry_protocol_surface(&duplicate).is_err(),
        "loaded duplicate profiles must not be silently repaired"
    );
    assert_eq!(
        normalize_gateway_entry_protocol_surface(duplicate).unwrap(),
        canonical,
        "producer normalization must canonicalize the closed profile set"
    );

    let mut empty = canonical.clone();
    let GatewayProtocolSurface::WebSocketConnect(connect) = &mut empty.protocol else {
        panic!("expected websocketConnect")
    };
    connect.rpc_profiles.clear();
    assert!(normalize_gateway_entry_protocol_surface(empty).is_err());

    let mut wrong_profile = serde_json::to_value(canonical).unwrap();
    wrong_profile["protocol"]["surface"]["rpcProfiles"] = json!(["future-rpc"]);
    assert!(
        serde_json::from_value::<GatewayEntryProtocolSurface>(wrong_profile).is_err(),
        "unknown profiles must fail at the strict artifact reader"
    );
}

#[test]
fn websocket_connect_close_surface_normalizes_and_rejects_foreign_sources() {
    let canonical = websocket_surface();
    let GatewayProtocolSurface::WebSocketConnect(connect) = &canonical.protocol else {
        panic!("expected websocketConnect")
    };
    assert_eq!(
        connect.connection_close_shape,
        GatewayWebSocketShapeVersion::V1
    );
    assert!(connect.close_external_sources.is_empty());

    let mut declared = canonical.clone();
    let GatewayProtocolSurface::WebSocketConnect(connect) = &mut declared.protocol else {
        panic!("expected websocketConnect")
    };
    connect.close_external_sources = vec![
        GatewayAdapterSource::WebSocketCloseReason,
        GatewayAdapterSource::WebSocketConnectionId,
        GatewayAdapterSource::WebSocketCloseCode,
        GatewayAdapterSource::WebSocketBusinessIdentity,
        GatewayAdapterSource::WebSocketCloseCode,
    ];
    assert!(
        validate_gateway_entry_protocol_surface(&declared).is_err(),
        "loaded close sources must already be canonical"
    );
    let normalized = normalize_gateway_entry_protocol_surface(declared).unwrap();
    let GatewayProtocolSurface::WebSocketConnect(connect) = &normalized.protocol else {
        panic!("expected websocketConnect")
    };
    assert_eq!(
        connect.close_external_sources,
        vec![
            GatewayAdapterSource::WebSocketBusinessIdentity,
            GatewayAdapterSource::WebSocketCloseCode,
            GatewayAdapterSource::WebSocketCloseReason,
            GatewayAdapterSource::WebSocketConnectionId,
        ]
    );
    assert_ne!(
        gateway_entry_identity(&normalized).unwrap(),
        gateway_entry_identity(&canonical).unwrap(),
        "declared close sources must enter the connect entry identity"
    );

    let mut wrong_phase = canonical.clone();
    let GatewayProtocolSurface::WebSocketConnect(connect) = &mut wrong_phase.protocol else {
        panic!("expected websocketConnect")
    };
    connect.close_external_sources = vec![GatewayAdapterSource::WebSocketConnectRequest];
    assert!(normalize_gateway_entry_protocol_surface(wrong_phase).is_err());
}

#[test]
fn websocket_json_rpc_surface_is_canonical_structured_and_phase_exact() {
    let canonical = websocket_json_rpc_surface(
        vec![
            GatewayAdapterSource::WebSocketJsonRpcParams,
            GatewayAdapterSource::WebSocketBusinessIdentity,
            GatewayAdapterSource::WebSocketConnectionId,
        ],
        string_record(&["requestId"], vec!["requestId".to_string()]),
        GatewayExternalSchema::Null,
    );
    let GatewayProtocolSurface::WebSocketJsonRpc(json_rpc) = &canonical.protocol else {
        panic!("expected websocketJsonRpc")
    };
    assert_eq!(
        json_rpc.external_sources,
        vec![
            GatewayAdapterSource::WebSocketBusinessIdentity,
            GatewayAdapterSource::WebSocketConnectionId,
            GatewayAdapterSource::WebSocketJsonRpcParams,
        ]
    );
    assert_eq!(json_rpc.result_schema, GatewayExternalSchema::Null);

    let mut noncanonical = canonical.clone();
    let json_rpc = websocket_json_rpc_surface_mut(&mut noncanonical);
    json_rpc.external_sources.reverse();
    assert!(
        validate_gateway_entry_protocol_surface(&noncanonical).is_err(),
        "loaded source order must already be canonical"
    );
    assert_eq!(
        normalize_gateway_entry_protocol_surface(noncanonical).unwrap(),
        canonical
    );

    let mut duplicate = canonical.clone();
    websocket_json_rpc_surface_mut(&mut duplicate)
        .external_sources
        .push(GatewayAdapterSource::WebSocketJsonRpcParams);
    assert!(
        validate_gateway_entry_protocol_surface(&duplicate).is_err(),
        "loaded duplicate sources must fail closed"
    );

    let mut missing_params = canonical.clone();
    websocket_json_rpc_surface_mut(&mut missing_params)
        .external_sources
        .retain(|source| *source != GatewayAdapterSource::WebSocketJsonRpcParams);
    assert!(normalize_gateway_entry_protocol_surface(missing_params).is_err());

    let mut wrong_phase = canonical.clone();
    websocket_json_rpc_surface_mut(&mut wrong_phase)
        .external_sources
        .push(GatewayAdapterSource::WebSocketConnectRequest);
    assert!(normalize_gateway_entry_protocol_surface(wrong_phase).is_err());

    let mut wrong_dispatch = canonical.clone();
    websocket_json_rpc_surface_mut(&mut wrong_dispatch).dispatch_mode =
        GatewayDispatchMode::ServerStream;
    assert!(normalize_gateway_entry_protocol_surface(wrong_dispatch).is_err());

    for invalid_params in [
        GatewayExternalSchema::Null,
        GatewayExternalSchema::String,
        GatewayExternalSchema::Nullable {
            inner: Box::new(string_record(&["id"], vec!["id".to_string()])),
        },
        GatewayExternalSchema::ClosedUnion {
            branches: vec![
                string_record(&["id"], vec!["id".to_string()]),
                GatewayExternalSchema::String,
            ],
        },
    ] {
        let mut invalid = canonical.clone();
        websocket_json_rpc_surface_mut(&mut invalid).params_schema = invalid_params;
        assert!(
            normalize_gateway_entry_protocol_surface(invalid).is_err(),
            "JSON-RPC params must remain object/array structured"
        );
    }

    let structured_union = websocket_json_rpc_surface(
        vec![GatewayAdapterSource::WebSocketJsonRpcParams],
        GatewayExternalSchema::ClosedUnion {
            branches: vec![
                string_record(&["id"], vec!["id".to_string()]),
                GatewayExternalSchema::Array {
                    items: Box::new(GatewayExternalSchema::String),
                },
            ],
        },
        GatewayExternalSchema::String,
    );
    validate_gateway_entry_protocol_surface(&structured_union).unwrap();

    let mut wrong_profile = serde_json::to_value(canonical).unwrap();
    wrong_profile["protocol"]["surface"]["profile"] = json!("future-rpc");
    assert!(
        serde_json::from_value::<GatewayEntryProtocolSurface>(wrong_profile).is_err(),
        "unknown JSON-RPC profiles must fail at the strict artifact reader"
    );
}

#[test]
fn websocket_json_rpc_identity_tracks_only_the_canonical_protocol_surface() {
    let base = websocket_json_rpc_surface(
        vec![
            GatewayAdapterSource::WebSocketJsonRpcParams,
            GatewayAdapterSource::WebSocketConnectionId,
        ],
        string_record(&["id"], vec!["id".to_string()]),
        string_record(&["value"], vec!["value".to_string()]),
    );
    let base_identity = gateway_entry_identity(&base).unwrap();
    let preimage =
        String::from_utf8(canonical_gateway_entry_identity_bytes(&base).unwrap()).unwrap();
    assert_eq!(
        preimage,
        r#"{"schema":"skiff-gateway-entry-identity-v2","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"websocketJsonRpc","surface":{"dispatchMode":"unary","externalSources":[{"kind":"websocket.connectionId"},{"kind":"websocket.jsonRpcParams"}],"paramsSchema":{"fields":{"id":{"kind":"string"}},"kind":"record","required":["id"]},"profile":"jsonrpc-2.0-text","resultSchema":{"fields":{"value":{"kind":"string"}},"kind":"record","required":["value"]}}}}}"#
    );
    assert_eq!(
        base_identity.as_str(),
        "skiff-gateway-entry-v2:sha256:76fd205e35d35474a2082dd58b914b25b653eeecbfd8b6c96c52d3d070eae331"
    );

    let reordered = websocket_json_rpc_surface(
        vec![
            GatewayAdapterSource::WebSocketConnectionId,
            GatewayAdapterSource::WebSocketJsonRpcParams,
        ],
        string_record(&["id"], vec!["id".to_string()]),
        string_record(&["value"], vec!["value".to_string()]),
    );
    assert_eq!(
        gateway_entry_identity(&reordered).unwrap(),
        base_identity,
        "formal parameter/source order is deployment-only"
    );

    let source_changed = websocket_json_rpc_surface(
        vec![
            GatewayAdapterSource::WebSocketJsonRpcParams,
            GatewayAdapterSource::WebSocketBusinessIdentity,
            GatewayAdapterSource::WebSocketConnectionId,
        ],
        string_record(&["id"], vec!["id".to_string()]),
        string_record(&["value"], vec!["value".to_string()]),
    );
    let params_changed = websocket_json_rpc_surface(
        vec![
            GatewayAdapterSource::WebSocketJsonRpcParams,
            GatewayAdapterSource::WebSocketConnectionId,
        ],
        string_record(&["requestId"], vec!["requestId".to_string()]),
        string_record(&["value"], vec!["value".to_string()]),
    );
    let result_changed = websocket_json_rpc_surface(
        vec![
            GatewayAdapterSource::WebSocketJsonRpcParams,
            GatewayAdapterSource::WebSocketConnectionId,
        ],
        string_record(&["id"], vec!["id".to_string()]),
        GatewayExternalSchema::Null,
    );
    for changed in [source_changed, params_changed, result_changed] {
        assert_ne!(gateway_entry_identity(&changed).unwrap(), base_identity);
    }

    for deployment_only in [
        "status.get",
        "status-entry",
        "pkg-callable:example.provider:status",
        "connectionFormal",
        "skiff-package-build-v13",
        "example.internal.Nominal",
    ] {
        assert!(
            !preimage.contains(deployment_only),
            "{deployment_only} leaked into {preimage}"
        );
    }
}

#[test]
fn gateway_http_identity_includes_kind_mode_external_sources_and_schemas() {
    let base = typed_http_surface();
    let base_identity = gateway_entry_identity(&base).expect("base identity");

    let raw = raw_http_surface(GatewayDispatchMode::Unary);
    assert_ne!(
        base_identity,
        gateway_entry_identity(&raw).expect("raw identity")
    );
    assert_ne!(
        gateway_entry_identity(&raw).expect("raw unary identity"),
        gateway_entry_identity(&raw_http_surface(GatewayDispatchMode::ServerStream))
            .expect("raw stream identity")
    );

    let mutate = |mut surface: GatewayEntryProtocolSurface,
                  apply: fn(&mut GatewayHttpProtocolSurface)| {
        let http = http_surface_mut(&mut surface);
        apply(http);
        normalize_gateway_entry_protocol_surface(surface).expect("valid mutation")
    };
    let body_changed = mutate(base.clone(), |http| {
        http.request_body_schema = Some(GatewayExternalSchema::Integer);
    });
    let response_changed = mutate(base.clone(), |http| {
        http.response_schema = Some(GatewayExternalSchema::Boolean);
    });
    let sources_changed = mutate(base.clone(), |http| {
        http.external_sources = vec![GatewayAdapterSource::HttpBody];
    });
    for changed in [body_changed, response_changed, sources_changed] {
        assert_ne!(
            base_identity,
            gateway_entry_identity(&changed).expect("mutated identity")
        );
    }
}

#[test]
fn gateway_error_projection_version_is_framed_and_unknown_version_fails_closed() {
    let surface = typed_http_surface();
    let identity = gateway_entry_identity(&surface).expect("base identity");
    let projection =
        gateway_entry_identity_projection(&surface).expect("gateway identity projection");
    let mut mutated = serde_json::to_value(projection).expect("projection JSON");
    mutated["surface"]["externalErrorProjection"]["version"] = json!("v2");
    let bytes = skiff_canonical_json::canonical_json_bytes(&mutated)
        .expect("mutated canonical preimage bytes");
    let mutated_identity = skiff_artifact_model::GatewayEntryIdentity::parse(format!(
        "{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}",
        hex::encode(Sha256::digest(bytes))
    ))
    .expect("well-framed mutation identity");
    assert_ne!(identity, mutated_identity);

    let mut wire = serde_json::to_value(&surface).expect("surface JSON");
    wire["externalErrorProjection"]["version"] = json!("v2");
    assert!(
        serde_json::from_value::<GatewayEntryProtocolSurface>(wire).is_err(),
        "unknown error projection generations must not load"
    );
}

#[test]
fn gateway_identity_excludes_keys_selectors_targets_builds_params_and_internal_context() {
    let surface = typed_http_surface();
    let expected = gateway_entry_identity(&surface).expect("identity");

    let keys = [
        GatewayEntryKey::parse("first").expect("key"),
        GatewayEntryKey::parse("second").expect("key"),
    ];
    let selectors = ["POST api.example.test/v1", "GET other.example.test/v2"];
    let handlers = ["pkg-callable:first", "pkg-callable:replacement"];
    let builds = [
        "skiff-package-build-v6:first",
        "skiff-package-build-v6:next",
    ];
    let args = [
        GatewayAdapterArg {
            param: "body".to_string(),
            source: GatewayAdapterSource::HttpBody,
        },
        GatewayAdapterArg {
            param: "renamedBody".to_string(),
            source: GatewayAdapterSource::HttpBody,
        },
    ];
    let context_codec_ids = ["context-codec:first", "context-codec:replacement"];
    for _excluded_combination in [
        (
            keys[0].as_str(),
            selectors[0],
            handlers[0],
            builds[0],
            &args[0],
            context_codec_ids[0],
        ),
        (
            keys[1].as_str(),
            selectors[1],
            handlers[1],
            builds[1],
            &args[1],
            context_codec_ids[1],
        ),
    ] {
        assert_eq!(
            expected,
            gateway_entry_identity(&surface).expect("identity ignores deployment facts")
        );
    }

    let preimage = String::from_utf8(
        canonical_gateway_entry_identity_bytes(&surface).expect("canonical preimage"),
    )
    .expect("preimage UTF-8");
    for forbidden in [
        "gatewayEntryKey",
        "selector",
        "host",
        "method",
        "path",
        "handler",
        "packageCallableId",
        "packageBuild",
        "deployment",
        "param",
        "contextCodec",
        "typeRefIr",
        "packageSchemaTypeId",
        "publicPath",
        "sourcePath",
        "nominal",
    ] {
        assert!(
            !preimage.contains(forbidden),
            "{forbidden} leaked into {preimage}"
        );
    }
}

#[test]
fn gateway_normalizer_canonicalizes_order_while_loaded_artifacts_reject_it() {
    let canonical = typed_http_surface();
    let expected = gateway_entry_identity(&canonical).expect("canonical identity");

    let mut reordered = canonical.clone();
    let http = http_surface_mut(&mut reordered);
    http.external_sources.reverse();
    let GatewayExternalSchema::Record { required, .. } =
        http.request_body_schema.as_mut().expect("body schema")
    else {
        panic!("record body");
    };
    required.reverse();
    let GatewayExternalSchema::ClosedUnion { branches } =
        http.response_schema.as_mut().expect("response schema")
    else {
        panic!("union response");
    };
    branches.reverse();

    assert!(
        validate_gateway_entry_protocol_surface(&reordered).is_err(),
        "loaded artifacts must not be repaired silently"
    );
    let normalized =
        normalize_gateway_entry_protocol_surface(reordered).expect("producer normalization");
    assert_eq!(canonical, normalized);
    assert_eq!(
        expected,
        gateway_entry_identity(&normalized).expect("normalized identity")
    );

    let differently_inserted = string_record(
        &["requestId", "query"],
        vec!["query".to_string(), "requestId".to_string()],
    );
    assert_eq!(
        differently_inserted,
        canonical_http_body(&canonical).clone(),
        "BTreeMap insertion order is not semantic"
    );
}

fn canonical_http_body(surface: &GatewayEntryProtocolSurface) -> &GatewayExternalSchema {
    let http = http_surface(surface);
    http.request_body_schema.as_ref().expect("body schema")
}

#[test]
fn gateway_validation_rejects_invalid_http_and_non_http_combinations() {
    let invalid_http = |mutate: fn(&mut GatewayHttpProtocolSurface)| {
        let mut surface = typed_http_surface();
        let http = http_surface_mut(&mut surface);
        mutate(http);
        normalize_gateway_entry_protocol_surface(surface).is_err()
    };
    assert!(invalid_http(|http| http.request_body_schema = None));
    assert!(invalid_http(|http| http.response_schema = None));
    assert!(invalid_http(|http| {
        http.external_sources = vec![GatewayAdapterSource::HttpContext]
    }));
    assert!(invalid_http(|http| {
        http.stream_item_schema = Some(GatewayExternalSchema::String)
    }));

    let mut raw = raw_http_surface(GatewayDispatchMode::Unary);
    let raw_http = http_surface_mut(&mut raw);
    raw_http.request_body_schema = Some(GatewayExternalSchema::String);
    assert!(normalize_gateway_entry_protocol_surface(raw).is_err());

    let mut stream = raw_http_surface(GatewayDispatchMode::ServerStream);
    let stream_http = http_surface_mut(&mut stream);
    stream_http.stream_item_schema = None;
    assert!(normalize_gateway_entry_protocol_surface(stream).is_err());

    let mut wire = serde_json::to_value(typed_http_surface()).expect("HTTP JSON");
    wire["protocol"]["surface"]["adapterKind"] = json!("websocketReceive");
    assert!(
        serde_json::from_value::<GatewayEntryProtocolSurface>(wire).is_err(),
        "non-HTTP adapter kinds must not load"
    );
    let mut wire = serde_json::to_value(typed_http_surface()).expect("HTTP JSON");
    wire["protocol"]["surface"]["adapterKind"] = json!("websocketConnectionClosed");
    let surface: GatewayEntryProtocolSurface =
        serde_json::from_value(wire).expect("connection-closed kind must deserialize");
    assert!(
        normalize_gateway_entry_protocol_surface(surface)
            .expect_err("connection-closed kind must not reach an HTTP surface")
            .to_string()
            .contains("websocketConnectionClosed"),
        "HTTP surface rejection must name the connection-closed kind"
    );
    let mut wire = serde_json::to_value(typed_http_surface()).expect("HTTP JSON");
    wire["protocol"]["surface"]["externalSources"] = json!([{ "kind": "websocket.message" }]);
    assert!(
        serde_json::from_value::<GatewayEntryProtocolSurface>(wire).is_err(),
        "non-HTTP sources must not load"
    );
}

#[test]
fn gateway_external_schema_rejects_duplicates_missing_required_and_noncanonical_shape() {
    let duplicate_required = GatewayExternalSchema::Record {
        fields: BTreeMap::from([("id".to_string(), GatewayExternalSchema::String)]),
        required: vec!["id".to_string(), "id".to_string()],
    };
    assert!(normalize_gateway_external_schema(duplicate_required).is_err());

    let missing_required = GatewayExternalSchema::Record {
        fields: BTreeMap::new(),
        required: vec!["id".to_string()],
    };
    assert!(normalize_gateway_external_schema(missing_required).is_err());

    let invalid_required = GatewayExternalSchema::Record {
        fields: BTreeMap::from([("".to_string(), GatewayExternalSchema::String)]),
        required: vec!["".to_string()],
    };
    assert!(normalize_gateway_external_schema(invalid_required).is_err());

    let duplicate_union = GatewayExternalSchema::ClosedUnion {
        branches: vec![GatewayExternalSchema::String, GatewayExternalSchema::String],
    };
    assert!(normalize_gateway_external_schema(duplicate_union).is_err());

    let noncanonical_nullable_union = GatewayExternalSchema::ClosedUnion {
        branches: vec![GatewayExternalSchema::Null, GatewayExternalSchema::String],
    };
    assert_eq!(
        normalize_gateway_external_schema(noncanonical_nullable_union)
            .expect("producer canonical nullable"),
        GatewayExternalSchema::Nullable {
            inner: Box::new(GatewayExternalSchema::String)
        }
    );
}
