use std::sync::Mutex;

use super::*;

#[test]
fn runtime_http_gateway_stream_writer_accepts_one_exact_terminal_sequence() {
    let sink = Arc::new(RecordingSink::default());
    let mut writer = ResponseStreamWriter::new("request-1".to_string(), sink.clone());
    writer
        .send_binary_http_event(HttpBoundaryResponseStreamEvent::Start {
            status: 202,
            headers: vec![HttpBoundaryNameValue {
                name: "content-type".to_string(),
                value: "application/octet-stream".to_string(),
            }],
        })
        .unwrap();
    writer
        .send_binary_http_event(HttpBoundaryResponseStreamEvent::Chunk(vec![1, 2, 3]))
        .unwrap();
    writer
        .send_binary_http_event(HttpBoundaryResponseStreamEvent::End)
        .unwrap();
    writer.require_exact_http_terminal().unwrap();
    assert_eq!(sink.events.lock().unwrap().len(), 3);
    assert!(writer
        .send_binary_http_event(HttpBoundaryResponseStreamEvent::End)
        .unwrap_err()
        .to_string()
        .contains("after end"));
}

#[test]
fn runtime_http_gateway_stream_writer_rejects_missing_or_out_of_order_terminal() {
    let sink = Arc::new(RecordingSink::default());
    let mut writer = ResponseStreamWriter::new("request-2".to_string(), sink);
    assert!(writer
        .send_binary_http_event(HttpBoundaryResponseStreamEvent::Chunk(vec![1]))
        .unwrap_err()
        .to_string()
        .contains("before start"));
    writer
        .send_binary_http_event(HttpBoundaryResponseStreamEvent::Start {
            status: 200,
            headers: Vec::new(),
        })
        .unwrap();
    assert!(writer
        .require_exact_http_terminal()
        .unwrap_err()
        .to_string()
        .contains("exactly start/chunk*/end"));
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, ResponseStreamEvent)>>,
}

impl ResponseEventSink for RecordingSink {
    fn send_stream_event(&self, request_id: &str, event: ResponseStreamEvent) -> RequestResult<()> {
        self.events
            .lock()
            .unwrap()
            .push((request_id.to_string(), event));
        Ok(())
    }
}
