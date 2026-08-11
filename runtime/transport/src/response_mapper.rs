use skiff_runtime_request_contract::{
    FixedServiceResponseFailure, HttpNameValue, HttpResponseMetadata, OrdinaryResponseErrorSource,
    ResponseEnd, ResponseError, ResponseEvent, ResponseStreamEvent,
};

use crate::protocol::{
    encode_bytecode_websocket_jsonrpc_response_end_frame,
    BytecodeWebSocketConnectResponseEndFrameHeader,
    BytecodeWebSocketConnectResponseFrameHeader,
    BytecodeWebSocketJsonRpcResponseEndFrameHeader,
    BytecodeWebSocketJsonRpcResponseFrameHeader,
};
use crate::{
    error::TransportResult,
    protocol::{
        encode_binary_frame, validate_response_error_frame, ResponseChunkFrameHeader,
        ResponseEndFrameHeader, ResponseEndFrameMetadata, ResponseErrorFrameHeader,
        ResponseStartFrameHeader, RuntimeErrorFramePayload, RuntimeHttpNameValueFrameHeader,
        RuntimeHttpResponseFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    },
};

pub fn bytecode_websocket_connect_response_into_frame(
    request_id: String,
    response: BytecodeWebSocketConnectResponseFrameHeader,
) -> TransportResult<Vec<u8>> {
    encode_binary_frame(
        &BytecodeWebSocketConnectResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "response.end".to_string(),
            request_id,
            websocket_connect: response,
        },
        &[],
    )
}

pub fn bytecode_websocket_jsonrpc_response_into_frame(
    request_id: String,
    response: BytecodeWebSocketJsonRpcResponseFrameHeader,
    payload: Vec<u8>,
) -> TransportResult<Vec<u8>> {
    encode_bytecode_websocket_jsonrpc_response_end_frame(
        &BytecodeWebSocketJsonRpcResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "response.end".to_string(),
            request_id,
            websocket_json_rpc: response,
        },
        &payload,
    )
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrdinaryResponseEvent {
    End(ResponseEnd),
    FixedServiceFailure(FixedServiceResponseFailure),
    Error(ResponseError),
}

impl OrdinaryResponseEvent {
    pub fn try_from_non_error(event: ResponseEvent) -> TransportResult<Self> {
        match event {
            ResponseEvent::End(end) => Ok(Self::End(end)),
            ResponseEvent::FixedServiceFailure(failure) => Ok(Self::FixedServiceFailure(failure)),
            ResponseEvent::Error(_) => Err(crate::TransportError::decode(
                "response.error requires an ordinary error source",
            )),
        }
    }

    pub fn try_error(source: &(impl OrdinaryResponseErrorSource + ?Sized)) -> Option<Self> {
        source.ordinary_response_error().map(Self::Error)
    }

    pub fn response_error(&self) -> Option<&ResponseError> {
        match self {
            Self::Error(error) => Some(error),
            Self::End(_) | Self::FixedServiceFailure(_) => None,
        }
    }
}

pub fn response_event_into_frame(
    request_id: String,
    event: OrdinaryResponseEvent,
) -> TransportResult<Vec<u8>> {
    match event {
        OrdinaryResponseEvent::End(end) => {
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
        OrdinaryResponseEvent::FixedServiceFailure(failure) => {
            let payload = failure.into_error().into_encoded_bytes();
            let header = ResponseErrorFrameHeader::fixed_service(request_id);
            validate_response_error_frame(&header, payload.clone())?;
            encode_response_frame(&header, &payload)
        }
        OrdinaryResponseEvent::Error(error) => {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseEndPhase {
    Payload,
    Http,
}

/// Validates the phase facts that cannot be represented by the shared wire header alone.
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
            header.payload_present != payload.is_empty()
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
    }
}

#[cfg(test)]
mod tests;
