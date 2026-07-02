use std::sync::{atomic::AtomicBool, Arc};

use serde_json::{Map, Value};
use skiff_runtime_capability_context::CancellationSignals;

use super::{
    call_context::HttpCallContext,
    stream::{collect_events, open_event_stream, HttpEventStream, HttpStreamMode},
};
use crate::{
    capability_context::{HttpRuntimeOptions, TARGET_STD_HTTP_SSE},
    error::{Result, RuntimeError},
};

#[allow(dead_code)]
pub async fn sse(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Option<&AtomicBool>,
    service_max_response_bytes: usize,
) -> Result<Vec<Value>> {
    collect_events(
        open_sse_inner(
            input,
            frame_deadline_ms,
            cancelled,
            service_max_response_bytes,
            HttpRuntimeOptions::from_env(),
        )
        .await?,
    )
    .await
}

#[allow(dead_code)]
pub async fn open_sse_with_cancel_flags(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Vec<Arc<AtomicBool>>,
    service_max_response_bytes: usize,
) -> Result<HttpEventStream<'static>> {
    open_sse_with_cancel_flags_and_options(
        input,
        frame_deadline_ms,
        cancelled,
        service_max_response_bytes,
        HttpRuntimeOptions::from_env(),
    )
    .await
}

pub(crate) async fn open_sse_with_cancel_flags_and_options(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Vec<Arc<AtomicBool>>,
    service_max_response_bytes: usize,
    options: HttpRuntimeOptions,
) -> Result<HttpEventStream<'static>> {
    open_sse_with_cancellation_and_options(
        input,
        frame_deadline_ms,
        CancellationSignals::from_flags(cancelled),
        service_max_response_bytes,
        options,
    )
    .await
}

pub(crate) async fn open_sse_with_cancellation_and_options(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancellation: CancellationSignals<'static>,
    service_max_response_bytes: usize,
    options: HttpRuntimeOptions,
) -> Result<HttpEventStream<'static>> {
    let context = HttpCallContext::owned(
        input,
        frame_deadline_ms,
        service_max_response_bytes,
        cancellation,
        options,
        TARGET_STD_HTTP_SSE,
    );
    open_event_stream(context, HttpStreamMode::Sse).await
}

pub(super) async fn open_sse_inner<'a>(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Option<&'a AtomicBool>,
    service_max_response_bytes: usize,
    options: HttpRuntimeOptions,
) -> Result<HttpEventStream<'a>> {
    let context = HttpCallContext::borrowed(
        input,
        frame_deadline_ms,
        service_max_response_bytes,
        cancelled,
        options,
        TARGET_STD_HTTP_SSE,
    );
    open_event_stream(context, HttpStreamMode::Sse).await
}

#[derive(Debug, Default)]
pub(super) struct SseDecoder {
    pending_utf8: Vec<u8>,
    line_buffer: String,
    current: SseEventBuilder,
}

impl SseDecoder {
    pub(super) fn feed(&mut self, chunk: &[u8], target: &str) -> Result<Vec<Value>> {
        let mut bytes = Vec::with_capacity(self.pending_utf8.len() + chunk.len());
        bytes.extend_from_slice(&self.pending_utf8);
        bytes.extend_from_slice(chunk);
        self.pending_utf8.clear();

        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text.to_string(),
            Err(error) if error.error_len().is_none() => {
                let valid_up_to = error.valid_up_to();
                self.pending_utf8.extend_from_slice(&bytes[valid_up_to..]);
                String::from_utf8(bytes[..valid_up_to].to_vec()).map_err(|error| {
                    RuntimeError::Protocol {
                        target: target.to_string(),
                        message: format!("SSE UTF-8 decode failed: {error}"),
                    }
                })?
            }
            Err(error) => {
                return Err(RuntimeError::Protocol {
                    target: target.to_string(),
                    message: format!("SSE UTF-8 decode failed: {error}"),
                });
            }
        };

        Ok(self.process_text(&text))
    }

    pub(super) fn finish(&mut self, target: &str) -> Result<Vec<Value>> {
        if !self.pending_utf8.is_empty() {
            return Err(RuntimeError::Protocol {
                target: target.to_string(),
                message: "SSE response ended with incomplete UTF-8 sequence".to_string(),
            });
        }

        let mut events = Vec::new();
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            if let Some(event) = self.process_line(&line) {
                events.push(event);
            }
        }
        if let Some(event) = self.current.dispatch() {
            events.push(event);
        }
        Ok(events)
    }

    fn process_text(&mut self, text: &str) -> Vec<Value> {
        let mut events = Vec::new();
        self.line_buffer.push_str(text);
        while let Some(newline_index) = self.line_buffer.find('\n') {
            let mut line = self.line_buffer.drain(..=newline_index).collect::<String>();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            if let Some(event) = self.process_line(&line) {
                events.push(event);
            }
        }
        events
    }

    fn process_line(&mut self, line: &str) -> Option<Value> {
        if line.is_empty() {
            return self.current.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }

        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
            (field, value.strip_prefix(' ').unwrap_or(value))
        });
        match field {
            "data" => self.current.data.push(value.to_string()),
            "event" => self.current.event = Some(value.to_string()),
            "id" => self.current.id = Some(value.to_string()),
            _ => {}
        }
        None
    }
}

#[derive(Debug, Default)]
struct SseEventBuilder {
    event: Option<String>,
    id: Option<String>,
    data: Vec<String>,
}

impl SseEventBuilder {
    fn dispatch(&mut self) -> Option<Value> {
        if self.data.is_empty() {
            self.event = None;
            self.id = None;
            return None;
        }

        let mut object = Map::new();
        object.insert("tag".to_string(), Value::String("event".to_string()));
        object.insert(
            "event".to_string(),
            self.event.take().map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "id".to_string(),
            self.id.take().map(Value::String).unwrap_or(Value::Null),
        );
        object.insert("data".to_string(), Value::String(self.data.join("\n")));
        self.data.clear();
        Some(Value::Object(object))
    }
}
