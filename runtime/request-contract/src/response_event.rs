pub use skiff_runtime_capability_context::{
    FixedServiceResponseFailure, HttpResponseMetadata, ResponseError,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseStreamEvent {
    Start { http_response: HttpResponseMetadata },
    Chunk { seq: u64, payload: Vec<u8> },
    End,
}
