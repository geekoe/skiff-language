use std::sync::Arc;

use skiff_runtime_boundary::http::{HttpBoundaryNameValue, HttpBoundaryResponseStreamEvent};

use crate::{
    HttpNameValue, HttpResponseMetadata, RequestError, RequestResult, ResponseEventSink,
    ResponseStreamEvent,
};

pub(crate) struct ResponseStreamWriter {
    request_id: String,
    response_events: Arc<dyn ResponseEventSink>,
    started: bool,
    ended: bool,
    next_seq: u64,
}

impl ResponseStreamWriter {
    pub(crate) fn new(request_id: String, response_events: Arc<dyn ResponseEventSink>) -> Self {
        Self {
            request_id,
            response_events,
            started: false,
            ended: false,
            next_seq: 0,
        }
    }

    pub(crate) fn send_binary_http_event(
        &mut self,
        event: HttpBoundaryResponseStreamEvent,
    ) -> RequestResult<()> {
        if self.ended {
            return Err(RequestError::Decode(
                "HttpResponseStreamEvent emitted after end".to_string(),
            ));
        }
        match event {
            HttpBoundaryResponseStreamEvent::Start { status, headers } => {
                if self.started {
                    return Err(RequestError::Decode(
                        "duplicate HttpResponseStreamEvent.start".to_string(),
                    ));
                }
                self.started = true;
                self.send_event(ResponseStreamEvent::Start {
                    http_response: HttpResponseMetadata::new(
                        status,
                        response_headers_from_boundary(headers),
                    ),
                })
            }
            HttpBoundaryResponseStreamEvent::Chunk(bytes) => {
                if !self.started {
                    return Err(RequestError::Decode(
                        "HttpResponseStreamEvent.chunk emitted before start".to_string(),
                    ));
                }
                self.send_chunk(bytes)
            }
            HttpBoundaryResponseStreamEvent::End => {
                if !self.started {
                    return Err(RequestError::Decode(
                        "HttpResponseStreamEvent.end emitted before start".to_string(),
                    ));
                }
                self.ended = true;
                self.send_event(ResponseStreamEvent::End)
            }
        }
    }

    pub(crate) fn start_runtime_stream(&mut self) -> RequestResult<()> {
        if self.started {
            return Err(RequestError::Decode(
                "duplicate response stream start".to_string(),
            ));
        }
        self.started = true;
        self.send_event(ResponseStreamEvent::Start {
            http_response: HttpResponseMetadata::new(200, Vec::new()),
        })
    }

    pub(crate) fn send_chunk(&mut self, payload: Vec<u8>) -> RequestResult<()> {
        if !self.started {
            return Err(RequestError::Decode(
                "response stream chunk emitted before start".to_string(),
            ));
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.send_event(ResponseStreamEvent::Chunk { seq, payload })
    }

    pub(crate) fn finish(&mut self) -> RequestResult<()> {
        if self.ended {
            return Ok(());
        }
        if !self.started {
            return Err(RequestError::Decode(
                "HTTP response stream ended before start".to_string(),
            ));
        }
        self.ended = true;
        self.send_event(ResponseStreamEvent::End)
    }

    fn send_event(&self, event: ResponseStreamEvent) -> RequestResult<()> {
        self.response_events
            .send_stream_event(&self.request_id, event)
    }
}

fn response_headers_from_boundary(headers: Vec<HttpBoundaryNameValue>) -> Vec<HttpNameValue> {
    headers
        .into_iter()
        .map(|header| HttpNameValue {
            name: header.name,
            value: header.value,
        })
        .collect()
}
