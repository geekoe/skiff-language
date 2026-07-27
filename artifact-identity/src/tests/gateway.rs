use std::collections::BTreeMap;

use serde_json::json;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    GatewayAdapterArg, GatewayAdapterKind, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayExternalErrorProjection,
    GatewayExternalSchema, GatewayHttpProtocolSurface, GatewayProtocolSurface,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
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
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .expect("canonical WebSocket connect surface")
}

fn http_surface(surface: &GatewayEntryProtocolSurface) -> &GatewayHttpProtocolSurface {
    match &surface.protocol {
        GatewayProtocolSurface::Http(http) => http,
        GatewayProtocolSurface::WebSocketConnect(_) => {
            panic!("HTTP surface helper received websocketConnect")
        }
    }
}

fn http_surface_mut(surface: &mut GatewayEntryProtocolSurface) -> &mut GatewayHttpProtocolSurface {
    match &mut surface.protocol {
        GatewayProtocolSurface::Http(http) => http,
        GatewayProtocolSurface::WebSocketConnect(_) => {
            panic!("HTTP surface helper received websocketConnect")
        }
    }
}

#[test]
fn gateway_identity_marker_parser_and_preimage_match_exact_golden() {
    assert_eq!(
        GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER,
        "skiff-gateway-entry-identity-v1"
    );
    assert_eq!(
        GATEWAY_ENTRY_IDENTITY_PREFIX,
        "skiff-gateway-entry-v1:sha256"
    );

    let surface = typed_http_surface();
    let bytes =
        canonical_gateway_entry_identity_bytes(&surface).expect("canonical gateway preimage");
    let preimage = String::from_utf8(bytes).expect("JSON UTF-8");
    assert_eq!(
        preimage,
        r#"{"schema":"skiff-gateway-entry-identity-v1","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"http","surface":{"adapterKind":"typedJson","dispatchMode":"unary","externalSources":[{"kind":"http.body"},{"kind":"http.request"}],"requestBodySchema":{"fields":{"query":{"kind":"string"},"requestId":{"kind":"string"}},"kind":"record","required":["query","requestId"]},"responseSchema":{"branches":[{"kind":"stringLiteral","value":"accepted"},{"kind":"stringLiteral","value":"ok"}],"kind":"closedUnion"},"streamItemSchema":null}}}}"#
    );
    let identity = gateway_entry_identity(&surface).expect("gateway identity");
    assert_eq!(
        identity.as_str(),
        "skiff-gateway-entry-v1:sha256:a24d48c28b531ef534b0ffcbff94554c505caab62f0a9de1cd47c4ab0ec4f685"
    );
    assert_eq!(
        gateway_entry_identity_hash(identity.as_str()).expect("identity hash"),
        identity.as_str().rsplit_once(':').expect("framed hash").1
    );

    let digest = "a".repeat(64);
    for invalid in [
        String::new(),
        format!("skiff-gateway-v1:sha256:{digest}"),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "a".repeat(63)),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "A".repeat(64)),
        format!("{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}", "g".repeat(64)),
    ] {
        assert!(
            gateway_entry_identity_hash(&invalid).is_err(),
            "{invalid} must be rejected"
        );
    }
}

#[test]
fn websocket_gateway_and_internal_entry_id_match_language_neutral_goldens() {
    let surface = websocket_surface();
    let bytes =
        canonical_gateway_entry_identity_bytes(&surface).expect("canonical gateway preimage");
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        r#"{"schema":"skiff-gateway-entry-identity-v1","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"websocketConnect","surface":{"connectRequestShape":"v1","connectResultShape":"v1","connectionPolicyShape":"v1","downlinkFrames":["binary","text"],"externalSources":[{"kind":"websocket.connectRequest"},{"kind":"websocket.connectionId"}]}}}}"#
    );
    assert_eq!(
        gateway_entry_identity(&surface).unwrap().as_str(),
        "skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936"
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
