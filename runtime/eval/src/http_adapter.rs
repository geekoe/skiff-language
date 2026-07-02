use super::{program_invocation::ProgramInvocationContext, Interpreter};
use crate::{
    error::{Result, RuntimeError},
    invocation::{
        AdapterArgSource, BinaryHttpRequestPlan, EvalBoundaryProjection, EvalInvocation,
        HttpAdapterProjection, HttpAdapterProjectionKind, HttpAdapterResponseProjection,
    },
    stream_callback::{
        map_callback_error, map_eval_error, EvalStreamExecutionError, EvalStreamResult,
    },
};
use skiff_runtime_boundary::http::{
    decode_typed_json_body, direct_http_request_coerce_plan, direct_http_request_runtime_value,
    direct_http_response_coerce_plan, direct_http_response_from_runtime_value,
    encode_typed_json_response, typed_json_body_decode_plan, typed_json_response_encode_plan,
    HttpBoundaryNameValue, HttpBoundaryResponseParts, HttpBoundaryResponseStreamEvent,
};
use skiff_runtime_capability_context::{
    binary_http_request_parts, BinaryHttpRequestContext, RequestPayloadContext,
};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

#[allow(dead_code)]
struct HttpAdapterContext<'a> {
    request: RequestPayloadContext<'a>,
    adapter: HttpAdapterProjection<'a>,
    pre_context: Option<RuntimeValue>,
    http_response_max_bytes: usize,
}

impl<'a> HttpAdapterContext<'a> {
    fn from_invocation(
        context: &ProgramInvocationContext<'a>,
        adapter: HttpAdapterProjection<'a>,
    ) -> Self {
        Self {
            request: context.request().clone(),
            adapter,
            pre_context: None,
            http_response_max_bytes: context.http_response_max_bytes(),
        }
    }

    fn target(&self) -> &str {
        self.request.target()
    }

    fn protocol_error(&self, message: impl Into<String>) -> RuntimeError {
        RuntimeError::Protocol {
            target: self.target().to_string(),
            message: message.into(),
        }
    }

    fn binary_http(&self) -> Result<&BinaryHttpRequestContext<'a>> {
        Ok(self.request.require_binary_http()?)
    }
}

struct HttpAdapterPreparedCall<'a> {
    invocation: EvalInvocation<'a>,
    args: Vec<RuntimeValue>,
}

struct HttpAdapterPreparedGuard<'a> {
    call: HttpAdapterPreparedCall<'a>,
    response: HttpAdapterResponseProjection,
}

impl Interpreter {
    pub async fn execute_program_http_adapter<'a>(
        &'a self,
        invocation_context: &ProgramInvocationContext<'a>,
        eval_invocation: EvalInvocation<'a>,
    ) -> Result<HttpBoundaryResponseParts> {
        let adapter = http_adapter_projection(eval_invocation)?;
        let mut context = HttpAdapterContext::from_invocation(invocation_context, adapter);
        if context.adapter.kind == HttpAdapterProjectionKind::RawHttp {
            return self
                .execute_program_http_raw_adapter(invocation_context, context)
                .await;
        }

        let mut heap = invocation_context.request_heap();

        let (guard_call, next_heap) = self.prepare_http_adapter_guard(&context, heap)?;
        heap = next_heap;
        if let Some(guard_call) = guard_call {
            let (value, _expected_plan, next_heap) = self
                .execute_eval_invocation_runtime_args_with_heap(
                    invocation_context,
                    guard_call.call.invocation,
                    guard_call.call.args,
                    heap,
                )
                .await?;
            heap = next_heap;
            if let Some(response) =
                self.http_adapter_guard_response(&guard_call.response, &value, &mut heap)?
            {
                return Ok(response);
            }
        }

        let (pre_call, mut heap) = self.prepare_http_adapter_pre(&context, heap)?;
        if let Some(pre_call) = pre_call {
            let (value, _expected_plan, next_heap) = self
                .execute_eval_invocation_runtime_args_with_heap(
                    invocation_context,
                    pre_call.invocation,
                    pre_call.args,
                    heap,
                )
                .await?;
            heap = next_heap;
            context.pre_context = Some(value);
        }
        let args = http_adapter_handler_values(&mut context, &mut heap)?;
        let handler_invocation = context.adapter.handler.as_ref().clone();
        let (value, response_plan, mut heap) = self
            .execute_eval_invocation_runtime_args_with_heap(
                invocation_context,
                handler_invocation,
                args,
                heap,
            )
            .await?;
        let response_boundary_plan = typed_json_response_encode_plan(response_plan);
        let body =
            encode_typed_json_response(&value, &response_boundary_plan, &mut heap)?.into_bytes();
        Ok(HttpBoundaryResponseParts {
            status: 200,
            headers: vec![HttpBoundaryNameValue {
                name: "content-type".to_string(),
                value: "application/json; charset=utf-8".to_string(),
            }],
            body,
        })
    }

    async fn execute_program_http_raw_adapter<'a>(
        &'a self,
        invocation_context: &ProgramInvocationContext<'a>,
        mut context: HttpAdapterContext<'a>,
    ) -> Result<HttpBoundaryResponseParts> {
        let mut heap = invocation_context.request_heap();

        let (guard_call, next_heap) = self.prepare_http_adapter_guard(&context, heap)?;
        heap = next_heap;
        if let Some(guard_call) = guard_call {
            let (value, _expected_plan, next_heap) = self
                .execute_eval_invocation_runtime_args_with_heap(
                    invocation_context,
                    guard_call.call.invocation,
                    guard_call.call.args,
                    heap,
                )
                .await?;
            heap = next_heap;
            if let Some(response) =
                self.http_adapter_guard_response(&guard_call.response, &value, &mut heap)?
            {
                return Ok(response);
            }
        }

        let (pre_call, mut heap) = self.prepare_http_adapter_pre(&context, heap)?;
        if let Some(pre_call) = pre_call {
            let (value, _expected_plan, next_heap) = self
                .execute_eval_invocation_runtime_args_with_heap(
                    invocation_context,
                    pre_call.invocation,
                    pre_call.args,
                    heap,
                )
                .await?;
            heap = next_heap;
            context.pre_context = Some(value);
        }
        let args = http_raw_adapter_handler_values(&mut context, &mut heap)?;
        let handler_invocation = context.adapter.handler.as_ref().clone();
        let (value, _response_plan, mut heap) = self
            .execute_eval_invocation_runtime_args_with_heap(
                invocation_context,
                handler_invocation,
                args,
                heap,
            )
            .await?;
        let response_plan = context
            .adapter
            .raw_handler_response
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "HTTP response boundary is missing return type".to_string(),
                )
            })
            .and_then(http_response_plan_from_projection)?;
        binary_http_response_from_plan(&value, response_plan, &mut heap)
    }

    pub async fn execute_program_http_raw_adapter_response_stream<'a, F, E>(
        &'a self,
        invocation_context: &ProgramInvocationContext<'a>,
        eval_invocation: EvalInvocation<'a>,
        mut on_event: F,
    ) -> EvalStreamResult<(), E>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
    {
        let adapter = map_eval_error(http_adapter_projection(eval_invocation))?;
        let mut context = HttpAdapterContext::from_invocation(invocation_context, adapter);
        if context.adapter.kind != HttpAdapterProjectionKind::RawHttp {
            return Err(EvalStreamExecutionError::Eval(context.protocol_error(
                "HTTP serverStream adapter dispatch requires rawHttp adapter",
            )));
        }

        let mut heap = invocation_context.request_heap();

        let (guard_call, next_heap) =
            map_eval_error(self.prepare_http_adapter_guard(&context, heap))?;
        heap = next_heap;
        if let Some(guard_call) = guard_call {
            let (value, _expected_plan, next_heap) = map_eval_error(
                self.execute_eval_invocation_runtime_args_with_heap(
                    invocation_context,
                    guard_call.call.invocation,
                    guard_call.call.args,
                    heap,
                )
                .await,
            )?;
            heap = next_heap;
            if let Some(response) = map_eval_error(self.http_adapter_guard_response(
                &guard_call.response,
                &value,
                &mut heap,
            ))? {
                emit_binary_http_response_as_stream(response, &mut on_event)?;
                return Ok(());
            }
        }

        let (pre_call, mut heap) = map_eval_error(self.prepare_http_adapter_pre(&context, heap))?;
        if let Some(pre_call) = pre_call {
            let (value, _expected_plan, next_heap) = map_eval_error(
                self.execute_eval_invocation_runtime_args_with_heap(
                    invocation_context,
                    pre_call.invocation,
                    pre_call.args,
                    heap,
                )
                .await,
            )?;
            heap = next_heap;
            context.pre_context = Some(value);
        }
        let args = map_eval_error(http_raw_adapter_handler_values(&mut context, &mut heap))?;
        let handler_invocation = context.adapter.handler.as_ref().clone();
        self.execute_eval_invocation_runtime_args_http_response_stream_with_heap(
            invocation_context,
            handler_invocation,
            args,
            heap,
            &mut on_event,
        )
        .await
    }

    fn prepare_http_adapter_guard<'a>(
        &self,
        context: &HttpAdapterContext<'a>,
        mut heap: RequestHeap,
    ) -> Result<(Option<HttpAdapterPreparedGuard<'a>>, RequestHeap)> {
        let Some(guard) = context.adapter.guard.as_ref().cloned() else {
            return Ok((None, heap));
        };

        let request = binary_http_request_arg_from_plan(context, &guard.request, &mut heap)?;
        Ok((
            Some(HttpAdapterPreparedGuard {
                call: HttpAdapterPreparedCall {
                    invocation: *guard.invocation,
                    args: vec![request],
                },
                response: guard.response,
            }),
            heap,
        ))
    }

    fn http_adapter_guard_response(
        &self,
        response: &HttpAdapterResponseProjection,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<Option<HttpBoundaryResponseParts>> {
        if matches!(value, RuntimeValue::Null) {
            return Ok(None);
        }

        let response_plan = http_response_plan_from_projection(response)?;
        binary_http_response_from_plan(value, response_plan, heap).map(Some)
    }

    fn prepare_http_adapter_pre<'a>(
        &self,
        context: &HttpAdapterContext<'a>,
        mut heap: RequestHeap,
    ) -> Result<(Option<HttpAdapterPreparedCall<'a>>, RequestHeap)> {
        let Some(pre) = context.adapter.pre.as_ref().cloned() else {
            return Ok((None, heap));
        };

        let request = binary_http_request_arg_from_plan(context, &pre.request, &mut heap)?;
        Ok((
            Some(HttpAdapterPreparedCall {
                invocation: *pre.invocation,
                args: vec![request],
            }),
            heap,
        ))
    }
}

fn http_adapter_projection<'a>(
    eval_invocation: EvalInvocation<'a>,
) -> Result<HttpAdapterProjection<'a>> {
    let target = eval_invocation.request().target().to_string();
    match eval_invocation.boundary_projection().clone() {
        EvalBoundaryProjection::HttpAdapter { adapter } => Ok(adapter),
        _ => Err(RuntimeError::Protocol {
            target,
            message: "HTTP adapter dispatch requires HTTP adapter projection".to_string(),
        }),
    }
}

fn binary_http_request_arg_from_plan(
    context: &HttpAdapterContext<'_>,
    request: &BinaryHttpRequestPlan,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    binary_http_request_arg_with_plan(
        context,
        request.parameter_name.as_str(),
        &request.parameter_plan,
        heap,
    )
}

fn binary_http_request_arg_with_plan(
    context: &HttpAdapterContext<'_>,
    parameter_name: &str,
    parameter_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let boundary_plan = direct_http_request_coerce_plan(parameter_plan.clone());
    Ok(direct_http_request_runtime_value(
        &binary_http_request_parts(context.binary_http()?),
        &boundary_plan,
        format!("binary HTTP request parameter {parameter_name}"),
        heap,
    )?)
}

fn http_adapter_handler_values(
    context: &mut HttpAdapterContext<'_>,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>> {
    let arg_plan = context.adapter.handler_args.clone();
    let mut args = Vec::with_capacity(arg_plan.len());
    let mut pre_context = context.pre_context.take();
    for arg in &arg_plan {
        match arg.source {
            AdapterArgSource::HttpRequest => {
                return Err(
                    context.protocol_error("HTTP adapter does not support request handler arg")
                );
            }
            AdapterArgSource::HttpBody => {
                let body = std::str::from_utf8(context.binary_http()?.body()).map_err(|error| {
                    RuntimeError::Decode(format!("HTTP adapter body is not UTF-8: {error}"))
                })?;
                let boundary_plan = typed_json_body_decode_plan(arg.parameter_plan.clone());
                args.push(decode_typed_json_body(body, &boundary_plan, heap)?);
            }
            AdapterArgSource::HttpContext => {
                let value = pre_context.take().ok_or_else(|| {
                    context.protocol_error("HTTP adapter context arg requires http.pre")
                })?;
                args.push(value);
            }
            AdapterArgSource::WebSocketConnectRequest
            | AdapterArgSource::WebSocketReceiveEvent
            | AdapterArgSource::WebSocketConnection
            | AdapterArgSource::WebSocketConnectionContext
            | AdapterArgSource::WebSocketMessage
            | AdapterArgSource::WebSocketMessageBody
            | AdapterArgSource::WebSocketConnectionId
            | AdapterArgSource::WebSocketBusinessIdentity => {
                return Err(context
                    .protocol_error("WebSocket adapter source is not valid for HTTP adapter"));
            }
        }
    }
    Ok(args)
}

fn http_raw_adapter_handler_values(
    context: &mut HttpAdapterContext<'_>,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>> {
    let arg_plan = context.adapter.handler_args.clone();
    let mut args = Vec::with_capacity(arg_plan.len());
    let mut pre_context = context.pre_context.take();
    for arg in &arg_plan {
        match arg.source {
            AdapterArgSource::HttpRequest => {
                args.push(binary_http_request_arg_with_plan(
                    context,
                    arg.parameter_name.as_str(),
                    &arg.parameter_plan,
                    heap,
                )?);
            }
            AdapterArgSource::HttpContext => {
                let value = pre_context.take().ok_or_else(|| {
                    context.protocol_error("HTTP raw adapter context arg requires http.pre")
                })?;
                args.push(value);
            }
            AdapterArgSource::HttpBody => {
                return Err(
                    context.protocol_error("HTTP raw adapter does not support body handler arg")
                );
            }
            AdapterArgSource::WebSocketConnectRequest
            | AdapterArgSource::WebSocketReceiveEvent
            | AdapterArgSource::WebSocketConnection
            | AdapterArgSource::WebSocketConnectionContext
            | AdapterArgSource::WebSocketMessage
            | AdapterArgSource::WebSocketMessageBody
            | AdapterArgSource::WebSocketConnectionId
            | AdapterArgSource::WebSocketBusinessIdentity => {
                return Err(context
                    .protocol_error("WebSocket adapter source is not valid for HTTP raw adapter"));
            }
        }
    }
    Ok(args)
}

fn http_response_plan_from_projection(
    response: &HttpAdapterResponseProjection,
) -> Result<&RuntimeTypePlan> {
    match response {
        HttpAdapterResponseProjection::Plan(plan) => Ok(plan),
        HttpAdapterResponseProjection::MissingReturnType => Err(RuntimeError::InvalidArtifact(
            "HTTP response boundary is missing return type".to_string(),
        )),
        HttpAdapterResponseProjection::InvalidHttpResponseType => Err(RuntimeError::Protocol {
            target: "response.end".to_string(),
            message: "binary HTTP handler must return std.http.HttpResponse".to_string(),
        }),
        HttpAdapterResponseProjection::InvalidArtifact(message) => {
            Err(RuntimeError::InvalidArtifact(message.clone()))
        }
    }
}

fn binary_http_response_from_plan(
    value: &RuntimeValue,
    response_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<HttpBoundaryResponseParts> {
    let boundary_plan = direct_http_response_coerce_plan(response_plan.clone());
    let parts = direct_http_response_from_runtime_value(
        value,
        &boundary_plan,
        "binary HTTP response",
        heap,
    )?;
    Ok(parts)
}

fn emit_binary_http_response_as_stream<F, E>(
    response: HttpBoundaryResponseParts,
    on_event: &mut F,
) -> EvalStreamResult<(), E>
where
    F: FnMut(HttpBoundaryResponseStreamEvent) -> std::result::Result<(), E>,
{
    let callback_result = on_event(HttpBoundaryResponseStreamEvent::Start {
        status: response.status,
        headers: response.headers,
    });
    map_callback_error(callback_result)?;
    if !response.body.is_empty() {
        let callback_result = on_event(HttpBoundaryResponseStreamEvent::Chunk(response.body));
        map_callback_error(callback_result)?;
    }
    let callback_result = on_event(HttpBoundaryResponseStreamEvent::End);
    map_callback_error(callback_result)
}
