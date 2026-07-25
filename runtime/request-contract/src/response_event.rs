use crate::envelope::WebSocketContextCodec;

pub use skiff_runtime_capability_context::{
    FixedServiceResponseFailure, HttpResponseMetadata, ResponseError,
    WebSocketConnectionPolicyControl,
};

#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryResponse {
    Event(ResponseEvent),
    StreamSent,
}

impl BoundaryResponse {
    pub fn payload(payload: Vec<u8>) -> Self {
        Self::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))
    }

    pub fn http(payload: Vec<u8>, metadata: HttpResponseMetadata) -> Self {
        Self::Event(ResponseEvent::End(ResponseEnd::Http { payload, metadata }))
    }

    pub fn websocket(response: WebSocketResponse) -> Self {
        Self::Event(ResponseEvent::End(ResponseEnd::WebSocket(response)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEvent {
    End(ResponseEnd),
    FixedServiceFailure(FixedServiceResponseFailure),
    Error(ResponseError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEnd {
    Payload(Vec<u8>),
    Http {
        payload: Vec<u8>,
        metadata: HttpResponseMetadata,
    },
    WebSocket(WebSocketResponse),
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebSocketResponse {
    ConnectAccept(WebSocketConnectAccept),
    ConnectReject(WebSocketConnectReject),
    Receive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebSocketConnectAccept {
    pub business_identity: Option<String>,
    pub connection_policy: Option<WebSocketConnectionPolicyControl>,
    pub context: WebSocketConnectContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketConnectContext {
    Null,
    Typed {
        payload: Vec<u8>,
        codec: WebSocketContextCodec,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketConnectReject {
    pub code: u16,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseStreamEvent {
    Start { http_response: HttpResponseMetadata },
    Chunk { seq: u64, payload: Vec<u8> },
    End,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_response_boundary_is_discriminated_by_phase_result() {
        let responses = [
            WebSocketResponse::ConnectAccept(WebSocketConnectAccept {
                business_identity: None,
                connection_policy: None,
                context: WebSocketConnectContext::Null,
            }),
            WebSocketResponse::ConnectReject(WebSocketConnectReject {
                code: 1008,
                reason: "policy".to_string(),
            }),
            WebSocketResponse::Receive,
        ];

        assert!(matches!(
            &responses[0],
            WebSocketResponse::ConnectAccept(WebSocketConnectAccept {
                context: WebSocketConnectContext::Null,
                ..
            })
        ));
        assert!(matches!(
            &responses[1],
            WebSocketResponse::ConnectReject(WebSocketConnectReject { code: 1008, .. })
        ));
        assert!(matches!(&responses[2], WebSocketResponse::Receive));
    }

    #[test]
    fn websocket_response_boundary_preserves_nominal_zero_byte_context() {
        let response =
            BoundaryResponse::websocket(WebSocketResponse::ConnectAccept(WebSocketConnectAccept {
                business_identity: None,
                connection_policy: None,
                context: WebSocketConnectContext::Typed {
                    payload: Vec::new(),
                    codec: WebSocketContextCodec {
                        operation_abi_id: "operation-abi".to_string(),
                        context_type_identity: "context-type".to_string(),
                    },
                },
            }));

        assert!(matches!(
            response,
            BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::WebSocket(
                WebSocketResponse::ConnectAccept(WebSocketConnectAccept {
                    context: WebSocketConnectContext::Typed { payload, .. },
                    ..
                })
            ))) if payload.is_empty()
        ));
    }
}
