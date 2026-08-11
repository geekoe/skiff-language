//! HTTP effect execution and response boundary conversion.

use std::{future::Future, pin::Pin, time::Duration};

use serde_json::Value;
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, ExecutionScope, ExecutionScopeLeaseTerminal,
    OwnedExecutionControl, StreamPullSource, StreamRuntimeError, StreamRuntimeResult,
};

use crate::{
    capability_context::{
        HttpClientCapabilityContext, HttpEffectContext, HttpRuntimeOptions,
        TARGET_STD_HTTP_REQUEST, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM,
    },
    config_view::{from_wire_json_plan, materialize_internal_json, materialize_json},
    error::{OrdinaryRuntimeError, Result, RuntimeError},
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
        self.dispatch_http_request_inner(input, None).await
    }

    #[allow(dead_code)]
    pub(crate) async fn dispatch_http_request_with_current_scope(
        &self,
        input: &Value,
        execution_control: OwnedExecutionControl,
    ) -> Result<Value> {
        let current_scope = current_http_scope(&execution_control, TARGET_STD_HTTP_REQUEST)?;
        self.dispatch_http_request_inner(input, Some(current_scope))
            .await
    }

    async fn dispatch_http_request_inner(
        &self,
        input: &Value,
        current_scope: Option<ExecutionScope>,
    ) -> Result<Value> {
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
        let output = match current_scope {
            Some(current_scope) => {
                let primitive_timeout_ms = http_primitive_timeout_ms(request.input());
                await_http_request_lower_with_current_scope(
                    current_scope,
                    primitive_timeout_ms,
                    || async {
                        request_with_cancellation_and_options(
                            request.input(),
                            None,
                            request.response_max_bytes,
                            CancellationSignals::none(),
                            request.http_options.clone(),
                        )
                        .await
                    },
                )
                .await
            }
            None => {
                request_with_cancellation_and_options(
                    request.input(),
                    request.deadline_ms,
                    request.response_max_bytes,
                    CancellationSignals::from_tokens([request.cancellation.clone()]),
                    request.http_options.clone(),
                )
                .await
            }
        };
        output.and_then(materialize_internal_json)
    }

    pub(crate) async fn dispatch_http_stream(
        &self,
        input: &Value,
        expected_body_item_type: Option<&RuntimeTypePlan>,
    ) -> Result<Value> {
        self.dispatch_http_stream_inner(input, expected_body_item_type, None)
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn dispatch_http_stream_with_current_scope(
        &self,
        input: &Value,
        expected_body_item_type: Option<&RuntimeTypePlan>,
        execution_control: OwnedExecutionControl,
    ) -> Result<Value> {
        let current_scope = current_http_scope(&execution_control, TARGET_STD_HTTP_STREAM)?;
        self.dispatch_http_stream_inner(input, expected_body_item_type, Some(current_scope))
            .await
    }

    async fn dispatch_http_stream_inner(
        &self,
        input: &Value,
        expected_body_item_type: Option<&RuntimeTypePlan>,
        current_scope: Option<ExecutionScope>,
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
        let http_stream = match current_scope {
            Some(current_scope) => {
                let primitive_timeout_ms = http_primitive_timeout_ms(request.input());
                await_http_body_open_lower_with_current_scope(
                    current_scope,
                    primitive_timeout_ms,
                    || async {
                        open_body_stream_with_cancellation_and_options(
                            request.input(),
                            None,
                            CancellationSignals::from_tokens([stream_cancellation.clone()]),
                            request.response_max_bytes,
                            request.http_options.clone(),
                        )
                        .await
                    },
                )
                .await?
            }
            None => {
                open_body_stream_with_cancellation_and_options(
                    request.input(),
                    request.deadline_ms,
                    CancellationSignals::from_tokens([
                        request.cancellation.clone(),
                        stream_cancellation.clone(),
                    ]),
                    request.response_max_bytes,
                    request.http_options.clone(),
                )
                .await?
            }
        };
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
        self.dispatch_http_sse_inner(input, expected_item_type, None)
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn dispatch_http_sse_with_current_scope(
        &self,
        input: &Value,
        expected_item_type: Option<&RuntimeTypePlan>,
        execution_control: OwnedExecutionControl,
    ) -> Result<Value> {
        let current_scope = current_http_scope(&execution_control, TARGET_STD_HTTP_SSE)?;
        self.dispatch_http_sse_inner(input, expected_item_type, Some(current_scope))
            .await
    }

    async fn dispatch_http_sse_inner(
        &self,
        input: &Value,
        expected_item_type: Option<&RuntimeTypePlan>,
        current_scope: Option<ExecutionScope>,
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
        let http_stream = match current_scope {
            Some(current_scope) => {
                let primitive_timeout_ms = http_primitive_timeout_ms(request.input());
                await_http_sse_open_lower_with_current_scope(
                    current_scope,
                    primitive_timeout_ms,
                    || async {
                        open_sse_with_cancellation_and_options(
                            request.input(),
                            None,
                            CancellationSignals::from_tokens([stream_cancellation.clone()]),
                            request.response_max_bytes,
                            request.http_options.clone(),
                        )
                        .await
                    },
                )
                .await?
            }
            None => {
                open_sse_with_cancellation_and_options(
                    request.input(),
                    request.deadline_ms,
                    CancellationSignals::from_tokens([
                        request.cancellation.clone(),
                        stream_cancellation.clone(),
                    ]),
                    request.response_max_bytes,
                    request.http_options.clone(),
                )
                .await?
            }
        };
        let stream = self.stream_runtime().pull_stream_with_cancellation(
            HttpEventPullSource::new(http_stream, expected_item_type),
            stream_cancellation,
        );
        Ok(stream)
    }
}

#[allow(dead_code)]
fn current_http_scope(
    execution_control: &OwnedExecutionControl,
    target: &str,
) -> Result<ExecutionScope> {
    execution_control.execution_scope().map_err(|error| {
        RuntimeError::invalid_artifact(format!(
            "current execution scope is unavailable for {target}: {error}"
        ))
    })
}

fn http_primitive_timeout_ms(input: &Value) -> Option<u64> {
    input.get("timeoutMs").and_then(Value::as_u64)
}

pub(super) async fn await_http_lower_with_current_scope<T, F, Fut>(
    current_scope: ExecutionScope,
    primitive_timeout_ms: Option<u64>,
    lower: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let (lease, _completion) = current_scope.acquire_lease();
    let lower = lower();
    let primitive_timeout = async move {
        match primitive_timeout_ms {
            Some(timeout_ms) => tokio::time::sleep(Duration::from_millis(timeout_ms)).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(lower);
    tokio::pin!(primitive_timeout);

    tokio::select! {
        biased;
        output = &mut lower => output,
        terminal = lease.wait() => match terminal {
            ExecutionScopeLeaseTerminal::Control(_) => Err(RuntimeError::cancelled()),
            ExecutionScopeLeaseTerminal::Completed => {
                unreachable!("HTTP lower completion is committed by the lower branch")
            }
        },
        _ = &mut primitive_timeout => Err(http_primitive_timeout_error()),
    }
}

pub(super) async fn await_http_request_lower_with_current_scope<T, F, Fut>(
    current_scope: ExecutionScope,
    primitive_timeout_ms: Option<u64>,
    lower: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    await_http_lower_with_current_scope(current_scope, primitive_timeout_ms, lower).await
}

pub(super) async fn await_http_body_open_lower_with_current_scope<T, F, Fut>(
    current_scope: ExecutionScope,
    primitive_timeout_ms: Option<u64>,
    lower: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    await_http_lower_with_current_scope(current_scope, primitive_timeout_ms, lower).await
}

pub(super) async fn await_http_sse_open_lower_with_current_scope<T, F, Fut>(
    current_scope: ExecutionScope,
    primitive_timeout_ms: Option<u64>,
    lower: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    await_http_lower_with_current_scope(current_scope, primitive_timeout_ms, lower).await
}

fn http_primitive_timeout_error() -> RuntimeError {
    RuntimeError::ExternalErrorPayload {
        code: "TimeoutError".to_string(),
        message: "HTTP request timeout exceeded".to_string(),
        status: None,
        details: Some(serde_json::json!({
            "reason": "httpRequestTimeout",
        })),
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
                .map_err(runtime_error_into_stream)?
            else {
                return Ok(None);
            };
            from_wire_json_plan(chunk, Some(&self.expected_item_type))
                .and_then(materialize_internal_json)
                .map_err(runtime_error_into_stream)
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
                .map_err(runtime_error_into_stream)?
            else {
                return Ok(None);
            };
            from_wire_json_plan(event, Some(&self.expected_item_type))
                .and_then(materialize_internal_json)
                .map_err(runtime_error_into_stream)
                .map(Some)
        })
    }
}

fn runtime_error_into_stream(error: RuntimeError) -> StreamRuntimeError {
    if error.is_cancellation_terminal() {
        StreamRuntimeError::Cancelled
    } else {
        StreamRuntimeError::producer(
            OrdinaryRuntimeError::try_new(error)
                .expect("HTTP stream cancellation was split before producer trait erasure"),
        )
    }
}
