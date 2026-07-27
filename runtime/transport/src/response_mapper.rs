use skiff_runtime_request_contract::{
    HttpNameValue, HttpResponseMetadata, OutboundResponse, ResponseEnd, ResponseError,
    ResponseEvent, ResponseStreamEvent, WebSocketConnectAccept, WebSocketConnectContext,
    WebSocketConnectReject, WebSocketContextCodec, WebSocketResponse,
};

use crate::{
    error::TransportResult,
    protocol::{
        encode_binary_frame, validate_response_error_frame, ResponseChunkFrameHeader,
        ResponseEndFrameHeader, ResponseEndFrameMetadata, ResponseErrorFrameHeader,
        ResponseStartFrameHeader, RuntimeErrorFramePayload, RuntimeHttpNameValueFrameHeader,
        RuntimeHttpResponseFrameHeader, RuntimeWebSocketConnectAcceptFrameHeader,
        RuntimeWebSocketConnectContextFrameHeader, RuntimeWebSocketConnectRejectFrameHeader,
        RuntimeWebSocketContextCodecFrameHeader, RuntimeWebSocketResponseFrameHeader,
        ValidatedResponseErrorFrame, RUNTIME_FRAME_SCHEMA_VERSION,
    },
    runtime_assembly_request::{
        RuntimeAssemblyWebSocketConnectResponseEndFrameHeader,
        RuntimeAssemblyWebSocketConnectResponseFrameHeader,
    },
};

pub fn runtime_assembly_websocket_connect_response_into_frame(
    request_id: String,
    response: RuntimeAssemblyWebSocketConnectResponseFrameHeader,
) -> TransportResult<Vec<u8>> {
    encode_binary_frame(
        &RuntimeAssemblyWebSocketConnectResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "response.end".to_string(),
            request_id,
            websocket_connect: response,
        },
        &[],
    )
}

pub fn response_event_into_frame(
    request_id: String,
    event: ResponseEvent,
) -> TransportResult<Vec<u8>> {
    match event {
        ResponseEvent::End(end) => {
            let (payload, payload_present, metadata, phase) = response_end_frame_parts(end);
            let header = ResponseEndFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "response.end".to_string(),
                request_id,
                payload_present,
                metadata,
            };
            validate_response_end_frame(&header, &payload, phase)?;
            encode_response_frame(&header, &payload)
        }
        ResponseEvent::FixedServiceFailure(failure) => {
            let payload = failure.into_error().into_encoded_bytes();
            let header = ResponseErrorFrameHeader::fixed_service(request_id);
            validate_response_error_frame(&header, payload.clone())?;
            encode_response_frame(&header, &payload)
        }
        ResponseEvent::Error(error) => {
            let header = ResponseErrorFrameHeader::control(
                request_id,
                RuntimeErrorFramePayload {
                    code: error.code,
                    message: error.message,
                    status: error.status,
                    details: error.details,
                },
            );
            validate_response_error_frame(&header, Vec::new())?;
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
                metadata: ResponseEndFrameMetadata::None,
            };
            encode_response_frame(&header, &[])
        }
    }
}

pub fn response_end_to_outbound(
    header: &ResponseEndFrameHeader,
    payload: Vec<u8>,
) -> OutboundResponse {
    if let Err(error) = validate_response_end_frame(header, &payload, ResponseEndPhase::Payload) {
        return invalid_response_end(&error.to_string());
    }
    OutboundResponse::End { payload }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseEndPhase {
    Payload,
    Http,
    WebSocketConnect,
    WebSocketReceive,
}

/// Validates the phase facts that cannot be represented by the shared wire header alone.
/// In particular, a typed WebSocket Context is nominally present even when its payload encodes to
/// zero bytes, while receive never carries a response payload or connect metadata.
pub fn validate_response_end_frame(
    header: &ResponseEndFrameHeader,
    payload: &[u8],
    phase: ResponseEndPhase,
) -> TransportResult<()> {
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION
        || header.envelope_type != "response.end"
    {
        return Err(crate::TransportError::decode(
            "response.end schemaVersion/type is invalid",
        ));
    }
    let valid = match (phase, &header.metadata) {
        (ResponseEndPhase::Payload, ResponseEndFrameMetadata::None)
        | (ResponseEndPhase::Http, ResponseEndFrameMetadata::Http(_)) => {
            header.payload_present == !payload.is_empty()
        }
        (
            ResponseEndPhase::WebSocketConnect,
            ResponseEndFrameMetadata::WebSocketConnect(
                RuntimeWebSocketResponseFrameHeader::ConnectAccept(accept),
            ),
        ) => match &accept.context {
            RuntimeWebSocketConnectContextFrameHeader::Null => {
                !header.payload_present && payload.is_empty()
            }
            RuntimeWebSocketConnectContextFrameHeader::Typed(_) => header.payload_present,
        },
        (
            ResponseEndPhase::WebSocketConnect,
            ResponseEndFrameMetadata::WebSocketConnect(
                RuntimeWebSocketResponseFrameHeader::ConnectReject(_),
            ),
        )
        | (ResponseEndPhase::WebSocketReceive, ResponseEndFrameMetadata::None) => {
            !header.payload_present && payload.is_empty()
        }
        _ => false,
    };
    if !valid {
        return Err(crate::TransportError::decode(
            "response.end metadata/payload does not match the admitted response phase",
        ));
    }
    Ok(())
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

pub fn response_error_to_outbound(
    header: &ResponseErrorFrameHeader,
    payload: Vec<u8>,
) -> OutboundResponse {
    match validate_response_error_frame(header, payload) {
        Ok(ValidatedResponseErrorFrame::FixedService(error)) => {
            OutboundResponse::fixed_service_failure(error)
        }
        Ok(ValidatedResponseErrorFrame::Control(error)) => {
            OutboundResponse::Error(request_response_error(error))
        }
        Err(error) => invalid_response_error(&error.to_string()),
    }
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

fn response_end_frame_parts(
    response: ResponseEnd,
) -> (Vec<u8>, bool, ResponseEndFrameMetadata, ResponseEndPhase) {
    match response {
        ResponseEnd::Payload(payload) => {
            let payload_present = !payload.is_empty();
            (
                payload,
                payload_present,
                ResponseEndFrameMetadata::None,
                ResponseEndPhase::Payload,
            )
        }
        ResponseEnd::Http { payload, metadata } => {
            let payload_present = !payload.is_empty();
            (
                payload,
                payload_present,
                ResponseEndFrameMetadata::Http(protocol_http_response_metadata(metadata)),
                ResponseEndPhase::Http,
            )
        }
        ResponseEnd::WebSocket(WebSocketResponse::ConnectAccept(response)) => {
            let (payload, payload_present, metadata) = protocol_websocket_connect_accept(response);
            (
                payload,
                payload_present,
                metadata,
                ResponseEndPhase::WebSocketConnect,
            )
        }
        ResponseEnd::WebSocket(WebSocketResponse::ConnectReject(response)) => (
            Vec::new(),
            false,
            ResponseEndFrameMetadata::WebSocketConnect(protocol_websocket_connect_reject(response)),
            ResponseEndPhase::WebSocketConnect,
        ),
        ResponseEnd::WebSocket(WebSocketResponse::Receive) => (
            Vec::new(),
            false,
            ResponseEndFrameMetadata::None,
            ResponseEndPhase::WebSocketReceive,
        ),
    }
}

fn protocol_websocket_connect_accept(
    response: WebSocketConnectAccept,
) -> (Vec<u8>, bool, ResponseEndFrameMetadata) {
    let (payload, payload_present, context) = match response.context {
        WebSocketConnectContext::Null => (
            Vec::new(),
            false,
            RuntimeWebSocketConnectContextFrameHeader::Null,
        ),
        WebSocketConnectContext::Typed { payload, codec } => (
            payload,
            true,
            RuntimeWebSocketConnectContextFrameHeader::Typed(protocol_websocket_context_codec(
                codec,
            )),
        ),
    };
    (
        payload,
        payload_present,
        ResponseEndFrameMetadata::WebSocketConnect(
            RuntimeWebSocketResponseFrameHeader::ConnectAccept(
                RuntimeWebSocketConnectAcceptFrameHeader {
                    business_identity: response.business_identity,
                    connection_policy: response.connection_policy,
                    context,
                },
            ),
        ),
    )
}

fn protocol_websocket_connect_reject(
    response: WebSocketConnectReject,
) -> RuntimeWebSocketResponseFrameHeader {
    RuntimeWebSocketResponseFrameHeader::ConnectReject(RuntimeWebSocketConnectRejectFrameHeader {
        code: response.code,
        reason: response.reason,
    })
}

fn invalid_response_end(message: &str) -> OutboundResponse {
    OutboundResponse::Error(ResponseError {
        code: "RuntimeProtocolViolation".to_string(),
        message: message.to_string(),
        status: None,
        details: None,
    })
}

fn invalid_response_error(message: &str) -> OutboundResponse {
    OutboundResponse::Error(ResponseError {
        code: "RuntimeProtocolViolation".to_string(),
        message: message.to_string(),
        status: None,
        details: None,
    })
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
mod tests;
