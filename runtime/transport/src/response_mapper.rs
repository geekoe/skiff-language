use skiff_runtime_request_contract::{
    HttpNameValue, HttpResponseMetadata, OutboundResponse, ResponseError, ResponseEvent,
    ResponseStreamEvent, WebSocketConnectResponse, WebSocketContextCodec,
};

use crate::{
    error::TransportResult,
    protocol::{
        encode_binary_frame, ResponseChunkFrameHeader, ResponseEndFrameHeader,
        ResponseErrorFrameHeader, ResponseStartFrameHeader, RuntimeErrorFramePayload,
        RuntimeHttpNameValueFrameHeader, RuntimeHttpResponseFrameHeader,
        RuntimeWebSocketContextCodecFrameHeader, RuntimeWebSocketResponseFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
};

pub fn response_event_into_frame(
    request_id: String,
    event: ResponseEvent,
) -> TransportResult<Vec<u8>> {
    match event {
        ResponseEvent::End {
            payload,
            http_response,
            websocket_connect,
        } => {
            let header = ResponseEndFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.end".to_string(),
                request_id,
                payload_present: !payload.is_empty(),
                http_response: http_response.map(protocol_http_response_metadata),
                websocket_connect: websocket_connect.map(protocol_websocket_connect_response),
            };
            encode_response_frame(&header, &payload)
        }
        ResponseEvent::Error(error) => {
            let header = ResponseErrorFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.error".to_string(),
                request_id,
                error: RuntimeErrorFramePayload {
                    code: error.code,
                    message: error.message,
                    status: error.status,
                    details: error.details,
                },
            };
            encode_response_frame(&header, &[])
        }
    }
}

pub fn response_stream_event_into_frame(
    request_id: &str,
    event: ResponseStreamEvent,
) -> TransportResult<Vec<u8>> {
    match event {
        ResponseStreamEvent::Start { http_response } => {
            let header = ResponseStartFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.start".to_string(),
                request_id: request_id.to_string(),
                http_response: protocol_http_response_metadata(http_response),
            };
            encode_response_frame(&header, &[])
        }
        ResponseStreamEvent::Chunk { seq, payload } => {
            let header = ResponseChunkFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.chunk".to_string(),
                request_id: request_id.to_string(),
                seq,
            };
            encode_response_frame(&header, &payload)
        }
        ResponseStreamEvent::End => {
            let header = ResponseEndFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.end".to_string(),
                request_id: request_id.to_string(),
                payload_present: false,
                http_response: None,
                websocket_connect: None,
            };
            encode_response_frame(&header, &[])
        }
    }
}

pub fn response_end_to_outbound(
    header: &ResponseEndFrameHeader,
    payload: Vec<u8>,
) -> OutboundResponse {
    let _payload_present = header.payload_present;
    OutboundResponse::End { payload }
}

pub fn response_start_to_outbound(header: &ResponseStartFrameHeader) -> OutboundResponse {
    OutboundResponse::Start {
        http_response: request_http_response_metadata(header.http_response.clone()),
    }
}

pub fn response_chunk_to_outbound(
    header: &ResponseChunkFrameHeader,
    payload: Vec<u8>,
) -> OutboundResponse {
    OutboundResponse::Chunk {
        seq: header.seq,
        payload,
    }
}

pub fn response_error_to_outbound(header: &ResponseErrorFrameHeader) -> OutboundResponse {
    OutboundResponse::Error(request_response_error(header.error.clone()))
}

fn encode_response_frame<THeader: serde::Serialize>(
    header: &THeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_binary_frame(header, payload)
}

fn protocol_http_response_metadata(
    response: HttpResponseMetadata,
) -> RuntimeHttpResponseFrameHeader {
    RuntimeHttpResponseFrameHeader {
        status: response.status,
        headers: response
            .headers
            .into_iter()
            .map(protocol_http_name_value)
            .collect(),
    }
}

fn protocol_http_name_value(item: HttpNameValue) -> RuntimeHttpNameValueFrameHeader {
    RuntimeHttpNameValueFrameHeader {
        name: item.name,
        value: item.value,
    }
}

fn request_http_response_metadata(
    response: RuntimeHttpResponseFrameHeader,
) -> HttpResponseMetadata {
    HttpResponseMetadata {
        status: response.status,
        headers: response
            .headers
            .into_iter()
            .map(request_http_name_value)
            .collect(),
    }
}

fn request_http_name_value(item: RuntimeHttpNameValueFrameHeader) -> HttpNameValue {
    HttpNameValue {
        name: item.name,
        value: item.value,
    }
}

fn request_response_error(error: RuntimeErrorFramePayload) -> ResponseError {
    ResponseError {
        code: error.code,
        message: error.message,
        status: error.status,
        details: error.details,
    }
}

fn protocol_websocket_connect_response(
    response: WebSocketConnectResponse,
) -> RuntimeWebSocketResponseFrameHeader {
    RuntimeWebSocketResponseFrameHeader {
        result: response.result,
        business_identity: response.business_identity,
        connection_policy: response.connection_policy,
        context_codec: response.context_codec.map(protocol_websocket_context_codec),
        context_payload_present: response.context_payload_present,
        code: response.code,
        reason: response.reason,
    }
}

fn protocol_websocket_context_codec(
    codec: WebSocketContextCodec,
) -> RuntimeWebSocketContextCodecFrameHeader {
    RuntimeWebSocketContextCodecFrameHeader {
        operation_abi_id: codec.operation_abi_id,
        context_type_identity: codec.context_type_identity,
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use serde_json::json;

    use super::{
        response_chunk_to_outbound, response_end_to_outbound, response_error_to_outbound,
        response_event_into_frame, response_start_to_outbound, response_stream_event_into_frame,
    };
    use crate::protocol::{
        decode_typed_binary_frame, ResponseChunkFrameHeader, ResponseEndFrameHeader,
        ResponseErrorFrameHeader, ResponseStartFrameHeader, RuntimeErrorFramePayload,
        RuntimeHttpNameValueFrameHeader, RuntimeHttpResponseFrameHeader,
        RuntimeWebSocketContextCodecFrameHeader, RuntimeWebSocketResponseFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    };
    use skiff_runtime_request_contract::{
        HttpNameValue, HttpResponseMetadata, OutboundResponse, ResponseError, ResponseEvent,
        ResponseStreamEvent, WebSocketConnectResponse, WebSocketConnectionPolicyControl,
        WebSocketConnectionPolicyOverflowControl, WebSocketContextCodec,
    };

    #[test]
    fn response_event_end_maps_to_response_end_frame_with_opaque_payload() {
        let payload = b"opaque response bytes".to_vec();
        let frame = response_event_into_frame(
            "request-1".to_string(),
            ResponseEvent::End {
                payload: payload.clone(),
                http_response: Some(HttpResponseMetadata::new(
                    202,
                    vec![HttpNameValue {
                        name: "content-type".to_string(),
                        value: "application/octet-stream".to_string(),
                    }],
                )),
                websocket_connect: Some(WebSocketConnectResponse {
                    result: "accepted".to_string(),
                    business_identity: Some("business-1".to_string()),
                    connection_policy: Some(WebSocketConnectionPolicyControl {
                        max_connections: NonZeroU32::new(1).expect("non-zero fixture"),
                        overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
                        close_code: None,
                        close_reason: None,
                    }),
                    context_codec: Some(WebSocketContextCodec {
                        operation_abi_id: "op-abi-1".to_string(),
                        context_type_identity: "context-type-1".to_string(),
                    }),
                    context_payload_present: true,
                    code: None,
                    reason: None,
                }),
            },
        )
        .expect("response.end frame should encode");

        let (header, decoded_payload): (ResponseEndFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("response.end frame should decode");

        assert_eq!(decoded_payload, payload);
        assert_eq!(
            header,
            ResponseEndFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.end".to_string(),
                request_id: "request-1".to_string(),
                payload_present: true,
                http_response: Some(RuntimeHttpResponseFrameHeader {
                    status: 202,
                    headers: vec![RuntimeHttpNameValueFrameHeader {
                        name: "content-type".to_string(),
                        value: "application/octet-stream".to_string(),
                    }],
                }),
                websocket_connect: Some(RuntimeWebSocketResponseFrameHeader {
                    result: "accepted".to_string(),
                    business_identity: Some("business-1".to_string()),
                    connection_policy: Some(WebSocketConnectionPolicyControl {
                        max_connections: NonZeroU32::new(1).expect("non-zero fixture"),
                        overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
                        close_code: None,
                        close_reason: None,
                    }),
                    context_codec: Some(RuntimeWebSocketContextCodecFrameHeader {
                        operation_abi_id: "op-abi-1".to_string(),
                        context_type_identity: "context-type-1".to_string(),
                    }),
                    context_payload_present: true,
                    code: None,
                    reason: None,
                }),
            }
        );
    }

    #[test]
    fn response_event_error_maps_to_response_error_frame() {
        let frame = response_event_into_frame(
            "request-2".to_string(),
            ResponseEvent::Error(ResponseError {
                code: "std.http.HttpError".to_string(),
                message: "upstream failed".to_string(),
                status: None,
                details: Some(json!({ "status": 503 })),
            }),
        )
        .expect("response.error frame should encode");

        let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("response.error frame should decode");

        assert!(payload.is_empty());
        assert_eq!(header.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(header.envelope_type, "response.error");
        assert_eq!(header.request_id, "request-2");
        assert_eq!(header.error.code, "std.http.HttpError");
        assert_eq!(header.error.message, "upstream failed");
        assert_eq!(header.error.details, Some(json!({ "status": 503 })));
    }

    #[test]
    fn response_stream_events_map_to_start_chunk_and_end_frames() {
        let start = response_stream_event_into_frame(
            "request-3",
            ResponseStreamEvent::Start {
                http_response: HttpResponseMetadata::new(
                    200,
                    vec![HttpNameValue {
                        name: "x-stream".to_string(),
                        value: "yes".to_string(),
                    }],
                ),
            },
        )
        .expect("response.start frame should encode");
        let (start_header, start_payload): (ResponseStartFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&start).expect("response.start frame should decode");

        assert!(start_payload.is_empty());
        assert_eq!(start_header.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(start_header.envelope_type, "response.start");
        assert_eq!(start_header.request_id, "request-3");
        assert_eq!(start_header.http_response.status, 200);
        assert_eq!(
            start_header.http_response.headers,
            vec![RuntimeHttpNameValueFrameHeader {
                name: "x-stream".to_string(),
                value: "yes".to_string(),
            }]
        );

        let chunk = response_stream_event_into_frame(
            "request-3",
            ResponseStreamEvent::Chunk {
                seq: 7,
                payload: b"chunk bytes".to_vec(),
            },
        )
        .expect("response.chunk frame should encode");
        let (chunk_header, chunk_payload): (ResponseChunkFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&chunk).expect("response.chunk frame should decode");

        assert_eq!(chunk_payload.as_slice(), b"chunk bytes");
        assert_eq!(chunk_header.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(chunk_header.envelope_type, "response.chunk");
        assert_eq!(chunk_header.request_id, "request-3");
        assert_eq!(chunk_header.seq, 7);

        let end = response_stream_event_into_frame("request-3", ResponseStreamEvent::End)
            .expect("response.end frame should encode");
        let (end_header, end_payload): (ResponseEndFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&end).expect("response.end frame should decode");

        assert!(end_payload.is_empty());
        assert_eq!(end_header.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(end_header.envelope_type, "response.end");
        assert_eq!(end_header.request_id, "request-3");
        assert!(!end_header.payload_present);
        assert!(end_header.http_response.is_none());
        assert!(end_header.websocket_connect.is_none());
    }

    #[test]
    fn response_frame_headers_map_to_outbound_router_response_facts() {
        let http_response = RuntimeHttpResponseFrameHeader {
            status: 206,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "content-range".to_string(),
                value: "bytes 0-4/10".to_string(),
            }],
        };
        let start = response_start_to_outbound(&ResponseStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.start".to_string(),
            request_id: "request-4".to_string(),
            http_response: http_response.clone(),
        });
        assert_eq!(start.kind(), "response.start");
        let expected_http_response = HttpResponseMetadata {
            status: http_response.status,
            headers: vec![HttpNameValue {
                name: "content-range".to_string(),
                value: "bytes 0-4/10".to_string(),
            }],
        };
        assert!(matches!(
            start,
            OutboundResponse::Start {
                http_response: actual
            } if actual == expected_http_response
        ));

        let chunk = response_chunk_to_outbound(
            &ResponseChunkFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.chunk".to_string(),
                request_id: "request-4".to_string(),
                seq: 9,
            },
            b"chunk bytes".to_vec(),
        );
        assert_eq!(chunk.kind(), "response.chunk");
        assert!(matches!(
            chunk,
            OutboundResponse::Chunk { seq: 9, payload }
                if payload.as_slice() == b"chunk bytes"
        ));

        let end = response_end_to_outbound(
            &ResponseEndFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.end".to_string(),
                request_id: "request-4".to_string(),
                payload_present: true,
                http_response: None,
                websocket_connect: None,
            },
            b"final bytes".to_vec(),
        );
        assert_eq!(end.kind(), "response.end");
        assert!(matches!(
            end,
            OutboundResponse::End { payload } if payload.as_slice() == b"final bytes"
        ));

        let error = RuntimeErrorFramePayload {
            code: "RemoteError".to_string(),
            message: "callee failed".to_string(),
            status: Some(502),
            details: Some(json!({ "upstream": "account" })),
        };
        let response_error = response_error_to_outbound(&ResponseErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.error".to_string(),
            request_id: "request-4".to_string(),
            error: error.clone(),
        });
        assert_eq!(response_error.kind(), "response.error");
        let expected_error = ResponseError {
            code: error.code,
            message: error.message,
            status: error.status,
            details: error.details,
        };
        assert!(matches!(
            response_error,
            OutboundResponse::Error(actual) if actual == expected_error
        ));
    }
}
