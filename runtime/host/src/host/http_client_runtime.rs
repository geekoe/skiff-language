//! HTTP effect execution and response boundary conversion.

use std::{future::Future, pin::Pin};

use serde_json::Value;
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, StreamPullSource, StreamRuntimeError,
    StreamRuntimeResult,
};

use crate::{
    capability_context::{
        HttpClientCapabilityContext, HttpEffectContext, HttpRuntimeOptions,
        TARGET_STD_HTTP_REQUEST, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM,
    },
    config_view::{from_wire_json_plan, materialize_internal_json, materialize_json},
    error::{Result, RuntimeError},
    host::http_runtime::{
        open_body_stream_with_cancellation_and_options, open_sse_with_cancellation_and_options,
        request_with_cancellation_and_options, HttpBodyStream, HttpEventStream,
    },
};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

pub(crate) struct HttpEffectRequest<'a> {
    target: &'a str,
    input: Value,
    deadline_ms: Option<u64>,
    response_max_bytes: usize,
    cancellation: CancellationToken,
    http_options: HttpRuntimeOptions,
}

impl<'a> HttpEffectRequest<'a> {
    fn new(
        target: &'a str,
        context: &HttpEffectContext,
        input: &Value,
        http_options: HttpRuntimeOptions,
    ) -> Result<Self> {
        Ok(Self {
            target,
            input: materialize_json(input.clone())?,
            deadline_ms: context.deadline_ms(),
            response_max_bytes: context.response_max_bytes(),
            cancellation: context.cancellation_token(),
            http_options,
        })
    }

    fn target(&self) -> &str {
        self.target
    }

    fn input(&self) -> &Value {
        &self.input
    }
}

impl HttpClientCapabilityContext {
    pub(crate) fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.test_effect_double_context()
            .dispatch_test_http_effect_invocation_double(target, input, arg_plan, return_plan, heap)
    }

    pub(crate) async fn dispatch_http_request(&self, input: &Value) -> Result<Value> {
        let request = HttpEffectRequest::new(
            TARGET_STD_HTTP_REQUEST,
            self.http(),
            input,
            self.http_options(),
        )?;
        let test_effect_doubles = self.test_effect_double_context();
        if let Some(value) =
            test_effect_doubles.dispatch_test_effect_double(request.target(), Some(request.input()))
        {
            return value;
        }
        test_effect_doubles.require_non_test_mode(request.target())?;
        request_with_cancellation_and_options(
            request.input(),
            request.deadline_ms,
            request.response_max_bytes,
            CancellationSignals::from_tokens([request.cancellation.clone()]),
            request.http_options.clone(),
        )
        .await
        .and_then(materialize_internal_json)
    }

    pub(crate) async fn dispatch_http_stream(
        &self,
        input: &Value,
        expected_body_item_type: Option<&RuntimeTypePlan>,
    ) -> Result<Value> {
        let expected_body_item_type = expected_body_item_type.cloned().ok_or_else(|| {
            RuntimeError::invalid_artifact(
                "std.http.stream boundary is missing expected body stream item type plan"
                    .to_string(),
            )
        })?;
        let request = HttpEffectRequest::new(
            TARGET_STD_HTTP_STREAM,
            self.http(),
            input,
            self.http_options(),
        )?;
        let test_effect_doubles = self.test_effect_double_context();
        if let Some(value) =
            test_effect_doubles.dispatch_test_effect_double(request.target(), Some(request.input()))
        {
            return value;
        }
        test_effect_doubles.require_non_test_mode(request.target())?;

        let stream_cancellation = CancellationToken::new();
        let http_stream = open_body_stream_with_cancellation_and_options(
            request.input(),
            request.deadline_ms,
            CancellationSignals::from_tokens([
                request.cancellation.clone(),
                stream_cancellation.clone(),
            ]),
            request.response_max_bytes,
            request.http_options.clone(),
        )
        .await?;
        let (status, headers) = http_stream.handle_metadata();
        let stream = self.stream_runtime().pull_stream_with_cancellation(
            HttpBodyPullSource::new(http_stream, expected_body_item_type),
            stream_cancellation,
        );
        Ok(HttpBodyStream::handle_value(status, headers, stream))
    }

    pub(crate) async fn dispatch_http_sse(
        &self,
        input: &Value,
        expected_item_type: Option<&RuntimeTypePlan>,
    ) -> Result<Value> {
        let expected_item_type = expected_item_type.cloned().ok_or_else(|| {
            RuntimeError::invalid_artifact(
                "std.http.sse boundary is missing expected stream item type plan".to_string(),
            )
        })?;
        let request =
            HttpEffectRequest::new(TARGET_STD_HTTP_SSE, self.http(), input, self.http_options())?;
        let test_effect_doubles = self.test_effect_double_context();
        if let Some(value) =
            test_effect_doubles.dispatch_test_effect_double(request.target(), Some(request.input()))
        {
            return value;
        }
        test_effect_doubles.require_non_test_mode(request.target())?;

        let stream_cancellation = CancellationToken::new();
        let http_stream = open_sse_with_cancellation_and_options(
            request.input(),
            request.deadline_ms,
            CancellationSignals::from_tokens([
                request.cancellation.clone(),
                stream_cancellation.clone(),
            ]),
            request.response_max_bytes,
            request.http_options.clone(),
        )
        .await?;
        let stream = self.stream_runtime().pull_stream_with_cancellation(
            HttpEventPullSource::new(http_stream, expected_item_type),
            stream_cancellation,
        );
        Ok(stream)
    }
}

struct HttpEventPullSource {
    stream: HttpEventStream<'static>,
    expected_item_type: RuntimeTypePlan,
}

struct HttpBodyPullSource {
    stream: HttpBodyStream<'static>,
    expected_item_type: RuntimeTypePlan,
}

impl HttpBodyPullSource {
    fn new(stream: HttpBodyStream<'static>, expected_item_type: RuntimeTypePlan) -> Self {
        Self {
            stream,
            expected_item_type,
        }
    }
}

impl StreamPullSource for HttpBodyPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        Box::pin(async move {
            let Some(chunk) = self
                .stream
                .next_body_chunk()
                .await
                .map_err(StreamRuntimeError::producer)?
            else {
                return Ok(None);
            };
            from_wire_json_plan(chunk, Some(&self.expected_item_type))
                .and_then(materialize_internal_json)
                .map_err(StreamRuntimeError::producer)
                .map(Some)
        })
    }
}

impl HttpEventPullSource {
    fn new(stream: HttpEventStream<'static>, expected_item_type: RuntimeTypePlan) -> Self {
        Self {
            stream,
            expected_item_type,
        }
    }
}

impl StreamPullSource for HttpEventPullSource {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
        Box::pin(async move {
            let Some(event) = self
                .stream
                .next_event()
                .await
                .map_err(StreamRuntimeError::producer)?
            else {
                return Ok(None);
            };
            from_wire_json_plan(event, Some(&self.expected_item_type))
                .and_then(materialize_internal_json)
                .map_err(StreamRuntimeError::producer)
                .map(Some)
        })
    }
}
