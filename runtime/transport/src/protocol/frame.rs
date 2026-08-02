use crate::TransportError;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

pub const BINARY_FRAME_MAGIC: [u8; 4] = *b"SKBF";
pub const BINARY_FRAME_VERSION: u8 = 1;
pub const BINARY_FRAME_HEADER_ENCODING_JSON: u8 = 1;
pub const RUNTIME_FRAME_SCHEMA_VERSION: &str = "skiff-runtime-frame-v3";
pub const RESPONSE_ERROR_FRAME_SCHEMA_VERSION: &str = "skiff-runtime-frame-v3";

const BINARY_FRAME_FIXED_HEADER_BYTES: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFrame {
    pub header: Value,
    pub payload_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFrameParts {
    pub header_bytes: Vec<u8>,
    pub payload_bytes: Vec<u8>,
}

pub use crate::BinaryFrameError;

pub fn encode_binary_frame<THeader: Serialize>(
    header: &THeader,
    payload_bytes: &[u8],
) -> std::result::Result<Vec<u8>, BinaryFrameError> {
    let header_bytes = serde_json::to_vec(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid skiff binary frame: header serialization failed: {error}"
        ))
    })?;
    if header_bytes.is_empty() {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header must not be empty",
        ));
    }
    if header_bytes.len() > u32::MAX as usize {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header length exceeds u32",
        ));
    }
    if payload_bytes.len() > u32::MAX as usize {
        return Err(TransportError::decode(
            "invalid skiff binary frame: payload length exceeds u32",
        ));
    }

    let mut frame = Vec::with_capacity(
        BINARY_FRAME_FIXED_HEADER_BYTES + header_bytes.len() + payload_bytes.len(),
    );
    frame.extend_from_slice(&BINARY_FRAME_MAGIC);
    frame.push(BINARY_FRAME_VERSION);
    frame.push(BINARY_FRAME_HEADER_ENCODING_JSON);
    frame.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(payload_bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(&header_bytes);
    frame.extend_from_slice(payload_bytes);
    Ok(frame)
}

pub fn decode_binary_frame(frame: &[u8]) -> std::result::Result<BinaryFrame, BinaryFrameError> {
    let parts = decode_binary_frame_parts(frame)?;
    let header: Value = serde_json::from_slice(&parts.header_bytes).map_err(|error| {
        TransportError::decode(format!(
            "invalid skiff binary frame: header is not valid JSON: {error}"
        ))
    })?;
    if !header.is_object() {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header must be an object",
        ));
    }

    Ok(BinaryFrame {
        header,
        payload_bytes: parts.payload_bytes,
    })
}

pub fn decode_binary_frame_parts(
    frame: &[u8],
) -> std::result::Result<BinaryFrameParts, BinaryFrameError> {
    if frame.len() < BINARY_FRAME_FIXED_HEADER_BYTES {
        return Err(TransportError::decode(
            "invalid skiff binary frame: frame is too short",
        ));
    }
    if frame[0..4] != BINARY_FRAME_MAGIC {
        return Err(TransportError::decode(
            "invalid skiff binary frame: expected skiff binary frame magic",
        ));
    }
    let version = frame[4];
    if version != BINARY_FRAME_VERSION {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: unsupported frame version {version}"
        )));
    }
    let header_encoding = frame[5];
    if header_encoding != BINARY_FRAME_HEADER_ENCODING_JSON {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: unsupported header encoding {header_encoding}"
        )));
    }

    let header_length = u32::from_be_bytes([frame[6], frame[7], frame[8], frame[9]]) as usize;
    let payload_length = u32::from_be_bytes([frame[10], frame[11], frame[12], frame[13]]) as usize;
    if header_length == 0 {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header must not be empty",
        ));
    }
    let expected_length = BINARY_FRAME_FIXED_HEADER_BYTES
        .checked_add(header_length)
        .and_then(|length| length.checked_add(payload_length))
        .ok_or_else(|| {
            TransportError::decode("invalid skiff binary frame: frame length overflow")
        })?;
    if frame.len() != expected_length {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: frame length {} does not match header length {} plus payload length {}",
            frame.len(),
            header_length,
            payload_length
        )));
    }

    let header_start = BINARY_FRAME_FIXED_HEADER_BYTES;
    let payload_start = header_start + header_length;
    Ok(BinaryFrameParts {
        header_bytes: frame[header_start..payload_start].to_vec(),
        payload_bytes: frame[payload_start..].to_vec(),
    })
}

pub fn decode_typed_binary_frame<THeader: DeserializeOwned>(
    frame: &[u8],
) -> std::result::Result<(THeader, Vec<u8>), BinaryFrameError> {
    let frame = decode_binary_frame(frame)?;
    let header = serde_json::from_value(frame.header).map_err(|error| {
        TransportError::decode(format!(
            "invalid skiff binary frame: header failed typed decode: {error}"
        ))
    })?;
    Ok((header, frame.payload_bytes))
}
