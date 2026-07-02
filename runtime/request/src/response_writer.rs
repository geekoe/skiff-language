use super::{RequestResult, ResponseStreamEvent};

pub trait ResponseEventSink: Send + Sync {
    fn send_stream_event(&self, request_id: &str, event: ResponseStreamEvent) -> RequestResult<()>;
}
