use std::collections::HashMap;

use serde_json::Value;
use skiff_runtime_request_contract::{
    BinaryHttpRequest, BinaryHttpRequestMetadata, GatewayAdapterArg, GatewayAdapterSource,
    HttpAdapter, HttpAdapterCallable, HttpAdapterKind, HttpNameValue, RequestCancel,
    RequestEffectDouble, RequestEnvelope, WebSocketAdapter, WebSocketAdapterKind,
    WebSocketConnectRequest, WebSocketContextCodec, WebSocketContextExpectation, WebSocketMessage,
    WebSocketMessageEncoding, WebSocketMessageTag, WebSocketPayloadSegment,
    WebSocketPayloadSegmentKind, WebSocketReceiveRequest,
};

use crate::protocol::{
    RequestCancelFrameHeader, RequestStartFrameHeader, RequestTestEffectDouble,
    RuntimeGatewayAdapterArgFrameHeader, RuntimeGatewayAdapterSourceFrameHeader,
    RuntimeHttpAdapterCallableFrameHeader, RuntimeHttpAdapterFrameHeader,
    RuntimeHttpAdapterKindFrameHeader, RuntimeHttpNameValueFrameHeader,
    RuntimeHttpRequestFrameHeader, RuntimeWebSocketAdapterFrameHeader,
    RuntimeWebSocketAdapterKindFrameHeader, RuntimeWebSocketConnectRequestFrameHeader,
    RuntimeWebSocketContextCodecFrameHeader, RuntimeWebSocketContextExpectationFrameHeader,
    RuntimeWebSocketMessageEncodingFrameHeader, RuntimeWebSocketMessageFrameHeader,
    RuntimeWebSocketMessageTagFrameHeader, RuntimeWebSocketPayloadSegmentFrameHeader,
    RuntimeWebSocketPayloadSegmentKindFrameHeader, RuntimeWebSocketReceiveRequestFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

pub fn request_envelope_from_start_frame(
    header: RequestStartFrameHeader,
    payload_bytes: Vec<u8>,
) -> Result<RequestEnvelope, String> {
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(format!(
            "request.start schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}, got {}",
            header.schema_version
        ));
    }
    if header.envelope_type != "request.start" {
        return Err(format!(
            "binary frame type must be request.start, got {}",
            header.envelope_type
        ));
    }
    if header.build_id.is_empty() {
        return Err("request.start buildId must be a non-empty string".to_string());
    }
    Ok(RequestEnvelope {
        request_id: header.request_id.clone(),
        mode: header.mode.clone(),
        target: header.target.clone(),
        operation_abi_id: header.operation_abi_id.clone(),
        selector: header.selector.clone(),
        service_id: header.service_id.clone(),
        build_id: header.build_id.clone(),
        service_protocol_identity: header.service_protocol_identity.clone(),
        contract_identity: None,
        activation_identity: header.activation_identity.clone(),
        binary_http: binary_http_request_from_frame(header.http_request.clone(), &payload_bytes),
        http_adapter: http_adapter_from_frame(header.http_adapter.clone()),
        websocket_adapter: websocket_adapter_from_frame(header.websocket_adapter.clone()),
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: request_effect_doubles_from_frame(&header.test_effect_doubles),
        payload_bytes,
        extra: request_start_extra_from_frame(&header),
    })
}

pub fn request_cancel_from_frame_header(header: RequestCancelFrameHeader) -> RequestCancel {
    RequestCancel {
        request_id: header.request_id,
        reason: Some(header.reason),
    }
}

fn binary_http_request_from_frame(
    metadata: Option<RuntimeHttpRequestFrameHeader>,
    payload_bytes: &[u8],
) -> Option<BinaryHttpRequest> {
    metadata.map(|metadata| BinaryHttpRequest {
        metadata: BinaryHttpRequestMetadata {
            method: metadata.method,
            url: metadata.url,
            path: metadata.path,
            query: http_name_values_from_frame(metadata.query),
            headers: http_name_values_from_frame(metadata.headers),
        },
        body: payload_bytes.to_vec(),
    })
}

fn http_name_values_from_frame(items: Vec<RuntimeHttpNameValueFrameHeader>) -> Vec<HttpNameValue> {
    items
        .into_iter()
        .map(|item| HttpNameValue {
            name: item.name,
            value: item.value,
        })
        .collect()
}

fn http_adapter_from_frame(metadata: Option<RuntimeHttpAdapterFrameHeader>) -> Option<HttpAdapter> {
    metadata.map(|metadata| HttpAdapter {
        kind: match metadata.kind {
            RuntimeHttpAdapterKindFrameHeader::TypedJson => HttpAdapterKind::TypedJson,
            RuntimeHttpAdapterKindFrameHeader::RawHttp => HttpAdapterKind::RawHttp,
        },
        handler: http_adapter_callable_from_frame(metadata.handler),
        guard: metadata.guard.map(http_adapter_callable_from_frame),
        pre: metadata.pre.map(http_adapter_callable_from_frame),
        adapter_args: gateway_adapter_args_from_frame(metadata.adapter_args),
    })
}

fn http_adapter_callable_from_frame(
    callable: RuntimeHttpAdapterCallableFrameHeader,
) -> HttpAdapterCallable {
    match callable {
        RuntimeHttpAdapterCallableFrameHeader::ServiceFunction {
            module_path,
            symbol,
        } => HttpAdapterCallable::ServiceFunction {
            module_path,
            symbol,
        },
        RuntimeHttpAdapterCallableFrameHeader::PackageFunction {
            package_id,
            symbol_path,
        } => HttpAdapterCallable::PackageFunction {
            package_id,
            symbol_path,
        },
    }
}

fn gateway_adapter_args_from_frame(
    args: Vec<RuntimeGatewayAdapterArgFrameHeader>,
) -> Vec<GatewayAdapterArg> {
    args.into_iter()
        .map(|arg| GatewayAdapterArg {
            param: arg.param,
            source: gateway_adapter_source_from_frame(arg.source),
        })
        .collect()
}

fn gateway_adapter_source_from_frame(
    source: RuntimeGatewayAdapterSourceFrameHeader,
) -> GatewayAdapterSource {
    match source {
        RuntimeGatewayAdapterSourceFrameHeader::HttpRequest => GatewayAdapterSource::HttpRequest,
        RuntimeGatewayAdapterSourceFrameHeader::HttpBody => GatewayAdapterSource::HttpBody,
        RuntimeGatewayAdapterSourceFrameHeader::HttpContext => GatewayAdapterSource::HttpContext,
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketConnectRequest => {
            GatewayAdapterSource::WebSocketConnectRequest
        }
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketReceiveEvent => {
            GatewayAdapterSource::WebSocketReceiveEvent
        }
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketConnection => {
            GatewayAdapterSource::WebSocketConnection
        }
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketConnectionContext => {
            GatewayAdapterSource::WebSocketConnectionContext
        }
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketMessage => {
            GatewayAdapterSource::WebSocketMessage
        }
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketMessageBody => {
            GatewayAdapterSource::WebSocketMessageBody
        }
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketConnectionId => {
            GatewayAdapterSource::WebSocketConnectionId
        }
        RuntimeGatewayAdapterSourceFrameHeader::WebSocketBusinessIdentity => {
            GatewayAdapterSource::WebSocketBusinessIdentity
        }
    }
}

fn websocket_adapter_from_frame(
    metadata: Option<RuntimeWebSocketAdapterFrameHeader>,
) -> Option<WebSocketAdapter> {
    metadata.map(|metadata| WebSocketAdapter {
        kind: match metadata.kind {
            RuntimeWebSocketAdapterKindFrameHeader::Connect => WebSocketAdapterKind::Connect,
            RuntimeWebSocketAdapterKindFrameHeader::Receive => WebSocketAdapterKind::Receive,
        },
        adapter_args: gateway_adapter_args_from_frame(metadata.adapter_args),
        context_expectation: metadata
            .context_expectation
            .map(websocket_context_expectation_from_frame),
        connect_request: metadata
            .connect_request
            .map(websocket_connect_request_from_frame),
        receive_request: metadata
            .receive_request
            .map(websocket_receive_request_from_frame),
    })
}

fn websocket_context_expectation_from_frame(
    expectation: RuntimeWebSocketContextExpectationFrameHeader,
) -> WebSocketContextExpectation {
    match expectation {
        RuntimeWebSocketContextExpectationFrameHeader::Null => WebSocketContextExpectation::Null,
        RuntimeWebSocketContextExpectationFrameHeader::Typed {
            connect_operation_abi_id,
            context_type_identity,
        } => WebSocketContextExpectation::Typed {
            connect_operation_abi_id,
            context_type_identity,
        },
    }
}

fn websocket_context_codec_from_frame(
    codec: RuntimeWebSocketContextCodecFrameHeader,
) -> WebSocketContextCodec {
    WebSocketContextCodec {
        operation_abi_id: codec.operation_abi_id,
        context_type_identity: codec.context_type_identity,
    }
}

fn websocket_connect_request_from_frame(
    request: RuntimeWebSocketConnectRequestFrameHeader,
) -> WebSocketConnectRequest {
    WebSocketConnectRequest {
        connection_id: request.connection_id,
        url: request.url,
        query: http_name_values_from_frame(request.query),
        headers: http_name_values_from_frame(request.headers),
        cookies: http_name_values_from_frame(request.cookies),
        version: request.version,
    }
}

fn websocket_receive_request_from_frame(
    request: RuntimeWebSocketReceiveRequestFrameHeader,
) -> WebSocketReceiveRequest {
    WebSocketReceiveRequest {
        connection_id: request.connection_id,
        business_identity: request.business_identity,
        message: websocket_message_from_frame(request.message),
        context_codec: request
            .context_codec
            .map(websocket_context_codec_from_frame),
        payload_segments: request
            .payload_segments
            .into_iter()
            .map(websocket_payload_segment_from_frame)
            .collect(),
    }
}

fn websocket_message_from_frame(message: RuntimeWebSocketMessageFrameHeader) -> WebSocketMessage {
    WebSocketMessage {
        tag: match message.tag {
            RuntimeWebSocketMessageTagFrameHeader::Text => WebSocketMessageTag::Text,
            RuntimeWebSocketMessageTagFrameHeader::Binary => WebSocketMessageTag::Binary,
        },
        encoding: match message.encoding {
            RuntimeWebSocketMessageEncodingFrameHeader::Utf8 => WebSocketMessageEncoding::Utf8,
            RuntimeWebSocketMessageEncodingFrameHeader::Raw => WebSocketMessageEncoding::Raw,
        },
    }
}

fn websocket_payload_segment_from_frame(
    segment: RuntimeWebSocketPayloadSegmentFrameHeader,
) -> WebSocketPayloadSegment {
    WebSocketPayloadSegment {
        kind: match segment.kind {
            RuntimeWebSocketPayloadSegmentKindFrameHeader::Context => {
                WebSocketPayloadSegmentKind::Context
            }
            RuntimeWebSocketPayloadSegmentKindFrameHeader::Message => {
                WebSocketPayloadSegmentKind::Message
            }
        },
        offset: segment.offset,
        length: segment.length,
    }
}

fn request_effect_doubles_from_frame(
    doubles: &HashMap<String, Vec<RequestTestEffectDouble>>,
) -> HashMap<String, Vec<RequestEffectDouble>> {
    doubles
        .iter()
        .map(|(target, sequence)| {
            (
                target.clone(),
                sequence
                    .iter()
                    .map(|double| RequestEffectDouble {
                        expect_request: double.expect_request.clone(),
                        response: double.response.clone(),
                    })
                    .collect(),
            )
        })
        .collect()
}

fn request_start_extra_from_frame(
    header: &RequestStartFrameHeader,
) -> serde_json::Map<String, Value> {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "caller".to_string(),
        serde_json::to_value(&header.caller).unwrap_or(Value::Null),
    );
    if let Some(gateway_entry_identity) = &header.gateway_entry_identity {
        extra.insert(
            "gatewayEntryIdentity".to_string(),
            Value::String(gateway_entry_identity.clone()),
        );
    }
    if let Some(service_id) = &header.service_id {
        extra.insert("serviceId".to_string(), Value::String(service_id.clone()));
    }
    if let Some(operation_abi_id) = &header.operation_abi_id {
        extra.insert(
            "operationAbiId".to_string(),
            Value::String(operation_abi_id.clone()),
        );
    }
    if let Some(selector) = &header.selector {
        extra.insert("selector".to_string(), Value::String(selector.clone()));
    }
    if let Some(business_identity) = &header.business_identity {
        extra.insert(
            "businessIdentity".to_string(),
            Value::String(business_identity.clone()),
        );
    }
    if let Some(websocket_entry_id) = &header.websocket_entry_id {
        extra.insert(
            "websocketEntryId".to_string(),
            Value::String(websocket_entry_id.clone()),
        );
    }
    if let Some(websocket_adapter) = &header.websocket_adapter {
        extra.insert(
            "websocketAdapter".to_string(),
            serde_json::to_value(websocket_adapter).unwrap_or(Value::Null),
        );
    }
    if let Some(client_session) = &header.client_session {
        extra.insert(
            "clientSession".to_string(),
            serde_json::to_value(client_session).unwrap_or(Value::Null),
        );
    }
    if let Some(deadline) = &header.deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline).unwrap_or(Value::Null),
        );
    }
    extra.insert(
        "trace".to_string(),
        serde_json::to_value(&header.trace).unwrap_or(Value::Null),
    );
    extra
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{request_cancel_from_frame_header, request_envelope_from_start_frame};
    use crate::protocol::{
        RequestCancelFrameHeader, RequestStartFrameHeader, RequestTestEffectDouble,
        RuntimeCallerFrameHeader, RuntimeGatewayAdapterArgFrameHeader,
        RuntimeGatewayAdapterSourceFrameHeader, RuntimeHttpAdapterCallableFrameHeader,
        RuntimeHttpAdapterFrameHeader, RuntimeHttpAdapterKindFrameHeader,
        RuntimeHttpNameValueFrameHeader, RuntimeHttpRequestFrameHeader,
        RuntimeTraceContextFrameHeader, RuntimeWebSocketAdapterFrameHeader,
        RuntimeWebSocketAdapterKindFrameHeader, RuntimeWebSocketConnectRequestFrameHeader,
        RuntimeWebSocketContextCodecFrameHeader, RuntimeWebSocketContextExpectationFrameHeader,
        RuntimeWebSocketMessageEncodingFrameHeader, RuntimeWebSocketMessageFrameHeader,
        RuntimeWebSocketMessageTagFrameHeader, RuntimeWebSocketPayloadSegmentFrameHeader,
        RuntimeWebSocketPayloadSegmentKindFrameHeader, RuntimeWebSocketReceiveRequestFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    };
    use skiff_runtime_request_contract::{
        GatewayAdapterSource, HttpAdapterCallable, HttpAdapterKind, RuntimeClientSessionControl,
        WebSocketAdapterKind, WebSocketContextExpectation, WebSocketMessageEncoding,
        WebSocketMessageTag, WebSocketPayloadSegmentKind,
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
                business_identity: Some("business-1".to_string()),
                websocket_entry_id: Some("ws-entry-1".to_string()),
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
                websocket_adapter: Some(RuntimeWebSocketAdapterFrameHeader {
                    kind: RuntimeWebSocketAdapterKindFrameHeader::Receive,
                    adapter_args: vec![RuntimeGatewayAdapterArgFrameHeader {
                        param: "message".to_string(),
                        source: RuntimeGatewayAdapterSourceFrameHeader::WebSocketMessageBody,
                    }],
                    context_expectation: Some(
                        RuntimeWebSocketContextExpectationFrameHeader::Typed {
                            connect_operation_abi_id: "connect-op".to_string(),
                            context_type_identity: "context-type".to_string(),
                        },
                    ),
                    connect_request: Some(RuntimeWebSocketConnectRequestFrameHeader {
                        connection_id: "conn-1".to_string(),
                        url: "wss://example.com/socket".to_string(),
                        query: vec![RuntimeHttpNameValueFrameHeader {
                            name: "token".to_string(),
                            value: "abc".to_string(),
                        }],
                        headers: Vec::new(),
                        cookies: Vec::new(),
                        version: Some("13".to_string()),
                    }),
                    receive_request: Some(RuntimeWebSocketReceiveRequestFrameHeader {
                        connection_id: "conn-1".to_string(),
                        business_identity: Some("business-1".to_string()),
                        message: RuntimeWebSocketMessageFrameHeader {
                            tag: RuntimeWebSocketMessageTagFrameHeader::Binary,
                            encoding: RuntimeWebSocketMessageEncodingFrameHeader::Raw,
                        },
                        context_codec: Some(RuntimeWebSocketContextCodecFrameHeader {
                            operation_abi_id: "context-op".to_string(),
                            context_type_identity: "context-type".to_string(),
                        }),
                        payload_segments: vec![RuntimeWebSocketPayloadSegmentFrameHeader {
                            kind: RuntimeWebSocketPayloadSegmentKindFrameHeader::Message,
                            offset: 0,
                            length: payload.len(),
                        }],
                    }),
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

        let websocket_adapter = request
            .websocket_adapter
            .expect("WebSocket adapter should map");
        assert_eq!(websocket_adapter.kind, WebSocketAdapterKind::Receive);
        assert_eq!(
            websocket_adapter.context_expectation,
            Some(WebSocketContextExpectation::Typed {
                connect_operation_abi_id: "connect-op".to_string(),
                context_type_identity: "context-type".to_string(),
            })
        );
        let receive_request = websocket_adapter
            .receive_request
            .expect("receive request should map");
        assert_eq!(receive_request.message.tag, WebSocketMessageTag::Binary);
        assert_eq!(
            receive_request.message.encoding,
            WebSocketMessageEncoding::Raw
        );
        assert_eq!(
            receive_request.payload_segments[0].kind,
            WebSocketPayloadSegmentKind::Message
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

        assert!(error.contains("request.start schemaVersion must be skiff-runtime-frame-v1"));
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
            business_identity: None,
            websocket_entry_id: None,
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
            websocket_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
        }
    }
}
