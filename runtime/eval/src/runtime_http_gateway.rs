use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface, GatewayProtocolSurface,
    PackageCallableId, PackageCallableSignature,
};
use skiff_runtime_boundary::http::{
    decode_typed_json_body, encode_typed_json_response, typed_json_body_decode_plan,
    typed_json_response_encode_plan, HttpBoundaryNameValue, HttpBoundaryResponseParts,
    HttpBoundaryResponseStreamEvent,
};
use skiff_runtime_capability_context::RequestPayloadContext;
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, LinkedExecutable};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

use crate::{
    binary_http_boundary::{
        binary_http_request_parameter_value, binary_http_response_from_runtime_value,
        linked_http_response_stream_item_type,
    },
    error::{Result, RuntimeError},
    program_execution::ProgramExecutionContext,
    runtime_ops::runtime_to_wire_required_plan,
    stream_callback::EvalStreamExecutionError,
    Interpreter, RuntimeAssemblyEvalTarget,
};

/// Exact callable facts already closed by the linked gateway entry.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeHttpGatewayCallable<'a> {
    pub callable_id: &'a PackageCallableId,
    pub signature: &'a PackageCallableSignature,
    pub addr: &'a ExecutableAddr,
}

/// Eval-facing view whose production implementation is the request-owned exact gateway target.
///
/// The view contains no source/display lookup and no service-boundary identity.
pub trait RuntimeHttpGatewayExecutionTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget;
    fn gateway_entry_key(&self) -> &GatewayEntryKey;
    fn gateway_entry_identity(&self) -> &GatewayEntryIdentity;
    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface;
    fn adapter_plan(&self) -> &GatewayAdapterPlan;
    fn handler(&self) -> RuntimeHttpGatewayCallable<'_>;
    fn pre(&self) -> Option<RuntimeHttpGatewayCallable<'_>>;
    fn guard(&self) -> Option<RuntimeHttpGatewayCallable<'_>>;
}

impl Interpreter {
    /// Executes an exact linked HTTP gateway unary plan through the canonical assembly image.
    pub async fn execute_runtime_http_gateway_unary(
        &self,
        context: ProgramExecutionContext<'_>,
        request: RequestPayloadContext<'_>,
        target: &impl RuntimeHttpGatewayExecutionTarget,
    ) -> Result<HttpBoundaryResponseParts> {
        validate_execution_pin(&context, target)?;
        validate_request_target(&request, target)?;
        let http = require_http_surface(target)?;
        if http.dispatch_mode != GatewayDispatchMode::Unary {
            return Err(protocol_error(
                target,
                "unary dispatch does not match the linked gateway mode",
            ));
        }
        request.require_binary_http()?;
        let mut heap = context.request_heap();
        if let Some(response) = self
            .execute_gateway_guard(&context, &request, target, &mut heap)
            .await?
        {
            return Ok(response);
        }
        let pre_context = self
            .execute_gateway_pre(&context, &request, target, &mut heap)
            .await?;
        let handler = target.handler();
        let args = handler_args(&request, target, handler, pre_context.as_ref(), &mut heap)?;
        let value = self
            .execute_runtime_assembly_addr(context, &mut heap, handler.addr, args)
            .await?;
        match http.adapter_kind {
            GatewayAdapterKind::TypedJson => {
                let response_plan = callable_return_plan(target, handler)?;
                let body = encode_typed_json_response(
                    &value,
                    &typed_json_response_encode_plan(response_plan),
                    &mut heap,
                )?
                .into_bytes();
                Ok(HttpBoundaryResponseParts {
                    status: 200,
                    headers: vec![HttpBoundaryNameValue {
                        name: "content-type".to_string(),
                        value: "application/json; charset=utf-8".to_string(),
                    }],
                    body,
                })
            }
            GatewayAdapterKind::RawHttp => binary_http_response_from_runtime_value(
                &value,
                callable_executable(target, handler)?.return_type.as_ref(),
                target.eval_target().execution_projection().type_view(),
                handler.addr,
                &mut heap,
            ),
            GatewayAdapterKind::WebSocketConnect => Err(protocol_error(
                target,
                "HTTP gateway execution refuses websocketConnect adapter plans",
            )),
        }
    }

    /// Executes an exact raw HTTP server stream through the existing stream runtime/cleanup path.
    pub async fn execute_runtime_http_gateway_server_stream<F>(
        &self,
        context: ProgramExecutionContext<'_>,
        request: RequestPayloadContext<'_>,
        target: &impl RuntimeHttpGatewayExecutionTarget,
        mut on_event: F,
    ) -> Result<()>
    where
        F: FnMut(HttpBoundaryResponseStreamEvent) -> Result<()>,
    {
        validate_execution_pin(&context, target)?;
        validate_request_target(&request, target)?;
        let http = require_http_surface(target)?;
        if (http.adapter_kind, http.dispatch_mode)
            != (
                GatewayAdapterKind::RawHttp,
                GatewayDispatchMode::ServerStream,
            )
        {
            return Err(protocol_error(
                target,
                "serverStream dispatch requires the exact rawHttp server-stream plan",
            ));
        }
        request.require_binary_http()?;
        let mut heap = context.request_heap();
        if let Some(response) = self
            .execute_gateway_guard(&context, &request, target, &mut heap)
            .await?
        {
            emit_unary_response_as_stream(response, &mut on_event)?;
            return Ok(());
        }
        let pre_context = self
            .execute_gateway_pre(&context, &request, target, &mut heap)
            .await?;
        let handler = target.handler();
        let args = handler_args(&request, target, handler, pre_context.as_ref(), &mut heap)?;
        let executable = callable_executable(target, handler)?;
        let return_type = executable.return_type.as_ref().ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "HTTP gateway stream handler is missing its linked return type".to_string(),
            )
        })?;
        let item_type = linked_http_response_stream_item_type(
            Some(return_type),
            target.eval_target().execution_projection().type_view(),
            handler.addr,
        )?
        .ok_or_else(|| {
            protocol_error(
                target,
                "rawHttp serverStream handler must return Stream<std.http.HttpResponseStreamEvent>",
            )
        })?;
        let plan_context = PlanContext::from_type_view(
            target.eval_target().execution_projection().type_view(),
            handler.addr,
        );
        let response_plan = RuntimeTypePlan::from_linked(return_type, &plan_context)?;
        let item_plan = RuntimeTypePlan::from_linked_nested_ref(item_type, &plan_context)?;
        let value = self
            .execute_runtime_assembly_addr_with_stream_defer(
                context.clone(),
                &mut heap,
                handler.addr,
                args,
            )
            .await?;
        let stream_value = runtime_to_wire_required_plan(
            &value,
            Some(&response_plan),
            "HTTP gateway response stream",
            &mut heap,
        )?;
        let consumer = self.consume_in_process_binary_http_response_stream(
            &context,
            &stream_value,
            &item_plan,
            &[],
            &mut on_event,
        );
        self.drive_deferred_stream_producer(
            context.clone(),
            handler.addr,
            &stream_value,
            async move {
                consumer.await.map_err(|error| match error {
                    EvalStreamExecutionError::Eval(error)
                    | EvalStreamExecutionError::Callback(error) => error,
                })
            },
        )
        .await
    }

    async fn execute_gateway_guard(
        &self,
        context: &ProgramExecutionContext<'_>,
        request: &RequestPayloadContext<'_>,
        target: &impl RuntimeHttpGatewayExecutionTarget,
        heap: &mut RequestHeap,
    ) -> Result<Option<HttpBoundaryResponseParts>> {
        let Some(guard) = target.guard() else {
            return Ok(None);
        };
        let executable = callable_executable(target, guard)?;
        let parameter = single_parameter(target, guard, executable)?;
        let value = binary_http_request_parameter_value(
            target.gateway_entry_key().as_str(),
            executable.symbol.as_str(),
            parameter.name.as_str(),
            Some(&parameter.ty),
            target.eval_target().execution_projection().type_view(),
            guard.addr,
            request.require_binary_http()?,
            heap,
        )?;
        let result = self
            .execute_runtime_assembly_addr(context.clone(), heap, guard.addr, vec![value])
            .await?;
        if result == RuntimeValue::Null {
            return Ok(None);
        }
        let response_type = match executable.return_type.as_ref() {
            Some(skiff_runtime_linked_program::LinkedTypeRef::Nullable { inner }) => {
                Some(inner.as_ref())
            }
            return_type => return_type,
        };
        binary_http_response_from_runtime_value(
            &result,
            response_type,
            target.eval_target().execution_projection().type_view(),
            guard.addr,
            heap,
        )
        .map(Some)
    }

    async fn execute_gateway_pre(
        &self,
        context: &ProgramExecutionContext<'_>,
        request: &RequestPayloadContext<'_>,
        target: &impl RuntimeHttpGatewayExecutionTarget,
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        let Some(pre) = target.pre() else {
            return Ok(None);
        };
        let executable = callable_executable(target, pre)?;
        let parameter = single_parameter(target, pre, executable)?;
        let value = binary_http_request_parameter_value(
            target.gateway_entry_key().as_str(),
            executable.symbol.as_str(),
            parameter.name.as_str(),
            Some(&parameter.ty),
            target.eval_target().execution_projection().type_view(),
            pre.addr,
            request.require_binary_http()?,
            heap,
        )?;
        self.execute_runtime_assembly_addr(context.clone(), heap, pre.addr, vec![value])
            .await
            .map(Some)
    }
}

fn validate_execution_pin(
    context: &ProgramExecutionContext<'_>,
    target: &impl RuntimeHttpGatewayExecutionTarget,
) -> Result<()> {
    let pinned = context.runtime_assembly_target()?;
    let expected = target.eval_target();
    if !std::sync::Arc::ptr_eq(pinned.execution_image(), expected.execution_image())
        || !std::sync::Arc::ptr_eq(pinned.activation_context(), expected.activation_context())
        || pinned.request_activation().generation() != expected.request_activation().generation()
    {
        return Err(protocol_error(
            target,
            "gateway execution context does not match the exact eval target",
        ));
    }
    expected.ensure_execution_ready()?;
    Ok(())
}

fn require_http_surface(
    target: &impl RuntimeHttpGatewayExecutionTarget,
) -> Result<&skiff_artifact_model::GatewayHttpProtocolSurface> {
    let GatewayProtocolSurface::Http(http) = &target.protocol_surface().protocol else {
        return Err(protocol_error(
            target,
            "HTTP gateway execution requires an HTTP protocol surface",
        ));
    };
    if http.adapter_kind != target.adapter_plan().kind {
        return Err(protocol_error(
            target,
            "gateway adapter kind does not match its protocol surface",
        ));
    }
    Ok(http)
}

fn validate_request_target(
    request: &RequestPayloadContext<'_>,
    target: &impl RuntimeHttpGatewayExecutionTarget,
) -> Result<()> {
    if request.target() != target.gateway_entry_key().as_str() {
        return Err(protocol_error(
            target,
            "request payload target does not match the exact gateway entry key",
        ));
    }
    Ok(())
}

fn callable_executable<'a>(
    target: &'a impl RuntimeHttpGatewayExecutionTarget,
    callable: RuntimeHttpGatewayCallable<'a>,
) -> Result<&'a LinkedExecutable> {
    let resolved = target
        .eval_target()
        .execution_projection()
        .resolve_executable(callable.addr)?;
    if resolved.addr != *callable.addr
        || resolved.executable.kind != ExecutableKind::Function
        || resolved.executable.self_type.is_some()
        || resolved.executable.return_type.is_none()
        || resolved.executable.type_params != callable.signature.type_params
        || resolved.executable.may_suspend != callable.signature.may_suspend
        || resolved.executable.params.len() != callable.signature.parameters.len()
        || resolved
            .executable
            .params
            .iter()
            .zip(&callable.signature.parameters)
            .any(|(linked, declared)| linked.name != declared.name)
    {
        return Err(protocol_error(
            target,
            format!(
                "gateway callable {} does not match its exact linked signature",
                callable.callable_id
            ),
        ));
    }
    Ok(resolved.executable)
}

fn single_parameter<'a>(
    target: &impl RuntimeHttpGatewayExecutionTarget,
    callable: RuntimeHttpGatewayCallable<'_>,
    executable: &'a LinkedExecutable,
) -> Result<&'a skiff_runtime_linked_program::ParamIr> {
    let [parameter] = executable.params.as_slice() else {
        return Err(protocol_error(
            target,
            format!(
                "gateway callable {} must have exactly one request parameter",
                callable.callable_id
            ),
        ));
    };
    Ok(parameter)
}

fn callable_return_plan(
    target: &impl RuntimeHttpGatewayExecutionTarget,
    callable: RuntimeHttpGatewayCallable<'_>,
) -> Result<RuntimeTypePlan> {
    let executable = callable_executable(target, callable)?;
    let return_type = executable.return_type.as_ref().ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "gateway callable {} is missing its linked return type",
            callable.callable_id
        ))
    })?;
    RuntimeTypePlan::from_linked(
        return_type,
        &PlanContext::from_type_view(
            target.eval_target().execution_projection().type_view(),
            callable.addr,
        ),
    )
    .map_err(Into::into)
}

fn handler_args(
    request: &RequestPayloadContext<'_>,
    target: &impl RuntimeHttpGatewayExecutionTarget,
    handler: RuntimeHttpGatewayCallable<'_>,
    pre_context: Option<&RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>> {
    let executable = callable_executable(target, handler)?;
    let http = require_http_surface(target)?;
    if executable.params.len() != target.adapter_plan().args.len() {
        return Err(protocol_error(
            target,
            "gateway adapter plan does not cover the exact handler signature",
        ));
    }
    let binary_http = request.require_binary_http()?;
    let mut values = Vec::with_capacity(executable.params.len());
    for parameter in &executable.params {
        let arg = target
            .adapter_plan()
            .args
            .iter()
            .find(|arg| arg.param == parameter.name)
            .ok_or_else(|| {
                protocol_error(
                    target,
                    format!(
                        "gateway adapter plan is missing parameter {}",
                        parameter.name
                    ),
                )
            })?;
        let value = match arg.source {
            GatewayAdapterSource::HttpRequest => binary_http_request_parameter_value(
                target.gateway_entry_key().as_str(),
                executable.symbol.as_str(),
                parameter.name.as_str(),
                Some(&parameter.ty),
                target.eval_target().execution_projection().type_view(),
                handler.addr,
                binary_http,
                heap,
            )?,
            GatewayAdapterSource::HttpBody => {
                if http.adapter_kind != GatewayAdapterKind::TypedJson {
                    return Err(protocol_error(
                        target,
                        "http.body is only valid for a typedJson gateway",
                    ));
                }
                let body = std::str::from_utf8(binary_http.body()).map_err(|error| {
                    RuntimeError::Decode(format!(
                        "HTTP gateway typedJson body is not UTF-8: {error}"
                    ))
                })?;
                let plan = RuntimeTypePlan::from_linked_nested_ref(
                    &parameter.ty,
                    &PlanContext::from_type_view(
                        target.eval_target().execution_projection().type_view(),
                        handler.addr,
                    ),
                )?;
                decode_typed_json_body(body, &typed_json_body_decode_plan(plan), heap)?
            }
            GatewayAdapterSource::HttpContext => pre_context.cloned().ok_or_else(|| {
                protocol_error(
                    target,
                    "http.context requires the exact linked pre callable result",
                )
            })?,
            GatewayAdapterSource::WebSocketConnectRequest
            | GatewayAdapterSource::WebSocketConnectionId => {
                return Err(protocol_error(
                    target,
                    "HTTP gateway execution refuses WebSocket adapter sources",
                ))
            }
        };
        values.push(value);
    }
    Ok(values)
}

fn emit_unary_response_as_stream<F>(
    response: HttpBoundaryResponseParts,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(HttpBoundaryResponseStreamEvent) -> Result<()>,
{
    on_event(HttpBoundaryResponseStreamEvent::Start {
        status: response.status,
        headers: response.headers,
    })?;
    if !response.body.is_empty() {
        on_event(HttpBoundaryResponseStreamEvent::Chunk(response.body))?;
    }
    on_event(HttpBoundaryResponseStreamEvent::End)
}

fn protocol_error(
    target: &impl RuntimeHttpGatewayExecutionTarget,
    message: impl Into<String>,
) -> RuntimeError {
    RuntimeError::Protocol {
        target: target.gateway_entry_key().as_str().to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
