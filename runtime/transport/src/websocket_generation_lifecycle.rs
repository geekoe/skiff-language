use serde::{Deserialize, Serialize};

use crate::{
    protocol::{decode_binary_frame_parts, encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    BinaryFrameError, TransportError,
};

pub const WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE: &str = "websocket.generation.lifecycle";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSocketGenerationLifecycleDirection {
    RouterToRuntime,
    RuntimeToRouter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebSocketGenerationLifecycleSender {
    Router,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebSocketGenerationLifecycleOperation {
    Acquire,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebSocketGenerationLifecycleRejectionCode {
    GenerationUnavailable,
    NotAcquired,
    RequestConflict,
    SenderMismatch,
    TupleMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSocketGenerationLifecycleTuple {
    pub router_session_id: String,
    pub service_id: String,
    /// Deployment build id the connection is pinned to (M4: deployment
    /// anchoring replaces the assembly generation keying).
    pub build_id: String,
    pub websocket_entry_id: String,
    pub connection_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WebSocketGenerationLifecycleControl {
    Acquire {
        schema_version: String,
        #[serde(rename = "type")]
        frame_type: String,
        request_id: String,
        sender: WebSocketGenerationLifecycleSender,
        tuple: WebSocketGenerationLifecycleTuple,
    },
    Release {
        schema_version: String,
        #[serde(rename = "type")]
        frame_type: String,
        request_id: String,
        sender: WebSocketGenerationLifecycleSender,
        tuple: WebSocketGenerationLifecycleTuple,
    },
    Ack {
        schema_version: String,
        #[serde(rename = "type")]
        frame_type: String,
        operation: WebSocketGenerationLifecycleOperation,
        request_id: String,
        sender: WebSocketGenerationLifecycleSender,
        tuple: WebSocketGenerationLifecycleTuple,
    },
    Reject {
        schema_version: String,
        #[serde(rename = "type")]
        frame_type: String,
        operation: WebSocketGenerationLifecycleOperation,
        request_id: String,
        sender: WebSocketGenerationLifecycleSender,
        tuple: WebSocketGenerationLifecycleTuple,
        code: WebSocketGenerationLifecycleRejectionCode,
        reason: String,
    },
}

pub fn encode_websocket_generation_lifecycle_frame(
    direction: WebSocketGenerationLifecycleDirection,
    control: &WebSocketGenerationLifecycleControl,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_control(direction, control)?;
    encode_binary_frame(control, &[])
}

pub fn decode_websocket_generation_lifecycle_frame(
    direction: WebSocketGenerationLifecycleDirection,
    frame: &[u8],
) -> Result<WebSocketGenerationLifecycleControl, BinaryFrameError> {
    let parts = decode_binary_frame_parts(frame)?;
    if !parts.payload_bytes.is_empty() {
        return Err(TransportError::decode(
            "websocket generation lifecycle frame payload must be empty",
        ));
    }
    let control: WebSocketGenerationLifecycleControl = serde_json::from_slice(&parts.header_bytes)
        .map_err(|error| {
            TransportError::decode(format!(
                "websocket generation lifecycle header failed strict decode: {error}"
            ))
        })?;
    validate_control(direction, &control)?;
    Ok(control)
}

pub fn assert_websocket_generation_lifecycle_response_matches(
    request: &WebSocketGenerationLifecycleControl,
    response: &WebSocketGenerationLifecycleControl,
) -> Result<(), BinaryFrameError> {
    let (request_operation, request_id, request_tuple) = request
        .request_parts()
        .ok_or_else(|| TransportError::decode("websocket generation lifecycle request expected"))?;
    let (response_operation, response_request_id, response_tuple) =
        response.response_parts().ok_or_else(|| {
            TransportError::decode("websocket generation lifecycle response expected")
        })?;
    if request_id != response_request_id {
        return Err(TransportError::decode(
            "websocket generation lifecycle response requestId mismatch",
        ));
    }
    if request_operation != response_operation {
        return Err(TransportError::decode(
            "websocket generation lifecycle response operation mismatch",
        ));
    }
    if request_tuple != response_tuple {
        return Err(TransportError::decode(
            "websocket generation lifecycle response tuple mismatch",
        ));
    }
    Ok(())
}

impl WebSocketGenerationLifecycleControl {
    fn common_parts(
        &self,
    ) -> (
        &str,
        &str,
        &str,
        WebSocketGenerationLifecycleSender,
        &WebSocketGenerationLifecycleTuple,
    ) {
        match self {
            Self::Acquire {
                schema_version,
                frame_type,
                request_id,
                sender,
                tuple,
            }
            | Self::Release {
                schema_version,
                frame_type,
                request_id,
                sender,
                tuple,
            }
            | Self::Ack {
                schema_version,
                frame_type,
                request_id,
                sender,
                tuple,
                ..
            }
            | Self::Reject {
                schema_version,
                frame_type,
                request_id,
                sender,
                tuple,
                ..
            } => (schema_version, frame_type, request_id, *sender, tuple),
        }
    }

    fn request_parts(
        &self,
    ) -> Option<(
        WebSocketGenerationLifecycleOperation,
        &str,
        &WebSocketGenerationLifecycleTuple,
    )> {
        match self {
            Self::Acquire {
                request_id, tuple, ..
            } => Some((
                WebSocketGenerationLifecycleOperation::Acquire,
                request_id,
                tuple,
            )),
            Self::Release {
                request_id, tuple, ..
            } => Some((
                WebSocketGenerationLifecycleOperation::Release,
                request_id,
                tuple,
            )),
            Self::Ack { .. } | Self::Reject { .. } => None,
        }
    }

    fn response_parts(
        &self,
    ) -> Option<(
        WebSocketGenerationLifecycleOperation,
        &str,
        &WebSocketGenerationLifecycleTuple,
    )> {
        match self {
            Self::Ack {
                operation,
                request_id,
                tuple,
                ..
            }
            | Self::Reject {
                operation,
                request_id,
                tuple,
                ..
            } => Some((*operation, request_id, tuple)),
            Self::Acquire { .. } | Self::Release { .. } => None,
        }
    }
}

fn validate_control(
    direction: WebSocketGenerationLifecycleDirection,
    control: &WebSocketGenerationLifecycleControl,
) -> Result<(), BinaryFrameError> {
    let (schema_version, frame_type, request_id, sender, tuple) = control.common_parts();
    if schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(TransportError::decode(format!(
            "websocket generation lifecycle schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        )));
    }
    if frame_type != WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE {
        return Err(TransportError::decode(format!(
            "websocket generation lifecycle type must be {WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE}"
        )));
    }
    validate_request_id(request_id)?;
    validate_tuple(tuple)?;

    let (expected_direction, expected_sender) = match control {
        WebSocketGenerationLifecycleControl::Acquire { .. } => (
            WebSocketGenerationLifecycleDirection::RuntimeToRouter,
            WebSocketGenerationLifecycleSender::Runtime,
        ),
        WebSocketGenerationLifecycleControl::Release { .. } => (
            WebSocketGenerationLifecycleDirection::RouterToRuntime,
            WebSocketGenerationLifecycleSender::Router,
        ),
        WebSocketGenerationLifecycleControl::Ack { operation, .. }
        | WebSocketGenerationLifecycleControl::Reject { operation, .. } => match operation {
            WebSocketGenerationLifecycleOperation::Acquire => (
                WebSocketGenerationLifecycleDirection::RouterToRuntime,
                WebSocketGenerationLifecycleSender::Router,
            ),
            WebSocketGenerationLifecycleOperation::Release => (
                WebSocketGenerationLifecycleDirection::RuntimeToRouter,
                WebSocketGenerationLifecycleSender::Runtime,
            ),
        },
    };
    if direction != expected_direction {
        return Err(TransportError::decode(
            "websocket generation lifecycle control has invalid direction",
        ));
    }
    if sender != expected_sender {
        return Err(TransportError::decode(
            "websocket generation lifecycle control has invalid sender",
        ));
    }
    if let WebSocketGenerationLifecycleControl::Reject { reason, .. } = control {
        if reason.is_empty() {
            return Err(TransportError::decode(
                "websocket generation lifecycle rejection reason must be non-empty",
            ));
        }
    }
    Ok(())
}

fn validate_request_id(value: &str) -> Result<(), BinaryFrameError> {
    validate_opaque_identity(
        value,
        "skiff-websocket-lifecycle-request-v1:opaque:",
        "requestId",
    )
}

fn validate_tuple(tuple: &WebSocketGenerationLifecycleTuple) -> Result<(), BinaryFrameError> {
    validate_opaque_identity(
        &tuple.router_session_id,
        "skiff-router-session-v1:opaque:",
        "tuple.routerSessionId",
    )?;
    validate_service_id(&tuple.service_id)?;
    validate_build_id(&tuple.build_id)?;
    validate_sha256_identity(
        &tuple.websocket_entry_id,
        "skiff-websocket-entry-v1:sha256:",
        "tuple.websocketEntryId",
    )?;
    if tuple.connection_id.is_empty()
        || tuple.connection_id.len() > 255
        || !tuple.connection_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'~' | b'-')
        })
    {
        return Err(TransportError::decode(
            "websocket generation lifecycle tuple.connectionId is invalid",
        ));
    }
    Ok(())
}

fn validate_build_id(value: &str) -> Result<(), BinaryFrameError> {
    if value.is_empty()
        || value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'~' | b'-'))
    {
        return Err(TransportError::decode(
            "websocket generation lifecycle tuple.buildId is invalid",
        ));
    }
    Ok(())
}

fn validate_opaque_identity(
    value: &str,
    prefix: &str,
    label: &str,
) -> Result<(), BinaryFrameError> {
    let suffix = value.strip_prefix(prefix).ok_or_else(|| {
        TransportError::decode(format!(
            "websocket generation lifecycle {label} has invalid prefix"
        ))
    })?;
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(TransportError::decode(format!(
            "websocket generation lifecycle {label} is invalid"
        )));
    }
    Ok(())
}

fn validate_sha256_identity(
    value: &str,
    prefix: &str,
    label: &str,
) -> Result<(), BinaryFrameError> {
    let digest = value.strip_prefix(prefix).ok_or_else(|| {
        TransportError::decode(format!(
            "websocket generation lifecycle {label} has invalid prefix"
        ))
    })?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(TransportError::decode(format!(
            "websocket generation lifecycle {label} must use 64 lowercase hex"
        )));
    }
    Ok(())
}

fn validate_service_id(value: &str) -> Result<(), BinaryFrameError> {
    if value.is_empty()
        || value.len() > 63
        || value == "std"
        || value != value.trim()
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value.bytes().any(|byte| byte.is_ascii_control())
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'/' | b'_' | b'-')
        })
    {
        return Err(TransportError::decode(
            "websocket generation lifecycle tuple.serviceId is invalid",
        ));
    }
    let Some((authority, local)) = value.split_once('/') else {
        return Err(TransportError::decode(
            "websocket generation lifecycle tuple.serviceId is invalid",
        ));
    };
    let authority_labels = authority.split('.').collect::<Vec<_>>();
    if authority_labels.len() < 2
        || authority_labels
            .iter()
            .any(|label| !is_valid_authority_label(label))
        || local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        return Err(TransportError::decode(
            "websocket generation lifecycle tuple.serviceId is invalid",
        ));
    }
    Ok(())
}

fn is_valid_authority_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0] != b'-'
        && bytes.last() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

#[cfg(test)]
#[path = "websocket_generation_lifecycle/tests.rs"]
mod tests;
