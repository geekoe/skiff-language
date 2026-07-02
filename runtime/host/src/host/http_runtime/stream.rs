use std::{
    collections::VecDeque,
    sync::{atomic::AtomicBool, Arc},
};

use serde_json::{Map, Value};
use skiff_runtime_capability_context::CancellationSignals;

use super::{
    call_context::HttpCallContext,
    cancel::{check_cancel_signals, wait_for_cancel_signals},
    response::{build_response_headers, chunk_event, response_event},
    response_parts::HttpResponseHead,
    sse::SseDecoder,
    transport::{map_reqwest_error_for, send_request},
};
use crate::{
    capability_context::{HttpRuntimeOptions, TARGET_STD_HTTP_STREAM},
    error::{Result, RuntimeError},
};

#[allow(dead_code)]
pub async fn stream(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Option<&AtomicBool>,
    service_max_response_bytes: usize,
) -> Result<Vec<Value>> {
    collect_events(
        open_stream_inner(
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
pub async fn open_stream_with_cancel_flags(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Vec<Arc<AtomicBool>>,
    service_max_response_bytes: usize,
) -> Result<HttpEventStream<'static>> {
    open_stream_with_cancel_flags_and_options(
        input,
        frame_deadline_ms,
        cancelled,
        service_max_response_bytes,
        HttpRuntimeOptions::from_env(),
    )
    .await
}

pub(crate) async fn open_stream_with_cancel_flags_and_options(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Vec<Arc<AtomicBool>>,
    service_max_response_bytes: usize,
    options: HttpRuntimeOptions,
) -> Result<HttpEventStream<'static>> {
    open_stream_with_cancellation_and_options(
        input,
        frame_deadline_ms,
        CancellationSignals::from_flags(cancelled),
        service_max_response_bytes,
        options,
    )
    .await
}

pub(crate) async fn open_stream_with_cancellation_and_options(
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
        TARGET_STD_HTTP_STREAM,
    );
    open_event_stream(context, HttpStreamMode::Raw).await
}

pub(crate) async fn open_body_stream_with_cancel_flags_and_options(
    input: &Value,
    frame_deadline_ms: Option<u64>,
    cancelled: Vec<Arc<AtomicBool>>,
    service_max_response_bytes: usize,
    options: HttpRuntimeOptions,
) -> Result<HttpBodyStream<'static>> {
    let context = HttpCallContext::owned(
        input,
        frame_deadline_ms,
        service_max_response_bytes,
        CancellationSignals::from_flags(cancelled),
        options,
        TARGET_STD_HTTP_STREAM,
    );
    let response = send_request(&context).await?;
    let head = HttpResponseHead::from_response(&response);
    let target = context.target();
    let max_response_bytes = context.service_max_response_bytes();
    let cancellation = context.into_cancel_signals();
    Ok(HttpBodyStream {
        head,
        events: HttpEventStream {
            response: Some(response),
            target,
            mode: HttpStreamMode::Raw,
            success_status: true,
            pending_events: VecDeque::new(),
            sse_decoder: SseDecoder::default(),
            bytes_read: 0,
            max_response_bytes,
            cancellation,
            finished: false,
        },
    })
}

pub(super) async fn open_stream_inner<'a>(
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
        TARGET_STD_HTTP_STREAM,
    );
    open_event_stream(context, HttpStreamMode::Raw).await
}

pub(super) async fn open_event_stream<'a>(
    context: HttpCallContext<'_, 'a>,
    mode: HttpStreamMode,
) -> Result<HttpEventStream<'a>> {
    let response = send_request(&context).await?;
    let head = HttpResponseHead::from_response(&response);
    let target = context.target();
    let max_response_bytes = context.service_max_response_bytes();
    let success_status = head.is_success_status();
    let pending_events = VecDeque::from([response_event(&head)]);
    let cancellation = context.into_cancel_signals();
    Ok(HttpEventStream {
        response: Some(response),
        target,
        mode,
        success_status,
        pending_events,
        sse_decoder: SseDecoder::default(),
        bytes_read: 0,
        max_response_bytes,
        cancellation,
        finished: false,
    })
}

pub(super) async fn collect_events(mut stream: HttpEventStream<'_>) -> Result<Vec<Value>> {
    let mut events = Vec::new();
    while let Some(event) = stream.next_event().await? {
        events.push(event);
    }
    Ok(events)
}

#[derive(Debug, Clone, Copy)]
pub(super) enum HttpStreamMode {
    Raw,
    Sse,
}

pub struct HttpEventStream<'a> {
    response: Option<reqwest::Response>,
    target: &'static str,
    mode: HttpStreamMode,
    success_status: bool,
    pending_events: VecDeque<Value>,
    sse_decoder: SseDecoder,
    bytes_read: usize,
    max_response_bytes: usize,
    cancellation: CancellationSignals<'a>,
    finished: bool,
}

pub struct HttpBodyStream<'a> {
    head: HttpResponseHead,
    events: HttpEventStream<'a>,
}

impl<'a> HttpBodyStream<'a> {
    pub fn handle_metadata(&self) -> (u16, Value) {
        (
            self.head.status(),
            build_response_headers(self.head.headers()),
        )
    }

    pub fn handle_value(status: u16, headers: Value, body: Value) -> Value {
        let mut object = Map::new();
        object.insert("status".to_string(), Value::Number(status.into()));
        object.insert("headers".to_string(), headers);
        object.insert("body".to_string(), body);
        Value::Object(object)
    }

    pub async fn next_body_chunk(&mut self) -> Result<Option<Value>> {
        let Some(event) = self.events.next_event().await? else {
            return Ok(None);
        };
        let chunk = event.get("value").cloned().ok_or_else(|| {
            RuntimeError::Decode("HTTP body stream chunk missing value".to_string())
        })?;
        Ok(Some(chunk))
    }
}

impl<'a> HttpEventStream<'a> {
    pub async fn next_event(&mut self) -> Result<Option<Value>> {
        check_cancel_signals(&self.cancellation)?;
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }
        if self.finished {
            return Ok(None);
        }

        loop {
            let Some(chunk) = self.next_body_chunk().await? else {
                self.finished = true;
                if matches!(self.mode, HttpStreamMode::Sse) && self.success_status {
                    self.pending_events
                        .extend(self.sse_decoder.finish(self.target)?);
                    if let Some(event) = self.pending_events.pop_front() {
                        return Ok(Some(event));
                    }
                }
                return Ok(None);
            };

            self.bytes_read = self.bytes_read.saturating_add(chunk.len());
            if self.bytes_read > self.max_response_bytes {
                return Err(RuntimeError::Protocol {
                    target: self.target.to_string(),
                    message: format!(
                        "response body exceeds max size of {} bytes",
                        self.max_response_bytes
                    ),
                });
            }

            match self.mode {
                HttpStreamMode::Raw => return Ok(Some(chunk_event("chunk", &chunk))),
                HttpStreamMode::Sse if !self.success_status => {
                    return Ok(Some(chunk_event("body", &chunk)));
                }
                HttpStreamMode::Sse => {
                    self.pending_events
                        .extend(self.sse_decoder.feed(&chunk, self.target)?);
                    if let Some(event) = self.pending_events.pop_front() {
                        return Ok(Some(event));
                    }
                }
            }
        }
    }

    async fn next_body_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(response) = self.response.as_mut() else {
            return Ok(None);
        };
        let chunk = if !self.cancellation.is_empty() {
            let chunk_future = response.chunk();
            tokio::select! {
                chunk = chunk_future => chunk.map_err(|error| map_reqwest_error_for(self.target, error))?,
                _ = wait_for_cancel_signals(&self.cancellation) => return Err(RuntimeError::cancelled()),
            }
        } else {
            response
                .chunk()
                .await
                .map_err(|error| map_reqwest_error_for(self.target, error))?
        };
        Ok(chunk.map(|bytes| bytes.to_vec()))
    }
}
