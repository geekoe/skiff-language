use crate::envelope::WebSocketContextCodec;

pub use skiff_runtime_capability_context::{
    HttpResponseMetadata, ResponseError, WebSocketConnectionPolicyControl,
};

#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryResponse {
    Event(ResponseEvent),
    StreamSent,
}

impl BoundaryResponse {
    pub fn end(
        payload: Vec<u8>,
        http_response: Option<HttpResponseMetadata>,
        websocket_connect: Option<WebSocketConnectResponse>,
    ) -> Self {
        Self::Event(ResponseEvent::End {
            payload,
            http_response,
            websocket_connect,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEvent {
    End {
        payload: Vec<u8>,
        http_response: Option<HttpResponseMetadata>,
        websocket_connect: Option<WebSocketConnectResponse>,
    },
    Error(ResponseError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseStreamEvent {
    Start { http_response: HttpResponseMetadata },
    Chunk { seq: u64, payload: Vec<u8> },
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebSocketConnectResponse {
    pub result: String,
    pub business_identity: Option<String>,
    pub connection_policy: Option<WebSocketConnectionPolicyControl>,
    pub context_codec: Option<WebSocketContextCodec>,
    pub context_payload_present: bool,
    pub code: Option<u16>,
    pub reason: Option<String>,
}
